use crate::model::{Envelope, RoomSnapshot};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard, TryLockError},
};

struct PendingEvent {
    room_id: String,
    run_id: String,
    sequence: u64,
    received_at_ms: u64,
    envelope_json: String,
}

impl PendingEvent {
    fn estimated_bytes(&self) -> usize {
        self.room_id
            .len()
            .saturating_add(self.run_id.len())
            .saturating_add(self.envelope_json.len())
            .saturating_add(std::mem::size_of::<Self>())
    }
}

#[derive(Default)]
struct PendingBacklog {
    events: Vec<PendingEvent>,
    bytes: usize,
}

// Roughly 34 seconds of 60 Hz telemetry from a full 16-player room. A failed
// disk must not turn the durable retry path into an unbounded memory leak.
// The byte ceiling is equally important: a peer can make two envelopes with
// the same count consume radically different amounts of memory.
const MAX_PENDING_EVENTS: usize = 32_768;
const MAX_PENDING_EVENT_BYTES: usize = 16 * 1024 * 1024;

pub struct Storage {
    connection: Mutex<Connection>,
    pending_events: Mutex<PendingBacklog>,
}

impl Storage {
    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>> {
        match self.connection.try_lock() {
            Ok(connection) => Ok(connection),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {
                bail!("another storage operation is still in progress")
            }
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path).context("open runtime SQLite database")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS room_recovery (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                snapshot_json TEXT NOT NULL,
                recover_until_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS match_history (
                room_id TEXT PRIMARY KEY,
                room_name TEXT NOT NULL,
                lifecycle TEXT NOT NULL,
                snapshot_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS run_events (
                room_id TEXT NOT NULL,
                run_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                received_at_ms INTEGER NOT NULL,
                envelope_json TEXT NOT NULL,
                PRIMARY KEY (room_id, run_id, sequence)
            );
            CREATE INDEX IF NOT EXISTS run_events_received_idx ON run_events(received_at_ms);
            ",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            pending_events: Mutex::new(PendingBacklog::default()),
        })
    }

    pub fn save_room(&self, room: &RoomSnapshot, recovery_ms: u64) -> Result<()> {
        let json = serde_json::to_string(room)?;
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO room_recovery(singleton,snapshot_json,recover_until_ms,updated_at_ms)
             VALUES(1,?1,?2,?3)
             ON CONFLICT(singleton) DO UPDATE SET snapshot_json=excluded.snapshot_json,
             recover_until_ms=excluded.recover_until_ms,updated_at_ms=excluded.updated_at_ms",
            params![
                &json,
                room.updated_at_ms.saturating_add(recovery_ms),
                room.updated_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO match_history(room_id,room_name,lifecycle,snapshot_json,updated_at_ms)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(room_id) DO UPDATE SET lifecycle=excluded.lifecycle,
             snapshot_json=excluded.snapshot_json,updated_at_ms=excluded.updated_at_ms",
            params![
                room.id,
                room.name,
                format!("{:?}", room.lifecycle).to_lowercase(),
                &json,
                room.updated_at_ms
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn recover_room(&self, now_ms: u64) -> Result<Option<RoomSnapshot>> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            "SELECT snapshot_json,recover_until_ms FROM room_recovery WHERE singleton=1",
        )?;
        let value = statement.query_row([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        });
        match value {
            Ok((json, recover_until)) if recover_until >= now_ms => {
                Ok(Some(serde_json::from_str(&json)?))
            }
            Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn append_event(&self, room_id: &str, envelope: &Envelope) -> Result<bool> {
        self.queue_event(room_id, envelope)?;
        Ok(self.flush_pending_events()? > 0)
    }

    /// Serializes an ordered event and queues it for the short storage batch.
    /// Network handling never waits for an SQLite commit per score mutation.
    pub fn queue_event(&self, room_id: &str, envelope: &Envelope) -> Result<()> {
        let run_id = envelope.run_id.as_deref().unwrap_or("unassigned");
        let envelope_json = serde_json::to_string(envelope)?;
        let mut pending = self
            .pending_events
            .lock()
            .expect("pending storage events poisoned");
        let event = PendingEvent {
            room_id: room_id.to_owned(),
            run_id: run_id.to_owned(),
            sequence: envelope.sequence,
            received_at_ms: crate::room::unix_ms(),
            envelope_json,
        };
        let event_bytes = event.estimated_bytes();
        if pending.events.len() >= MAX_PENDING_EVENTS
            || pending.bytes.saturating_add(event_bytes) > MAX_PENDING_EVENT_BYTES
        {
            bail!("durable event backlog reached its safety limit");
        }
        pending.bytes += event_bytes;
        pending.events.push(event);
        Ok(())
    }

    /// Commits every queued event in one WAL transaction. At the normal 25 ms
    /// cadence this replaces dozens of durable autocommits with one commit while
    /// preserving the primary-key ordering and duplicate protection.
    pub fn flush_pending_events(&self) -> Result<usize> {
        let mut pending = std::mem::take(
            &mut *self
                .pending_events
                .lock()
                .expect("pending storage events poisoned"),
        );
        if pending.events.is_empty() {
            return Ok(0);
        }
        let result = (|| -> Result<usize> {
            let mut connection = self.lock_connection()?;
            let transaction = connection.transaction()?;
            let mut inserted = 0;
            {
                let mut statement = transaction.prepare(
                    "INSERT OR IGNORE INTO run_events(room_id,run_id,sequence,received_at_ms,envelope_json)
                     VALUES(?1,?2,?3,?4,?5)",
                )?;
                for event in &pending.events {
                    inserted += statement.execute(params![
                        event.room_id,
                        event.run_id,
                        event.sequence,
                        event.received_at_ms,
                        event.envelope_json
                    ])?;
                }
            }
            transaction.commit()?;
            Ok(inserted)
        })();
        if result.is_err() {
            let mut retry = self
                .pending_events
                .lock()
                .expect("pending storage events poisoned");
            // Older failed events must remain ahead of anything queued while the
            // transaction was running so a retry preserves arrival order.
            pending.events.append(&mut retry.events);
            pending.bytes = pending.bytes.saturating_add(retry.bytes);
            let mut dropped = 0usize;
            while pending.events.len() > MAX_PENDING_EVENTS
                || pending.bytes > MAX_PENDING_EVENT_BYTES
            {
                let Some(event) = pending.events.pop() else {
                    break;
                };
                pending.bytes = pending.bytes.saturating_sub(event.estimated_bytes());
                dropped += 1;
            }
            if dropped > 0 {
                tracing::error!(dropped, "durable event backlog discarded newest events");
            }
            *retry = pending;
        }
        result
    }

    pub fn has_pending_events(&self) -> bool {
        !self
            .pending_events
            .lock()
            .expect("pending storage events poisoned")
            .events
            .is_empty()
    }

    pub fn events(&self, room_id: &str, run_id: &str, from: u64) -> Result<Vec<Envelope>> {
        self.flush_pending_events()?;
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            "SELECT envelope_json FROM run_events
             WHERE room_id=?1 AND run_id=?2 AND sequence>=?3 ORDER BY sequence",
        )?;
        let rows = statement.query_map(params![room_id, run_id, from], |row| {
            row.get::<_, String>(0)
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(serde_json::from_str(&row?)?);
        }
        Ok(events)
    }

    pub fn prune_raw_events(&self, older_than_ms: u64) -> Result<usize> {
        self.flush_pending_events()?;
        let connection = self.lock_connection()?;
        Ok(connection.execute(
            "DELETE FROM run_events WHERE received_at_ms < ?1",
            params![older_than_ms],
        )?)
    }

    pub fn history(&self) -> Result<Vec<RoomSnapshot>> {
        let connection = self.lock_connection()?;
        let mut statement = connection
            .prepare("SELECT snapshot_json FROM match_history ORDER BY updated_at_ms DESC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut history = Vec::new();
        for row in rows {
            history.push(serde_json::from_str(&row?)?);
        }
        Ok(history)
    }

    pub fn delete_history(&self, room_id: &str) -> Result<bool> {
        self.flush_pending_events()?;
        let connection = self.lock_connection()?;
        connection.execute("DELETE FROM run_events WHERE room_id=?1", params![room_id])?;
        Ok(connection.execute(
            "DELETE FROM match_history WHERE room_id=?1",
            params![room_id],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::AdmissionMode, room::RoomEngine};

    #[test]
    fn room_recovers_only_during_grace_window() {
        let path = std::env::temp_dir().join(format!("bbt-storage-{}.db", rand::random::<u64>()));
        let storage = Storage::open(&path).unwrap();
        let room = RoomEngine::host(
            "Recovery".into(),
            "Host".into(),
            AdmissionMode::PasswordOnly,
        );
        storage.save_room(&room.snapshot, 120_000).unwrap();
        assert!(storage
            .recover_room(room.snapshot.updated_at_ms + 1)
            .unwrap()
            .is_some());
        assert!(storage
            .recover_room(room.snapshot.updated_at_ms + 120_001)
            .unwrap()
            .is_none());
        drop(storage);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn queued_events_commit_in_one_ordered_batch() {
        let path =
            std::env::temp_dir().join(format!("bbt-storage-batch-{}.db", rand::random::<u64>()));
        let storage = Storage::open(&path).unwrap();
        for sequence in 0..250 {
            let mut event = Envelope::new(
                "run.score_delta",
                sequence,
                serde_json::json!({"runSequence":sequence}),
            );
            event.run_id = Some("run-batch".into());
            storage.queue_event("room-batch", &event).unwrap();
        }
        assert_eq!(storage.flush_pending_events().unwrap(), 250);
        let recovered = storage.events("room-batch", "run-batch", 0).unwrap();
        assert_eq!(recovered.len(), 250);
        assert_eq!(recovered.first().unwrap().sequence, 0);
        assert_eq!(recovered.last().unwrap().sequence, 249);
        drop(storage);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pending_event_backlog_is_bounded() {
        let storage = Storage::open(":memory:").unwrap();
        let event = Envelope::new("run.score_delta", 0, serde_json::json!({}));
        for _ in 0..MAX_PENDING_EVENTS {
            storage.queue_event("bounded-room", &event).unwrap();
        }
        assert!(storage.queue_event("bounded-room", &event).is_err());
        assert_eq!(
            storage.pending_events.lock().unwrap().events.len(),
            MAX_PENDING_EVENTS
        );
    }

    #[test]
    fn pending_event_backlog_is_bounded_by_serialized_bytes() {
        let storage = Storage::open(":memory:").unwrap();
        let event = Envelope::new(
            "run.score_delta",
            0,
            serde_json::json!({"padding":"x".repeat(512 * 1024)}),
        );
        let mut accepted = 0usize;
        while storage.queue_event("bounded-room", &event).is_ok() {
            accepted += 1;
        }
        let pending = storage.pending_events.lock().unwrap();
        assert!(accepted < MAX_PENDING_EVENTS);
        assert!(pending.bytes <= MAX_PENDING_EVENT_BYTES);
        assert_eq!(pending.events.len(), accepted);
    }

    #[test]
    fn busy_database_requeues_once_without_waiting_for_the_lock() {
        let storage = Storage::open(":memory:").unwrap();
        let mut event = Envelope::new("run.score_delta", 1, serde_json::json!({}));
        event.run_id = Some("busy-run".into());
        storage.queue_event("busy-room", &event).unwrap();
        let connection = storage.connection.lock().unwrap();
        let started = std::time::Instant::now();
        assert!(storage.flush_pending_events().is_err());
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        assert_eq!(storage.pending_events.lock().unwrap().events.len(), 1);
        drop(connection);
        assert_eq!(storage.flush_pending_events().unwrap(), 1);
    }
}
