use crate::mod_payload::SHARED_MOD_PAYLOAD;
use crate::model::{
    GameplayState, RenderSample, RendererRequest, RendererSlot, MAX_RENDER_STREAMS,
};
use anyhow::{bail, Context, Result};
use memmap2::MmapMut;
use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};

const FRAME_HEADER: usize = 64;
const FRAME_COUNT: usize = 3;
const MAX_FRAME_SIZE: usize = 1920 * 1080 * 4;

/// Creates an isolated APPDATA tree for spectator Beatblock processes. Keeping
/// this outside the installer avoids pulling installer UI/payload code into the
/// lightweight runtime binary.
pub fn prepare_renderer_profile(data_dir: &Path) -> Result<PathBuf> {
    let profile = data_dir.join("renderer-profile");
    let directory = profile.join("Beatblock/Mods/BeatblockOnlineRenderer");
    std::fs::create_dir_all(directory.join("bbt"))?;
    std::fs::create_dir_all(directory.join("lovely"))?;
    for (relative, bytes) in SHARED_MOD_PAYLOAD.iter().copied().chain(std::iter::once((
        "lovely/bootstrap.toml",
        include_bytes!("../../mod/standalone/lovely/bootstrap.toml").as_slice(),
    ))) {
        let target = directory.join(relative);
        // The shared inventory can grow beyond the pre-created bbt/lovely
        // folders (for example assets). Always materialize its full layout.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
    }
    Ok(profile)
}

pub struct RendererManager {
    data_dir: PathBuf,
    slots: Mutex<Vec<RendererSlot>>,
    processes: Mutex<HashMap<String, Child>>,
    render_samples: Mutex<HashMap<String, VecDeque<RenderSample>>>,
    player_states: Mutex<HashMap<String, VecDeque<GameplayState>>>,
    input_maps: Mutex<HashMap<String, MmapMut>>,
    frame_observations: Mutex<HashMap<String, (u64, u64, u64)>>,
}

impl RendererManager {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(data_dir.join("render-streams"))?;
        let slots = (0..MAX_RENDER_STREAMS)
            .map(|index| {
                RendererSlot::defaults(&((b'A' + index as u8) as char).to_string(), index == 0)
            })
            .collect();
        Ok(Self {
            data_dir,
            slots: Mutex::new(slots),
            processes: Mutex::new(HashMap::new()),
            render_samples: Mutex::new(HashMap::new()),
            player_states: Mutex::new(HashMap::new()),
            input_maps: Mutex::new(HashMap::new()),
            frame_observations: Mutex::new(HashMap::new()),
        })
    }

    pub fn slots(&self) -> Vec<RendererSlot> {
        self.slots.lock().expect("renderer slots poisoned").clone()
    }

    pub fn slot(&self, slot_id: &str) -> Option<RendererSlot> {
        self.slots()
            .into_iter()
            .find(|slot| slot.id.eq_ignore_ascii_case(slot_id))
    }

    pub fn active_slots(&self) -> Vec<RendererSlot> {
        self.slots()
            .into_iter()
            .filter(|slot| slot.active)
            .collect()
    }

    /// Returns the fastest configured renderer clock. The runtime uses this to
    /// park the input-alignment task when no stream is active and to avoid
    /// polling twice as fast as a 60 Hz renderer can consume samples.
    pub fn active_input_fps(&self) -> Option<u32> {
        self.slots
            .lock()
            .expect("renderer slots poisoned")
            .iter()
            .filter(|slot| slot.active)
            .map(|slot| slot.fps)
            .max()
    }

    pub fn has_active_featured_slot(&self) -> bool {
        self.slots
            .lock()
            .expect("renderer slots poisoned")
            .iter()
            .any(|slot| slot.active && slot.featured)
    }

    pub fn configure(&self, slot_id: &str, request: RendererRequest) -> Result<RendererSlot> {
        // Build and validate a complete candidate before publishing any field.
        // A rejected width/FPS must not leave a participant or mode half-applied.
        let (index, previous) = {
            let slots = self.slots.lock().expect("renderer slots poisoned");
            let index = slots
                .iter()
                .position(|slot| slot.id.eq_ignore_ascii_case(slot_id))
                .ok_or_else(|| anyhow::anyhow!("unknown renderer slot"))?;
            (index, slots[index].clone())
        };
        let mut configured = previous.clone();
        if let Some(participant_id) = request.participant_id {
            configured.participant_id = if participant_id.is_empty() {
                None
            } else {
                Some(participant_id)
            };
        }
        if let Some(participant_name) = request.participant_name {
            configured.participant_name = if participant_name.is_empty() {
                None
            } else {
                Some(participant_name)
            };
        }
        if let Some(mode) = request.mode {
            configured.mode = mode;
        }
        if let Some(width) = request.width {
            if !(320..=1920).contains(&width) {
                bail!("renderer width must be 320-1920");
            }
            configured.width = width;
        }
        if let Some(height) = request.height {
            if !(180..=1080).contains(&height) {
                bail!("renderer height must be 180-1080");
            }
            configured.height = height;
        }
        if let Some(fps) = request.fps {
            if fps != 30 && fps != 60 {
                bail!("renderer FPS must be 30 or 60");
            }
            configured.fps = fps;
        }
        if let Some(delay) = request.delay_ms {
            configured.delay_ms = delay.clamp(250, 1_500);
        }
        configured.active = configured.participant_id.is_some();
        let render_changed = previous.participant_id != configured.participant_id
            || previous.mode != configured.mode
            || previous.width != configured.width
            || previous.height != configured.height
            || previous.fps != configured.fps
            || previous.delay_ms != configured.delay_ms;
        if render_changed {
            configured.healthy = false;
            configured.last_frame_at_ms = None;
            configured.last_error = None;
            configured.frame_sequence = 0;
            configured.dropped_frames = 0;
            configured.actual_fps = 0.0;
        }
        if render_changed || !self.frame_path(&configured.id).is_file() {
            self.create_frame_ring(&configured)?;
            self.create_input_map(&configured.id)?;
            self.frame_observations
                .lock()
                .expect("renderer observations poisoned")
                .remove(&configured.id);
        }
        {
            let mut slots = self.slots.lock().expect("renderer slots poisoned");
            if request.featured == Some(true) {
                for slot in slots.iter_mut() {
                    slot.featured = false;
                }
                configured.featured = true;
            }
            slots[index] = configured.clone();
        }
        if !configured.active {
            self.kill_process(&configured.id);
        }
        Ok(configured)
    }

    pub fn set_error(&self, slot_id: &str, message: impl Into<String>) {
        if let Some(slot) = self
            .slots
            .lock()
            .expect("renderer slots poisoned")
            .iter_mut()
            .find(|slot| slot.id.eq_ignore_ascii_case(slot_id))
        {
            slot.healthy = false;
            slot.last_error = Some(message.into());
        }
    }

    pub fn budget_warning(&self) -> Option<String> {
        let pixels_per_second = self
            .slots()
            .into_iter()
            .filter(|slot| slot.active)
            .map(|slot| slot.width as u64 * slot.height as u64 * slot.fps as u64)
            .sum::<u64>();
        let baseline = 4 * 1280u64 * 720 * 60;
        (pixels_per_second > baseline).then(|| {
            format!(
                "Configured renderer load is {:.1}x the tested four-stream 720p60 budget",
                pixels_per_second as f64 / baseline as f64
            )
        })
    }

    pub fn push_sample(&self, participant_id: &str, mut sample: RenderSample) {
        sample.run_time_us = crate::room::unix_ms() * 1_000;
        let mut buffers = self.render_samples.lock().expect("render buffers poisoned");
        let buffer = buffers.entry(participant_id.into()).or_default();
        buffer.push_back(sample);
        while buffer.len() > 60 * 5 {
            buffer.pop_front();
        }
    }

    pub fn push_player_state(&self, participant_id: &str, state: GameplayState) {
        let mut buffers = self
            .player_states
            .lock()
            .expect("player state buffers poisoned");
        let buffer = buffers.entry(participant_id.into()).or_default();
        buffer.push_back(state);
        while buffer.len() > 30 * 5 {
            buffer.pop_front();
        }
    }

    pub fn aligned_featured_state(&self, now_ms: u64) -> Option<GameplayState> {
        let featured = self.slots().into_iter().find(|slot| slot.featured)?;
        let participant_id = featured.participant_id?;
        let target = now_ms.saturating_sub(featured.delay_ms as u64);
        let buffers = self.player_states.lock().ok()?;
        let buffer = buffers.get(&participant_id)?;
        buffer
            .iter()
            .rev()
            .find(|state| state.updated_at_ms <= target)
            .cloned()
    }

    pub fn frame_path(&self, slot_id: &str) -> PathBuf {
        self.data_dir
            .join("render-streams")
            .join(format!("stream-{}.bbtframe", slot_id.to_ascii_uppercase()))
    }

    pub fn state_path(&self, slot_id: &str) -> PathBuf {
        self.data_dir
            .join("render-streams")
            .join(format!("stream-{}.bbtstate", slot_id.to_ascii_uppercase()))
    }

    pub fn error_path(&self, slot_id: &str) -> PathBuf {
        self.data_dir
            .join("render-streams")
            .join(format!("stream-{}.bbterror", slot_id.to_ascii_uppercase()))
    }

    pub fn write_aligned_inputs(&self, now_us: u64) {
        let slots = self.slots();
        let buffers = self.render_samples.lock().expect("render buffers poisoned");
        let mut maps = self.input_maps.lock().expect("input maps poisoned");
        for slot in slots.into_iter().filter(|slot| slot.active) {
            let Some(participant) = slot.participant_id.as_deref() else {
                continue;
            };
            let Some(buffer) = buffers.get(participant) else {
                continue;
            };
            let target = now_us.saturating_sub(slot.delay_ms as u64 * 1_000);
            let Some(sample) = buffer
                .iter()
                .rev()
                .find(|sample| sample.run_time_us <= target)
            else {
                continue;
            };
            let Some(map) = maps.get_mut(&slot.id) else {
                continue;
            };
            let bytes = sample.encode();
            map[..8].copy_from_slice(&bytes[..8]);
            map[12..].copy_from_slice(&bytes[12..]);
            std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
            map[8..12].copy_from_slice(&bytes[8..12]);
        }
    }

    pub fn launch_slot(
        &self,
        slot_id: &str,
        game_executable: &Path,
        renderer_profile: &Path,
        chart_path: &str,
        variant: &str,
    ) -> Result<()> {
        let slot = self
            .slots()
            .into_iter()
            .find(|slot| slot.id.eq_ignore_ascii_case(slot_id))
            .ok_or_else(|| anyhow::anyhow!("unknown renderer slot"))?;
        if !slot.active {
            bail!("assign a participant before launching the renderer");
        }
        self.kill_process(slot_id);
        self.create_frame_ring(&slot)?;
        let mut command = self.renderer_command(
            &slot,
            game_executable,
            renderer_profile,
            chart_path,
            variant,
        );
        let child = command
            .spawn()
            .context("launch isolated Beatblock renderer")?;
        self.processes
            .lock()
            .expect("renderer processes poisoned")
            .insert(slot.id, child);
        if let Some(current) = self
            .slots
            .lock()
            .expect("renderer slots poisoned")
            .iter_mut()
            .find(|current| current.id.eq_ignore_ascii_case(slot_id))
        {
            current.healthy = false;
            current.last_error = None;
        }
        Ok(())
    }

    /// Builds the child launch in one testable place. `APPDATA` alone does not
    /// redirect Lovely on Windows because its directory resolver uses the
    /// roaming known-folder API. The explicit Lovely override is what prevents
    /// a renderer from injecting the player's full mod set into a second game.
    fn renderer_command(
        &self,
        slot: &RendererSlot,
        game_executable: &Path,
        renderer_profile: &Path,
        chart_path: &str,
        variant: &str,
    ) -> Command {
        let mut command = Command::new(game_executable);
        command
            // Lovely parses Beatblock's process arguments before our Lua
            // bootstrap runs. Keep renderer metadata in the environment so
            // unknown application flags cannot crash the injector.
            .env("BBT_RENDERER_STREAM", &slot.id)
            .env(
                "BBT_RENDERER_MODE",
                format!("{:?}", slot.mode).to_lowercase(),
            )
            .env(
                "BBT_RENDERER_PARTICIPANT",
                slot.participant_id.as_deref().unwrap_or_default(),
            )
            .env("BBT_RENDERER_FRAME_PATH", self.frame_path(&slot.id))
            .env("BBT_RENDERER_ERROR_PATH", self.error_path(&slot.id))
            .env("BBT_RENDERER_WIDTH", slot.width.to_string())
            .env("BBT_RENDERER_HEIGHT", slot.height.to_string())
            .env("BBT_RENDERER_FPS", slot.fps.to_string())
            .env("BBT_RENDERER_DELAY_MS", slot.delay_ms.to_string())
            .env("BBT_RENDERER_AUDIO", if slot.featured { "1" } else { "0" })
            .env("BBT_RENDERER_CHART", chart_path)
            .env("BBT_RENDERER_VARIANT", variant)
            .env("APPDATA", renderer_profile)
            .env("LOVELY_MOD_DIR", renderer_profile.join("Beatblock/Mods"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        command
    }

    pub fn stop_slot(&self, slot_id: &str) {
        self.kill_process(slot_id);
        let reset = {
            let mut slots = self.slots.lock().expect("renderer slots poisoned");
            let Some(slot) = slots
                .iter_mut()
                .find(|slot| slot.id.eq_ignore_ascii_case(slot_id))
            else {
                return;
            };
            slot.participant_id = None;
            slot.participant_name = None;
            slot.active = false;
            slot.healthy = false;
            slot.frame_sequence = 0;
            slot.dropped_frames = 0;
            slot.actual_fps = 0.0;
            slot.last_frame_at_ms = None;
            slot.last_error = None;
            slot.clone()
        };
        if let Err(error) = self.create_frame_ring(&reset) {
            self.set_error(
                &reset.id,
                format!("reset renderer frame ring failed: {error}"),
            );
        }
        self.frame_observations
            .lock()
            .expect("renderer observations poisoned")
            .remove(&reset.id);
    }

    /// Stops only the child process while preserving its desired slot config.
    /// Feature switches use this to silence the old audio source before either
    /// process is relaunched with its new authority.
    pub fn stop_process(&self, slot_id: &str) {
        self.kill_process(slot_id);
    }

    pub fn stop_participant(&self, participant_id: &str) {
        let slots = self
            .slots()
            .into_iter()
            .filter(|slot| slot.participant_id.as_deref() == Some(participant_id))
            .map(|slot| slot.id)
            .collect::<Vec<_>>();
        for slot in slots {
            self.stop_slot(&slot);
        }
        // A disconnected participant can never supply another delayed frame.
        // Release both bounded history queues instead of retaining one pair for
        // every session id seen over the lifetime of the runtime.
        self.render_samples
            .lock()
            .expect("render buffers poisoned")
            .remove(participant_id);
        self.player_states
            .lock()
            .expect("player state buffers poisoned")
            .remove(participant_id);
    }

    fn kill_process(&self, slot_id: &str) {
        if let Some(mut child) = self
            .processes
            .lock()
            .expect("renderer processes poisoned")
            .remove(&slot_id.to_ascii_uppercase())
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Polls renderer children and their frame headers. Dashboard health is
    /// therefore based on frames OBS can actually consume, not assignment state.
    pub fn refresh_health(&self, now_ms: u64) {
        let mut running = HashMap::new();
        let mut exited = Vec::new();
        {
            let mut processes = self.processes.lock().expect("renderer processes poisoned");
            for (id, child) in processes.iter_mut() {
                match child.try_wait() {
                    Ok(None) => {
                        running.insert(id.clone(), true);
                    }
                    Ok(Some(status)) => {
                        exited.push((id.clone(), format!("renderer exited with {status}")))
                    }
                    Err(error) => {
                        exited.push((id.clone(), format!("renderer status failed: {error}")))
                    }
                }
            }
            for (id, _) in &exited {
                processes.remove(id);
            }
        }

        let mut observations = self
            .frame_observations
            .lock()
            .expect("renderer observations poisoned");
        let mut slots = self.slots.lock().expect("renderer slots poisoned");
        for slot in slots.iter_mut().filter(|slot| slot.active) {
            if let Ok(error) = std::fs::read_to_string(self.error_path(&slot.id)) {
                slot.healthy = false;
                slot.last_error = Some(format!("renderer capture failed: {error}"));
                continue;
            }
            if let Some((_, error)) = exited.iter().find(|(id, _)| id == &slot.id) {
                slot.healthy = false;
                slot.last_error = Some(error.clone());
                continue;
            }
            let mut header = [0u8; FRAME_HEADER];
            let header_ok = File::open(self.frame_path(&slot.id))
                .and_then(|mut file| file.read_exact(&mut header))
                .is_ok()
                && &header[..8] == b"BBTFRAME";
            if !header_ok {
                slot.healthy = false;
                continue;
            }
            let sequence = u64::from_le_bytes(header[32..40].try_into().unwrap_or_default());
            let dropped = u64::from_le_bytes(header[48..56].try_into().unwrap_or_default());
            let observation = observations
                .entry(slot.id.clone())
                .or_insert((0, now_ms, now_ms));
            if sequence != observation.0 {
                let elapsed_ms = now_ms.saturating_sub(observation.2);
                if observation.0 > 0 && elapsed_ms > 0 {
                    let measured = (sequence.saturating_sub(observation.0) as f32 * 1_000.0)
                        / elapsed_ms as f32;
                    slot.actual_fps = if slot.actual_fps > 0.0 {
                        slot.actual_fps * 0.75 + measured * 0.25
                    } else {
                        measured
                    };
                }
                *observation = (sequence, now_ms, now_ms);
                slot.last_frame_at_ms = Some(now_ms);
            }
            slot.frame_sequence = sequence;
            slot.dropped_frames = dropped;
            slot.healthy = running.get(&slot.id).copied().unwrap_or(false)
                && sequence > 0
                && now_ms.saturating_sub(observation.1) <= 1_000;
            if !slot.healthy {
                slot.actual_fps = 0.0;
            }
        }
    }

    pub fn stop_all(&self) {
        let ids = self
            .slots()
            .into_iter()
            .map(|slot| slot.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.stop_slot(&id);
        }
        // Room/session shutdown is the ownership boundary for every delayed
        // telemetry queue and writable input map.
        self.render_samples
            .lock()
            .expect("render buffers poisoned")
            .clear();
        self.player_states
            .lock()
            .expect("player state buffers poisoned")
            .clear();
        self.input_maps.lock().expect("input maps poisoned").clear();
        self.frame_observations
            .lock()
            .expect("renderer observations poisoned")
            .clear();
    }

    fn create_frame_ring(&self, slot: &RendererSlot) -> Result<()> {
        let stride = slot.width as usize * 4;
        let frame_size = stride * slot.height as usize;
        // Every stream file keeps one stable maximum-size mapping. Resolution
        // changes only reset the header, so OBS can retain a read-only mapping
        // without reopening a multi-megabyte file every video tick.
        let total_size = FRAME_HEADER + MAX_FRAME_SIZE * FRAME_COUNT;
        let path = self.frame_path(&slot.id);
        // A new renderer attempt must not inherit capture diagnostics from a
        // previous child.
        let _ = std::fs::remove_file(self.error_path(&slot.id));
        let file = OpenOptions::new()
            .create(true)
            // Keep the maximum-size mapping stable so an OBS source can retain
            // its read-only view while a stream is reconfigured.
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        file.set_len(total_size as u64)?;
        let mut map = unsafe { MmapMut::map_mut(&file)? };
        map[..8].copy_from_slice(b"BBTFRAME");
        map[8..12].copy_from_slice(&2u32.to_le_bytes());
        map[12..16].copy_from_slice(&slot.width.to_le_bytes());
        map[16..20].copy_from_slice(&slot.height.to_le_bytes());
        map[20..24].copy_from_slice(&(stride as u32).to_le_bytes());
        map[24..28].copy_from_slice(&(FRAME_COUNT as u32).to_le_bytes());
        map[28..32].fill(0);
        map[32..40].copy_from_slice(&0u64.to_le_bytes());
        map[40..48].copy_from_slice(&(frame_size as u64).to_le_bytes());
        map[48..56].copy_from_slice(&0u64.to_le_bytes());
        map.flush()?;
        Ok(())
    }

    fn create_input_map(&self, slot_id: &str) -> Result<()> {
        let path = self.state_path(slot_id);
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)?;
        file.set_len(RenderSample::WIRE_SIZE as u64)?;
        let mut map = unsafe { MmapMut::map_mut(&file)? };
        map.fill(0);
        self.input_maps
            .lock()
            .expect("input maps poisoned")
            .insert(slot_id.to_ascii_uppercase(), map);
        Ok(())
    }
}

impl Drop for RendererManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RendererMode, RendererRequest};
    use std::ffi::OsStr;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn enforces_stream_limits_and_creates_frame_contract() {
        let root = std::env::temp_dir().join(format!("bbt-renderer-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        assert_eq!(manager.slot("A").unwrap().mode, RendererMode::Full);
        let slot = manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("player-1".into()),
                    participant_name: Some("Player 1".into()),
                    mode: Some(RendererMode::Full),
                    width: Some(1280),
                    height: Some(720),
                    fps: Some(60),
                    delay_ms: Some(100),
                    featured: Some(true),
                },
            )
            .unwrap();
        assert_eq!(slot.delay_ms, 250);
        assert!(manager.has_active_featured_slot());
        assert_eq!(
            &std::fs::read(manager.frame_path("A")).unwrap()[..8],
            b"BBTFRAME"
        );
        let bytes = std::fs::read(manager.frame_path("A")).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
        assert_eq!(bytes.len(), FRAME_HEADER + MAX_FRAME_SIZE * FRAME_COUNT);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejected_renderer_configuration_is_transactional() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-atomic-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        let before = manager.slot("A").unwrap();
        let error = manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("should-not-stick".into()),
                    participant_name: Some("Invalid".into()),
                    mode: Some(RendererMode::Full),
                    width: Some(8_000),
                    height: Some(720),
                    fps: Some(60),
                    delay_ms: Some(500),
                    featured: Some(true),
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("width"));
        let after = manager.slot("A").unwrap();
        assert_eq!(after.participant_id, before.participant_id);
        assert_eq!(after.participant_name, before.participant_name);
        assert_eq!(after.mode, before.mode);
        assert_eq!(after.featured, before.featured);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn feature_only_updates_do_not_reset_frames_and_stop_deactivates_slot() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-state-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        manager
            .configure(
                "B",
                RendererRequest {
                    participant_id: Some("player-1".into()),
                    participant_name: Some("Player 1".into()),
                    mode: Some(RendererMode::Clean),
                    width: Some(320),
                    height: Some(180),
                    fps: Some(60),
                    delay_ms: Some(500),
                    featured: Some(false),
                },
            )
            .unwrap();
        assert_eq!(manager.active_input_fps(), Some(60));
        let path = manager.frame_path("B");
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(32)).unwrap();
        file.write_all(&7u64.to_le_bytes()).unwrap();
        drop(file);

        manager
            .configure(
                "B",
                RendererRequest {
                    participant_id: None,
                    participant_name: None,
                    mode: None,
                    width: None,
                    height: None,
                    fps: None,
                    delay_ms: None,
                    featured: Some(true),
                },
            )
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(u64::from_le_bytes(bytes[32..40].try_into().unwrap()), 7);

        manager.stop_slot("B");
        assert_eq!(manager.active_input_fps(), None);
        let stopped = manager.slot("B").unwrap();
        assert!(!stopped.active);
        assert!(stopped.participant_id.is_none());
        assert_eq!(
            u64::from_le_bytes(std::fs::read(&path).unwrap()[32..40].try_into().unwrap()),
            0
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn renderer_launch_uses_only_the_isolated_lovely_mod_directory() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-profile-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        let slot = manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("host-session".into()),
                    participant_name: Some("Host".into()),
                    mode: Some(RendererMode::Clean),
                    width: Some(1280),
                    height: Some(720),
                    fps: Some(60),
                    delay_ms: Some(500),
                    featured: Some(true),
                },
            )
            .unwrap();
        let profile = prepare_renderer_profile(&root).unwrap();
        let command = manager.renderer_command(
            &slot,
            Path::new("Beatblock.exe"),
            &profile,
            "Custom Levels/Test/",
            "Default",
        );
        let env = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == OsStr::new(name))
                .and_then(|(_, value)| value)
                .map(PathBuf::from)
        };

        assert_eq!(env("APPDATA"), Some(profile.clone()));
        assert_eq!(env("LOVELY_MOD_DIR"), Some(profile.join("Beatblock/Mods")));
        assert_eq!(env("BBT_RENDERER_STREAM"), Some(PathBuf::from("A")));
        assert_eq!(env("BBT_RENDERER_AUDIO"), Some(PathBuf::from("1")));
        assert_eq!(
            env("BBT_RENDERER_ERROR_PATH"),
            Some(manager.error_path("A"))
        );
        assert!(
            command.get_args().next().is_none(),
            "Lovely owns Beatblock's command-line parser; renderer metadata must use environment variables"
        );
        assert!(profile
            .join("Beatblock/Mods/BeatblockOnlineRenderer/lovely/bootstrap.toml")
            .is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn participant_and_room_shutdown_release_delayed_renderer_state() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-cleanup-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        manager.push_sample(
            "old-session",
            RenderSample {
                session_id: 1,
                sequence: 1,
                run_time_us: 0,
                beat: 1.0,
                paddle_angle: 2.0,
                tap_mask: 0,
                flags: 1,
            },
        );
        manager.push_player_state("old-session", GameplayState::default());
        manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("old-session".into()),
                    participant_name: Some("Old player".into()),
                    mode: None,
                    width: None,
                    height: None,
                    fps: None,
                    delay_ms: None,
                    featured: None,
                },
            )
            .unwrap();

        manager.stop_participant("old-session");
        assert!(!manager
            .render_samples
            .lock()
            .unwrap()
            .contains_key("old-session"));
        assert!(!manager
            .player_states
            .lock()
            .unwrap()
            .contains_key("old-session"));

        manager.push_sample(
            "next-session",
            RenderSample {
                session_id: 2,
                sequence: 1,
                run_time_us: 0,
                beat: 3.0,
                paddle_angle: 4.0,
                tap_mask: 1,
                flags: 1,
            },
        );
        manager.push_player_state("next-session", GameplayState::default());
        manager.stop_all();
        assert!(manager.render_samples.lock().unwrap().is_empty());
        assert!(manager.player_states.lock().unwrap().is_empty());
        assert!(manager.input_maps.lock().unwrap().is_empty());
        assert!(manager.frame_observations.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aligned_input_uses_the_exact_delayed_sixty_hz_sample() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-sync-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("player-1".into()),
                    participant_name: Some("Player 1".into()),
                    mode: Some(RendererMode::Clean),
                    width: Some(320),
                    height: Some(180),
                    fps: Some(60),
                    delay_ms: Some(500),
                    featured: None,
                },
            )
            .unwrap();
        manager.render_samples.lock().unwrap().insert(
            "player-1".into(),
            VecDeque::from([
                RenderSample {
                    session_id: 0,
                    sequence: 1,
                    run_time_us: 1_000_000,
                    beat: 10.0,
                    paddle_angle: 20.0,
                    tap_mask: 1,
                    flags: 1,
                },
                RenderSample {
                    session_id: 0,
                    sequence: 2,
                    run_time_us: 1_016_667,
                    beat: 10.25,
                    paddle_angle: 21.0,
                    tap_mask: 2,
                    flags: 1,
                },
            ]),
        );

        manager.write_aligned_inputs(1_516_667);

        let sample =
            RenderSample::decode(&std::fs::read(manager.state_path("A")).unwrap()).unwrap();
        assert_eq!(sample.sequence, 2);
        assert_eq!(sample.beat, 10.25);
        assert_eq!(sample.paddle_angle, 21.0);
        assert_eq!(sample.tap_mask, 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aligned_input_waits_until_the_configured_delay_has_elapsed() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-preroll-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("player-1".into()),
                    participant_name: Some("Player 1".into()),
                    mode: Some(RendererMode::Full),
                    width: Some(320),
                    height: Some(180),
                    fps: Some(60),
                    delay_ms: Some(500),
                    featured: None,
                },
            )
            .unwrap();
        manager.render_samples.lock().unwrap().insert(
            "player-1".into(),
            VecDeque::from([RenderSample {
                session_id: 0,
                sequence: 7,
                run_time_us: 1_000_000,
                beat: -8.0,
                paddle_angle: 135.0,
                tap_mask: 0,
                flags: 1,
            }]),
        );

        manager.write_aligned_inputs(1_499_999);
        assert!(
            std::fs::read(manager.state_path("A"))
                .unwrap()
                .iter()
                .all(|byte| *byte == 0),
            "the first frame must not bypass the configured spectator delay"
        );

        manager.write_aligned_inputs(1_500_000);
        let sample =
            RenderSample::decode(&std::fs::read(manager.state_path("A")).unwrap()).unwrap();
        assert_eq!(sample.sequence, 7);
        assert_eq!(sample.paddle_angle, 135.0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn featured_exports_wait_for_the_same_delay_as_video() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-exports-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        manager
            .configure(
                "A",
                RendererRequest {
                    participant_id: Some("player-1".into()),
                    participant_name: Some("Player 1".into()),
                    mode: Some(RendererMode::Full),
                    width: None,
                    height: None,
                    fps: None,
                    delay_ms: Some(500),
                    featured: Some(true),
                },
            )
            .unwrap();
        manager.player_states.lock().unwrap().insert(
            "player-1".into(),
            VecDeque::from([GameplayState {
                updated_at_ms: 1_000,
                player_name: "Delayed player".into(),
                ..GameplayState::default()
            }]),
        );

        assert!(manager.aligned_featured_state(1_499).is_none());
        assert_eq!(
            manager.aligned_featured_state(1_500).unwrap().player_name,
            "Delayed player"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
