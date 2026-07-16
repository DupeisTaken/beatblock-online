use crate::mod_payload::SHARED_MOD_PAYLOAD;
use crate::model::{
    GameplayState, RenderSample, RendererRequest, RendererSlot, MAX_RENDER_STREAMS,
};
use anyhow::{bail, Context, Result};
use memmap2::MmapMut;
use std::{
    collections::{HashMap, VecDeque},
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};

const FRAME_HEADER: usize = 64;
const FRAME_COUNT: usize = 3;

/// Creates an isolated APPDATA tree for spectator Beatblock processes. Keeping
/// this outside the installer avoids pulling installer UI/payload code into the
/// lightweight runtime binary.
pub fn prepare_renderer_profile(data_dir: &Path) -> Result<PathBuf> {
    let profile = data_dir.join("renderer-profile");
    let directory = profile.join("Beatblock/Mods/BeatblockTogetherRenderer");
    std::fs::create_dir_all(directory.join("bbt"))?;
    std::fs::create_dir_all(directory.join("lovely"))?;
    for (relative, bytes) in SHARED_MOD_PAYLOAD.iter().copied().chain(std::iter::once((
        "lovely/bootstrap.toml",
        include_bytes!("../../mod/standalone/lovely/bootstrap.toml").as_slice(),
    ))) {
        std::fs::write(directory.join(relative), bytes)?;
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
        })
    }

    pub fn slots(&self) -> Vec<RendererSlot> {
        self.slots.lock().expect("renderer slots poisoned").clone()
    }

    pub fn configure(&self, slot_id: &str, request: RendererRequest) -> Result<RendererSlot> {
        let mut slots = self.slots.lock().expect("renderer slots poisoned");
        let index = slots
            .iter()
            .position(|slot| slot.id.eq_ignore_ascii_case(slot_id))
            .ok_or_else(|| anyhow::anyhow!("unknown renderer slot"))?;
        if let Some(participant_id) = request.participant_id {
            slots[index].participant_id = if participant_id.is_empty() {
                None
            } else {
                Some(participant_id)
            };
        }
        if let Some(participant_name) = request.participant_name {
            slots[index].participant_name = if participant_name.is_empty() {
                None
            } else {
                Some(participant_name)
            };
        }
        if let Some(mode) = request.mode {
            slots[index].mode = mode;
        }
        if let Some(width) = request.width {
            if !(320..=1920).contains(&width) {
                bail!("renderer width must be 320-1920");
            }
            slots[index].width = width;
        }
        if let Some(height) = request.height {
            if !(180..=1080).contains(&height) {
                bail!("renderer height must be 180-1080");
            }
            slots[index].height = height;
        }
        if let Some(fps) = request.fps {
            if fps != 30 && fps != 60 {
                bail!("renderer FPS must be 30 or 60");
            }
            slots[index].fps = fps;
        }
        if let Some(delay) = request.delay_ms {
            slots[index].delay_ms = delay.clamp(250, 1_500);
        }
        if request.featured == Some(true) {
            for slot in slots.iter_mut() {
                slot.featured = false;
            }
            slots[index].featured = true;
        }
        slots[index].active = slots[index].participant_id.is_some();
        let configured = slots[index].clone();
        drop(slots);
        self.create_frame_ring(&configured)?;
        self.create_input_map(&configured.id)?;
        Ok(configured)
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
            .or_else(|| buffer.front().cloned())
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
                .or_else(|| buffer.front())
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
        self.stop_slot(slot_id);
        self.create_frame_ring(&slot)?;
        let mut command = Command::new(game_executable);
        command
            .arg("--bbt-renderer")
            .arg(format!("--bbt-stream={}", slot.id))
            .env(
                "BBT_RENDERER_MODE",
                format!("{:?}", slot.mode).to_lowercase(),
            )
            .env(
                "BBT_RENDERER_PARTICIPANT",
                slot.participant_id.as_deref().unwrap_or_default(),
            )
            .env("BBT_RENDERER_FRAME_PATH", self.frame_path(&slot.id))
            .env("BBT_RENDERER_WIDTH", slot.width.to_string())
            .env("BBT_RENDERER_HEIGHT", slot.height.to_string())
            .env("BBT_RENDERER_FPS", slot.fps.to_string())
            .env("BBT_RENDERER_DELAY_MS", slot.delay_ms.to_string())
            .env("BBT_RENDERER_CHART", chart_path)
            .env("BBT_RENDERER_VARIANT", variant)
            .env("APPDATA", renderer_profile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        let child = command
            .spawn()
            .context("launch isolated Beatblock renderer")?;
        self.processes
            .lock()
            .expect("renderer processes poisoned")
            .insert(slot.id, child);
        Ok(())
    }

    pub fn stop_slot(&self, slot_id: &str) {
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

    pub fn stop_all(&self) {
        let ids = self
            .processes
            .lock()
            .expect("renderer processes poisoned")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            self.stop_slot(&id);
        }
    }

    fn create_frame_ring(&self, slot: &RendererSlot) -> Result<()> {
        let stride = slot.width as usize * 4;
        let frame_size = stride * slot.height as usize;
        let total_size = FRAME_HEADER + frame_size * FRAME_COUNT;
        let path = self.frame_path(&slot.id);
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)?;
        file.set_len(total_size as u64)?;
        let mut map = unsafe { MmapMut::map_mut(&file)? };
        map[..8].copy_from_slice(b"BBTFRAME");
        map[8..12].copy_from_slice(&1u32.to_le_bytes());
        map[12..16].copy_from_slice(&slot.width.to_le_bytes());
        map[16..20].copy_from_slice(&slot.height.to_le_bytes());
        map[20..24].copy_from_slice(&(stride as u32).to_le_bytes());
        map[24..28].copy_from_slice(&(FRAME_COUNT as u32).to_le_bytes());
        map[28..36].copy_from_slice(&0u64.to_le_bytes());
        map[36..44].copy_from_slice(&(frame_size as u64).to_le_bytes());
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

    #[test]
    fn enforces_stream_limits_and_creates_frame_contract() {
        let root = std::env::temp_dir().join(format!("bbt-renderer-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
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
        assert_eq!(
            &std::fs::read(manager.frame_path("A")).unwrap()[..8],
            b"BBTFRAME"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
