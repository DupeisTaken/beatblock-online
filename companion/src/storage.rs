use crate::model::{Envelope, RoomSnapshot};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::{path::Path, sync::Mutex};

pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
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
        })
    }

    pub fn save_room(&self, room: &RoomSnapshot, recovery_ms: u64) -> Result<()> {
        let json = serde_json::to_string(room)?;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO room_recovery(singleton,snapshot_json,recover_until_ms,updated_at_ms)
             VALUES(1,?1,?2,?3)
             ON CONFLICT(singleton) DO UPDATE SET snapshot_json=excluded.snapshot_json,
             recover_until_ms=excluded.recover_until_ms,updated_at_ms=excluded.updated_at_ms",
            params![
                json,
                room.updated_at_ms.saturating_add(recovery_ms),
                room.updated_at_ms
            ],
        )?;
        connection.execute(
            "INSERT INTO match_history(room_id,room_name,lifecycle,snapshot_json,updated_at_ms)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(room_id) DO UPDATE SET lifecycle=excluded.lifecycle,
             snapshot_json=excluded.snapshot_json,updated_at_ms=excluded.updated_at_ms",
            params![
                room.id,
                room.name,
                format!("{:?}", room.lifecycle).to_lowercase(),
                serde_json::to_string(room)?,
                room.updated_at_ms
            ],
        )?;
        Ok(())
    }

    pub fn recover_room(&self, now_ms: u64) -> Result<Option<RoomSnapshot>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
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
        let run_id = envelope.run_id.as_deref().unwrap_or("unassigned");
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO run_events(room_id,run_id,sequence,received_at_ms,envelope_json)
             VALUES(?1,?2,?3,?4,?5)",
            params![
                room_id,
                run_id,
                envelope.sequence,
                crate::room::unix_ms(),
                serde_json::to_string(envelope)?
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn events(&self, room_id: &str, run_id: &str, from: u64) -> Result<Vec<Envelope>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
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
        let connection = self.connection.lock().expect("storage mutex poisoned");
        Ok(connection.execute(
            "DELETE FROM run_events WHERE received_at_ms < ?1",
            params![older_than_ms],
        )?)
    }

    pub fn history(&self) -> Result<Vec<RoomSnapshot>> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
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
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute("DELETE FROM run_events WHERE room_id=?1", params![room_id])?;
        Ok(connection.execute("DELETE FROM match_history WHERE room_id=?1", params![room_id])? == 1)
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
}
