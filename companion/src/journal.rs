use crate::model::Envelope;
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

enum JournalSignal {
    Append {
        run_id: String,
        line: String,
    },
    Flush(mpsc::SyncSender<()>),
    Prune {
        older_than: SystemTime,
        done: mpsc::SyncSender<usize>,
    },
    #[cfg(test)]
    OpenWriterCount(mpsc::SyncSender<usize>),
}

const MAX_OPEN_JOURNALS: usize = 32;
const JOURNAL_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const JOURNAL_QUEUE_CAPACITY: usize = 8_192;

struct ActiveWriter {
    writer: BufWriter<File>,
    last_used: Instant,
}

/// Keeps active run journals open and flushes them in short batches. Ordered
/// events still reach the worker immediately, but the game/runtime hot path no
/// longer opens and flushes a file for every score mutation.
#[derive(Clone)]
pub struct JournalPublisher {
    signals: mpsc::SyncSender<JournalSignal>,
}

impl JournalPublisher {
    pub fn new(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let (signals, receiver) = mpsc::sync_channel(JOURNAL_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("bbt-journal".into())
            .spawn(move || run_worker(&directory, receiver))
            .context("spawn run journal worker")?;
        let publisher = Self { signals };
        // Enforce the documented retention policy at runtime startup as well
        // as when the operator invokes History > Prune.
        publisher.prune_days(30);
        Ok(publisher)
    }

    pub fn publish(&self, envelope: &Envelope) -> Result<()> {
        let run_id = envelope
            .run_id
            .as_deref()
            .or_else(|| {
                envelope
                    .payload
                    .get("runId")
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("unassigned");
        self.signals
            .try_send(JournalSignal::Append {
                run_id: safe_run_id(run_id),
                line: serde_json::to_string(envelope)?,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    anyhow::anyhow!("run journal queue is full; disk writer is not keeping up")
                }
                mpsc::TrySendError::Disconnected(_) => {
                    anyhow::anyhow!("run journal worker stopped")
                }
            })
    }

    pub fn flush(&self) {
        let (done, complete) = mpsc::sync_channel(0);
        if self.signals.try_send(JournalSignal::Flush(done)).is_ok() {
            let _ = complete.recv_timeout(Duration::from_secs(5));
        }
    }

    pub fn prune_days(&self, days: u64) -> usize {
        let age = Duration::from_secs(days.saturating_mul(86_400));
        let older_than = SystemTime::now()
            .checked_sub(age)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let (done, complete) = mpsc::sync_channel(0);
        if self
            .signals
            .try_send(JournalSignal::Prune { older_than, done })
            .is_ok()
        {
            return complete.recv_timeout(Duration::from_secs(5)).unwrap_or(0);
        }
        0
    }

    #[cfg(test)]
    fn open_writer_count(&self) -> usize {
        let (done, complete) = mpsc::sync_channel(0);
        if self
            .signals
            .try_send(JournalSignal::OpenWriterCount(done))
            .is_ok()
        {
            return complete.recv_timeout(Duration::from_secs(5)).unwrap_or(0);
        }
        0
    }
}

fn run_worker(directory: &Path, receiver: mpsc::Receiver<JournalSignal>) {
    let mut writers: HashMap<String, ActiveWriter> = HashMap::new();
    let mut dirty = false;
    let flush_interval = Duration::from_millis(50);
    let mut next_flush = Instant::now() + flush_interval;
    loop {
        let wait = next_flush.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(wait) {
            Ok(JournalSignal::Append { run_id, line }) => {
                let result = (|| -> Result<()> {
                    if !writers.contains_key(&run_id) && writers.len() >= MAX_OPEN_JOURNALS {
                        close_oldest_writer(&mut writers);
                    }
                    let active = match writers.entry(run_id.clone()) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            let file = OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(directory.join(format!("{run_id}.ndjson")))?;
                            entry.insert(ActiveWriter {
                                writer: BufWriter::with_capacity(64 * 1024, file),
                                last_used: Instant::now(),
                            })
                        }
                    };
                    active.writer.write_all(line.as_bytes())?;
                    active.writer.write_all(b"\n")?;
                    active.last_used = Instant::now();
                    dirty = true;
                    Ok(())
                })();
                if let Err(error) = result {
                    tracing::warn!(%error, "run journal append failed");
                }
            }
            Ok(JournalSignal::Flush(done)) => {
                flush_writers(&mut writers);
                dirty = false;
                next_flush = Instant::now() + flush_interval;
                let _ = done.send(());
            }
            Ok(JournalSignal::Prune { older_than, done }) => {
                flush_writers(&mut writers);
                writers.clear();
                dirty = false;
                next_flush = Instant::now() + flush_interval;
                let _ = done.send(prune_files(directory, older_than));
            }
            #[cfg(test)]
            Ok(JournalSignal::OpenWriterCount(done)) => {
                let _ = done.send(writers.len());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                flush_writers(&mut writers);
                break;
            }
        }
        if Instant::now() >= next_flush {
            if dirty {
                flush_writers(&mut writers);
                dirty = false;
            }
            close_idle_writers(&mut writers, Instant::now());
            next_flush = Instant::now() + flush_interval;
        }
    }
}

fn flush_writers(writers: &mut HashMap<String, ActiveWriter>) {
    for active in writers.values_mut() {
        if let Err(error) = active.writer.flush() {
            tracing::warn!(%error, "run journal flush failed");
        }
    }
}

fn close_oldest_writer(writers: &mut HashMap<String, ActiveWriter>) {
    let oldest = writers
        .iter()
        .min_by_key(|(_, active)| active.last_used)
        .map(|(run_id, _)| run_id.clone());
    if let Some(run_id) = oldest {
        if let Some(mut active) = writers.remove(&run_id) {
            let _ = active.writer.flush();
        }
    }
}

fn close_idle_writers(writers: &mut HashMap<String, ActiveWriter>, now: Instant) {
    let idle = writers
        .iter()
        .filter(|(_, active)| {
            now.saturating_duration_since(active.last_used) >= JOURNAL_IDLE_TIMEOUT
        })
        .map(|(run_id, _)| run_id.clone())
        .collect::<Vec<_>>();
    for run_id in idle {
        if let Some(mut active) = writers.remove(&run_id) {
            let _ = active.writer.flush();
        }
    }
}

fn prune_files(directory: &Path, older_than: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "ndjson")
        })
        .filter(|entry| {
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .is_some_and(|modified| modified < older_than)
        })
        .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
        .count()
}

fn safe_run_id(run_id: &str) -> String {
    let safe: String = run_id
        .chars()
        .filter(|value| value.is_ascii_alphanumeric() || *value == '-' || *value == '_')
        .take(96)
        .collect();
    if safe.is_empty() {
        "unassigned".into()
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_ordered_events_and_flushes_complete_lines() {
        let root = std::env::temp_dir().join(format!("bbt-journal-{}", rand::random::<u64>()));
        let journal = JournalPublisher::new(root.clone()).unwrap();
        for sequence in 0..100 {
            let mut event = Envelope::new("run.score_delta", sequence, serde_json::json!({}));
            event.run_id = Some("run-one".into());
            journal.publish(&event).unwrap();
        }
        journal.flush();
        let lines = std::fs::read_to_string(root.join("run-one.ndjson")).unwrap();
        assert_eq!(lines.lines().count(), 100);
        assert!(lines
            .lines()
            .all(|line| serde_json::from_str::<Envelope>(line).is_ok()));
        drop(journal);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn periodic_flush_is_bounded_even_without_an_explicit_flush() {
        let root =
            std::env::temp_dir().join(format!("bbt-journal-periodic-{}", rand::random::<u64>()));
        let journal = JournalPublisher::new(root.clone()).unwrap();
        let mut event = Envelope::new("run.score_delta", 1, serde_json::json!({}));
        event.run_id = Some("periodic".into());
        journal.publish(&event).unwrap();

        // Keep this an observation of the periodic flush rather than forcing a
        // flush through the public API. Parallel test workers can delay this
        // background thread beyond one timer interval on a busy CI runner, so
        // poll within a bounded deadline instead of assuming exact scheduling.
        let path = root.join("periodic.ndjson");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let line_count = std::fs::read_to_string(&path)
                .map(|contents| contents.lines().count())
                .unwrap_or(0);
            if line_count == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
        drop(journal);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_run_journals_are_bounded() {
        let root =
            std::env::temp_dir().join(format!("bbt-journal-bounded-{}", rand::random::<u64>()));
        let journal = JournalPublisher::new(root.clone()).unwrap();
        for index in 0..(MAX_OPEN_JOURNALS * 3) {
            let mut event = Envelope::new("run.score_delta", index as u64, serde_json::json!({}));
            event.run_id = Some(format!("run-{index}"));
            journal.publish(&event).unwrap();
        }
        journal.flush();
        assert!(journal.open_writer_count() <= MAX_OPEN_JOURNALS);
        drop(journal);
        let _ = std::fs::remove_dir_all(root);
    }
}
