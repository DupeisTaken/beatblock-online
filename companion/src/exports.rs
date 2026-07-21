use crate::model::{GameplayState, RendererSlot, RoomSnapshot};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct PendingExports {
    gameplay: Option<GameplayState>,
    room: Option<(RoomSnapshot, Vec<RendererSlot>)>,
    // The outer option means "a featured update is pending"; the inner option
    // distinguishes a new featured state from an explicit clear operation.
    featured: Option<Option<GameplayState>>,
    broadcast_metadata: Option<BroadcastExportMetadata>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastExportMetadata {
    pub plan_revision: u64,
    pub authority: &'static str,
    pub local_mirror_enabled: bool,
}

enum ExportSignal {
    Wake,
    Flush(mpsc::SyncSender<()>),
}

/// Coalesces high-rate snapshots onto one filesystem worker. Runtime IPC and
/// renderer clocks only publish immutable state here and never wait for disk
/// flushes or antivirus/file-indexer latency.
#[derive(Clone)]
pub struct ExportPublisher {
    pending: Arc<Mutex<PendingExports>>,
    wake_queued: Arc<AtomicBool>,
    completed_writes: Arc<AtomicU64>,
    completed_bytes: Arc<AtomicU64>,
    signals: mpsc::Sender<ExportSignal>,
}

impl ExportPublisher {
    pub fn new(directory: PathBuf) -> Result<Self> {
        let pending = Arc::new(Mutex::new(PendingExports::default()));
        let wake_queued = Arc::new(AtomicBool::new(false));
        let completed_writes = Arc::new(AtomicU64::new(0));
        let completed_bytes = Arc::new(AtomicU64::new(0));
        let (signals, receiver) = mpsc::channel();
        let worker_pending = pending.clone();
        let worker_wake = wake_queued.clone();
        let worker_writes = completed_writes.clone();
        let worker_bytes = completed_bytes.clone();
        std::thread::Builder::new()
            .name("bbt-exports".into())
            .spawn(move || {
                let mut cache = HashMap::new();
                while let Ok(signal) = receiver.recv() {
                    let mut flushers = Vec::new();
                    if let ExportSignal::Flush(done) = signal {
                        flushers.push(done);
                    } else {
                        // OBS text does not benefit from per-frame filesystem
                        // churn. Hold one short window so gameplay, featured,
                        // and room updates collapse into a single coherent batch.
                        let deadline = Instant::now() + Duration::from_millis(100);
                        loop {
                            let remaining = deadline.saturating_duration_since(Instant::now());
                            if remaining.is_zero() {
                                break;
                            }
                            match receiver.recv_timeout(remaining) {
                                Ok(ExportSignal::Wake) => {}
                                Ok(ExportSignal::Flush(done)) => {
                                    flushers.push(done);
                                    break;
                                }
                                Err(mpsc::RecvTimeoutError::Timeout) => break,
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    }
                    worker_wake.store(false, Ordering::Release);
                    write_pending(
                        &directory,
                        &worker_pending,
                        &mut cache,
                        &worker_writes,
                        &worker_bytes,
                    );
                    for done in flushers {
                        let _ = done.send(());
                    }
                }
            })
            .context("spawn OBS export worker")?;
        Ok(Self {
            pending,
            wake_queued,
            completed_writes,
            completed_bytes,
            signals,
        })
    }

    pub fn publish_gameplay(&self, state: GameplayState) {
        self.pending.lock().expect("export queue poisoned").gameplay = Some(state);
        self.wake();
    }

    pub fn publish_room(&self, room: RoomSnapshot, slots: Vec<RendererSlot>) {
        self.pending.lock().expect("export queue poisoned").room = Some((room, slots));
        self.wake();
    }

    pub fn publish_featured(&self, state: Option<GameplayState>) {
        self.pending.lock().expect("export queue poisoned").featured = Some(state);
        self.wake();
    }

    pub fn publish_broadcast_metadata(
        &self,
        plan_revision: u64,
        authority: &'static str,
        local_mirror_enabled: bool,
    ) {
        self.pending
            .lock()
            .expect("export queue poisoned")
            .broadcast_metadata = Some(BroadcastExportMetadata {
            plan_revision,
            authority,
            local_mirror_enabled,
        });
        self.wake();
    }

    fn wake(&self) {
        if !self.wake_queued.swap(true, Ordering::AcqRel) {
            let _ = self.signals.send(ExportSignal::Wake);
        }
    }

    /// Waits until every snapshot published before this call has been written.
    /// Production uses this during shutdown; tests use it before inspecting files.
    pub fn flush(&self) {
        let (done, complete) = mpsc::sync_channel(0);
        if self.signals.send(ExportSignal::Flush(done)).is_ok() {
            let _ = complete.recv_timeout(Duration::from_secs(5));
        }
    }

    pub fn completed_writes(&self) -> u64 {
        self.completed_writes.load(Ordering::Relaxed)
    }

    pub fn completed_bytes(&self) -> u64 {
        self.completed_bytes.load(Ordering::Relaxed)
    }
}

fn write_pending(
    directory: &Path,
    pending: &Mutex<PendingExports>,
    cache: &mut HashMap<PathBuf, String>,
    completed_writes: &AtomicU64,
    completed_bytes: &AtomicU64,
) {
    let batch = std::mem::take(&mut *pending.lock().expect("export queue poisoned"));
    let mut write = |path: PathBuf, content: &str| {
        if cache.get(&path).is_some_and(|previous| previous == content) {
            return Ok(());
        }
        atomic(path.clone(), content)?;
        cache.insert(path, content.to_owned());
        completed_writes.fetch_add(1, Ordering::Relaxed);
        completed_bytes.fetch_add(content.len() as u64, Ordering::Relaxed);
        Ok(())
    };
    let result = (|| -> Result<()> {
        if let Some((room, slots)) = batch.room {
            write_room_exports_with(directory, &room, &slots, &mut write)?;
        }
        if let Some(gameplay) = batch.gameplay {
            write_exports_with(directory, &gameplay, &mut write)?;
        }
        if let Some(featured) = batch.featured {
            if let Some(featured) = featured {
                write_featured_exports_with(directory, &featured, &mut write)?;
            } else {
                clear_featured_exports_with(directory, &mut write)?;
            }
        }
        if let Some(metadata) = batch.broadcast_metadata {
            write(
                directory.join("broadcast_metadata.json"),
                &serde_json::to_string_pretty(&metadata)?,
            )?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        tracing::warn!(%error, "OBS export batch failed");
    }
}

pub fn write_exports(directory: &Path, state: &GameplayState) -> Result<()> {
    write_exports_with(directory, state, &mut |path, content| atomic(path, content))
}

fn write_exports_with(
    directory: &Path,
    state: &GameplayState,
    write: &mut impl FnMut(PathBuf, &str) -> Result<()>,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    write(directory.join("player_name.txt"), &state.player_name)?;
    write(directory.join("song_name.txt"), &state.song_name)?;
    write(
        directory.join("accuracy.txt"),
        &format!("{:.2}%", state.accuracy),
    )?;
    write(directory.join("combo.txt"), &state.combo.to_string())?;
    write(directory.join("misses.txt"), &state.misses.to_string())?;
    write(directory.join("rank.txt"), &state.rank.to_string())?;
    write(directory.join("lobby_name.txt"), &state.lobby_name)?;
    write(
        directory.join("gameplay.json"),
        &serde_json::to_string_pretty(state)?,
    )?;
    Ok(())
}

pub fn write_featured_exports(directory: &Path, state: &GameplayState) -> Result<()> {
    write_featured_exports_with(directory, state, &mut |path, content| atomic(path, content))
}

fn write_featured_exports_with(
    directory: &Path,
    state: &GameplayState,
    write: &mut impl FnMut(PathBuf, &str) -> Result<()>,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    write(directory.join("featured_name.txt"), &state.player_name)?;
    write(
        directory.join("featured_accuracy.txt"),
        &format!("{:.2}%", state.accuracy),
    )?;
    write(
        directory.join("featured_combo.txt"),
        &state.combo.to_string(),
    )?;
    write(
        directory.join("featured_misses.txt"),
        &state.misses.to_string(),
    )?;
    write(directory.join("featured_rank.txt"), &state.rank.to_string())?;
    Ok(())
}

pub fn clear_featured_exports(directory: &Path) -> Result<()> {
    clear_featured_exports_with(directory, &mut |path, content| atomic(path, content))
}

fn clear_featured_exports_with(
    directory: &Path,
    write: &mut impl FnMut(PathBuf, &str) -> Result<()>,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    for name in [
        "featured_name.txt",
        "featured_accuracy.txt",
        "featured_combo.txt",
        "featured_misses.txt",
        "featured_rank.txt",
    ] {
        write(directory.join(name), "")?;
    }
    Ok(())
}

fn clear_stream_exports_with(
    directory: &Path,
    write: &mut impl FnMut(PathBuf, &str) -> Result<()>,
) -> Result<()> {
    // OBS text sources retain their last file contents indefinitely. Clearing
    // every participant-derived field makes an unassigned or departed stream
    // visibly empty instead of impersonating its previous player.
    for name in [
        "player_name.txt",
        "accuracy.txt",
        "combo.txt",
        "misses.txt",
        "rank.txt",
    ] {
        write(directory.join(name), "")?;
    }
    Ok(())
}

pub fn write_room_exports(
    directory: &Path,
    room: &RoomSnapshot,
    slots: &[RendererSlot],
) -> Result<()> {
    write_room_exports_with(directory, room, slots, &mut |path, content| {
        atomic(path, content)
    })
}

fn write_room_exports_with(
    directory: &Path,
    room: &RoomSnapshot,
    slots: &[RendererSlot],
    write: &mut impl FnMut(PathBuf, &str) -> Result<()>,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    write(directory.join("room_name.txt"), &room.name)?;
    write(directory.join("lobby_name.txt"), &room.name)?;
    for slot in slots {
        let slot_directory = directory.join("streams").join(&slot.id);
        std::fs::create_dir_all(&slot_directory)?;
        write(
            slot_directory.join("state.json"),
            &serde_json::to_string_pretty(slot)?,
        )?;
        let participant = slot.participant_id.as_deref().and_then(|participant_id| {
            room.participants
                .iter()
                .find(|participant| participant.session_id == participant_id)
        });
        if let Some(participant) = participant {
            write(
                slot_directory.join("player_name.txt"),
                &participant.display_name,
            )?;
            write(
                slot_directory.join("accuracy.txt"),
                &format!("{:.2}%", participant.accuracy),
            )?;
            write(
                slot_directory.join("combo.txt"),
                &participant.totals.combo.to_string(),
            )?;
            write(
                slot_directory.join("misses.txt"),
                &participant.totals.misses.to_string(),
            )?;
            write(
                slot_directory.join("rank.txt"),
                &participant.rank.unwrap_or(0).to_string(),
            )?;
        } else {
            clear_stream_exports_with(&slot_directory, write)?;
        }
    }
    write(
        directory.join("state.json"),
        &serde_json::to_string_pretty(&serde_json::json!({
            "room": room,
            "streams": slots,
        }))?,
    )?;
    Ok(())
}

pub fn atomic(path: PathBuf, content: &str) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    replace_file_fast(&temporary, &path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file_fast(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_fast(source: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(source, destination)?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_gameplay_never_overwrites_featured_exports() {
        let root = std::env::temp_dir().join(format!("bbt-exports-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        atomic(root.join("featured_name.txt"), "Remote player").unwrap();
        let state = GameplayState {
            player_name: "Host".into(),
            song_name: "Song".into(),
            lobby_name: "Room".into(),
            ..GameplayState::default()
        };

        write_exports(&root, &state).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("featured_name.txt")).unwrap(),
            "Remote player"
        );
        assert!(root.join("gameplay.json").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clearing_featured_exports_removes_stale_values() {
        let root = std::env::temp_dir().join(format!("bbt-featured-{}", rand::random::<u64>()));
        write_featured_exports(
            &root,
            &GameplayState {
                player_name: "Player".into(),
                ..GameplayState::default()
            },
        )
        .unwrap();

        clear_featured_exports(&root).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("featured_name.txt")).unwrap(),
            ""
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unassigned_or_departed_streams_clear_stale_player_text() {
        use crate::{
            model::{AdmissionMode, RendererMode},
            room::RoomEngine,
        };

        let root =
            std::env::temp_dir().join(format!("bbt-stream-export-clear-{}", rand::random::<u64>()));
        let room = RoomEngine::host("Room".into(), "Player".into(), AdmissionMode::PasswordOnly);
        let participant_id = room.snapshot.host_session_id.clone();
        let mut slot = RendererSlot::defaults("A", true);
        slot.active = true;
        slot.participant_id = Some(participant_id.clone());
        slot.participant_name = Some("Player".into());
        slot.mode = RendererMode::Full;

        write_room_exports(&root, &room.snapshot, &[slot.clone()]).unwrap();
        let stream = root.join("streams/A");
        assert_eq!(
            std::fs::read_to_string(stream.join("player_name.txt")).unwrap(),
            "Player"
        );

        // A stale assignment can outlive the participant snapshot briefly.
        let mut without_player = room.snapshot.clone();
        without_player.participants.clear();
        write_room_exports(&root, &without_player, &[slot.clone()]).unwrap();
        for name in [
            "player_name.txt",
            "accuracy.txt",
            "combo.txt",
            "misses.txt",
            "rank.txt",
        ] {
            assert_eq!(std::fs::read_to_string(stream.join(name)).unwrap(), "");
        }

        // Reassignment can repopulate the files, and an explicit unassign must
        // clear them again for OBS text sources that watch stable paths.
        write_room_exports(&root, &room.snapshot, &[slot.clone()]).unwrap();
        slot.participant_id = None;
        slot.participant_name = None;
        write_room_exports(&root, &room.snapshot, &[slot]).unwrap();
        assert_eq!(
            std::fs::read_to_string(stream.join("player_name.txt")).unwrap(),
            ""
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn publisher_skips_unchanged_export_files() {
        let root = std::env::temp_dir().join(format!("bbt-export-cache-{}", rand::random::<u64>()));
        let publisher = ExportPublisher::new(root.clone()).unwrap();
        let state = GameplayState {
            player_name: "Stable".into(),
            song_name: "No churn".into(),
            ..GameplayState::default()
        };
        publisher.publish_gameplay(state.clone());
        publisher.flush();
        let path = root.join("gameplay.json");
        let first_modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        publisher.publish_gameplay(state);
        publisher.flush();
        let second_modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(first_modified, second_modified);
        assert_eq!(publisher.completed_writes(), 8);
        drop(publisher);
        let _ = std::fs::remove_dir_all(root);
    }
}
