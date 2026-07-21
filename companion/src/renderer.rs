use crate::mod_payload::SHARED_MOD_PAYLOAD;
use crate::model::{
    GameplayState, RenderSample, RendererRequest, RendererSlot, MAX_RENDER_STREAMS,
};
use anyhow::{bail, Context, Result};
use memmap2::MmapMut;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};

const FRAME_HEADER: usize = 64;
const FRAME_COUNT: usize = 3;
const MAX_FRAME_SIZE: usize = 1920 * 1080 * 4;
const MAX_RENDER_SAMPLES: usize = 60 * 60;
const MAX_RENDER_TAPS: usize = 2_048;

const FLAG_PLAYING: u16 = 1;
const FLAG_PARKED: u16 = 1 << 3;
const FLAG_CAPTURE_ENABLED: u16 = 1 << 4;
const FLAG_SYNC_RELEASE: u16 = 1 << 5;
const FLAG_TAP_PRESSED: u16 = 1 << 6;
const FLAG_TAP_RELEASED: u16 = 1 << 7;

#[derive(Clone)]
struct BufferedRenderSample {
    sample: RenderSample,
    received_at_us: u64,
}

#[derive(Clone, Copy, Debug)]
struct RenderAnchor {
    first_note_beat: f32,
    input_offset_ms: f32,
    source_time_us: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct RenderTap {
    sequence: u64,
    beat: f32,
    judgement_beat: f32,
    pressed: bool,
    released: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SyncEpoch {
    participants: Vec<String>,
    release_at_us: u64,
    released: bool,
}

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
    render_samples: Mutex<HashMap<String, VecDeque<BufferedRenderSample>>>,
    render_anchors: Mutex<HashMap<String, RenderAnchor>>,
    render_taps: Mutex<HashMap<String, VecDeque<RenderTap>>>,
    player_states: Mutex<HashMap<String, VecDeque<GameplayState>>>,
    input_maps: Mutex<HashMap<String, MmapMut>>,
    input_sequences: Mutex<HashMap<String, u32>>,
    input_sources: Mutex<HashMap<String, String>>,
    tap_cursors: Mutex<HashMap<String, u64>>,
    sync_epoch: Mutex<Option<SyncEpoch>>,
    presentation_origin_us: Mutex<Option<u64>>,
    presentation_samples: Mutex<HashMap<String, RenderSample>>,
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
            render_anchors: Mutex::new(HashMap::new()),
            render_taps: Mutex::new(HashMap::new()),
            player_states: Mutex::new(HashMap::new()),
            input_maps: Mutex::new(HashMap::new()),
            input_sequences: Mutex::new(HashMap::new()),
            input_sources: Mutex::new(HashMap::new()),
            tap_cursors: Mutex::new(HashMap::new()),
            sync_epoch: Mutex::new(None),
            presentation_origin_us: Mutex::new(None),
            presentation_samples: Mutex::new(HashMap::new()),
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
        configured.parked = false;
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
            self.reset_input_map(&configured.id)?;
            self.input_sources
                .lock()
                .expect("renderer input sources poisoned")
                .remove(&configured.id);
            self.tap_cursors
                .lock()
                .expect("renderer tap cursors poisoned")
                .remove(&configured.id);
            *self
                .sync_epoch
                .lock()
                .expect("renderer sync epoch poisoned") = None;
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

    pub fn push_sample(&self, participant_id: &str, sample: RenderSample) {
        self.push_sample_at(participant_id, sample, crate::room::unix_ms() * 1_000);
    }

    fn push_sample_at(&self, participant_id: &str, sample: RenderSample, received_at_us: u64) {
        // Keep the sender's monotonic timestamp. Arrival time is recorded
        // separately for pre-anchor buffering; after the first-note barrier all
        // sources are selected in their own source-relative clock domains.
        let mut buffers = self.render_samples.lock().expect("render buffers poisoned");
        let buffer = buffers.entry(participant_id.into()).or_default();
        if buffer
            .back()
            .is_some_and(|current| sample.sequence <= current.sample.sequence)
        {
            return;
        }
        buffer.push_back(BufferedRenderSample {
            sample,
            received_at_us,
        });
        while buffer.len() > MAX_RENDER_SAMPLES {
            buffer.pop_front();
        }
    }

    pub fn push_render_anchor(
        &self,
        participant_id: &str,
        first_note_beat: f32,
        input_offset_ms: f32,
    ) {
        if !first_note_beat.is_finite() || !input_offset_ms.is_finite() {
            return;
        }
        self.render_anchors
            .lock()
            .expect("renderer anchors poisoned")
            .entry(participant_id.into())
            .and_modify(|anchor| {
                anchor.first_note_beat = first_note_beat;
                anchor.input_offset_ms = input_offset_ms;
            })
            .or_insert(RenderAnchor {
                first_note_beat,
                input_offset_ms,
                source_time_us: None,
            });
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
    }

    pub fn push_tap(
        &self,
        participant_id: &str,
        sequence: u64,
        beat: f32,
        judgement_beat: f32,
        pressed: bool,
        released: bool,
    ) {
        if (!pressed && !released) || !beat.is_finite() || !judgement_beat.is_finite() {
            return;
        }
        let mut taps = self.render_taps.lock().expect("renderer taps poisoned");
        let buffer = taps.entry(participant_id.into()).or_default();
        if buffer.iter().any(|tap| tap.sequence == sequence) {
            return;
        }
        buffer.push_back(RenderTap {
            sequence,
            beat,
            judgement_beat,
            pressed,
            released,
        });
        while buffer.len() > MAX_RENDER_TAPS {
            buffer.pop_front();
        }
    }

    pub fn push_player_state(&self, participant_id: &str, mut state: GameplayState) {
        if state.beat == 0.0 {
            state.beat = self
                .render_samples
                .lock()
                .ok()
                .and_then(|buffers| {
                    buffers
                        .get(participant_id)
                        .and_then(|buffer| buffer.back())
                        .map(|buffered| buffered.sample.beat as f64)
                })
                .unwrap_or(0.0);
        }
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

    pub fn aligned_featured_state(&self, _now_ms: u64) -> Option<GameplayState> {
        let featured = self.slots().into_iter().find(|slot| slot.featured)?;
        let participant_id = featured.participant_id?;
        let target_beat = self
            .presentation_samples
            .lock()
            .ok()?
            .get(&participant_id)?
            .beat as f64;
        let buffers = self.player_states.lock().ok()?;
        let buffer = buffers.get(&participant_id)?;
        buffer
            .iter()
            .rev()
            .find(|state| state.beat <= target_beat + f64::EPSILON)
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
        let slots = self
            .slots()
            .into_iter()
            .filter(|slot| slot.active)
            .collect::<Vec<_>>();
        if slots.is_empty() {
            return;
        }
        let common_delay_us = slots
            .iter()
            .map(|slot| slot.delay_ms as u64 * 1_000)
            .max()
            .unwrap_or(500_000);
        let mut participants = slots
            .iter()
            .filter_map(|slot| slot.participant_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        participants.sort();

        let buffers = self.render_samples.lock().expect("render buffers poisoned");
        let mut anchors = self
            .render_anchors
            .lock()
            .expect("renderer anchors poisoned");
        let mut anchor_times = HashMap::new();
        for participant in &participants {
            let Some(anchor) = anchors.get_mut(participant) else {
                continue;
            };
            if anchor.source_time_us.is_none() {
                anchor.source_time_us = buffers.get(participant).and_then(|buffer| {
                    buffer
                        .iter()
                        .find(|buffered| {
                            buffered.sample.flags & FLAG_PLAYING != 0
                                && buffered.sample.beat + 0.0001 >= anchor.first_note_beat
                        })
                        .map(|buffered| buffered.sample.run_time_us)
                });
            }
            if let Some(source_time_us) = anchor.source_time_us {
                anchor_times.insert(participant.clone(), source_time_us);
            }
        }

        let mut epoch = self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned");
        if epoch
            .as_ref()
            .is_some_and(|current| current.participants != participants)
        {
            *epoch = None;
        }
        if epoch.is_none() && anchor_times.len() == participants.len() {
            let existing_origin = *self
                .presentation_origin_us
                .lock()
                .expect("renderer presentation origin poisoned");
            *epoch = Some(SyncEpoch {
                participants: participants.clone(),
                release_at_us: existing_origin
                    .unwrap_or_else(|| now_us.saturating_add(common_delay_us)),
                released: existing_origin.is_some(),
            });
        }
        let release_at_us = epoch.as_ref().map(|value| value.release_at_us);
        let released = release_at_us.is_some_and(|release| now_us >= release);
        let first_release = released && epoch.as_ref().is_some_and(|value| !value.released);

        let mut maps = self.input_maps.lock().expect("input maps poisoned");
        let taps = self.render_taps.lock().expect("renderer taps poisoned");
        let mut tap_cursors = self
            .tap_cursors
            .lock()
            .expect("renderer tap cursors poisoned");
        let mut input_sources = self
            .input_sources
            .lock()
            .expect("renderer input sources poisoned");
        let mut input_sequences = self
            .input_sequences
            .lock()
            .expect("renderer input sequences poisoned");
        let mut presentation = self
            .presentation_samples
            .lock()
            .expect("renderer presentation samples poisoned");

        for slot in slots {
            let Some(participant) = slot.participant_id.as_deref() else {
                continue;
            };
            let Some(buffer) = buffers.get(participant) else {
                continue;
            };
            let selected = if released {
                let Some(anchor_time_us) = anchor_times.get(participant).copied() else {
                    continue;
                };
                let elapsed = now_us.saturating_sub(release_at_us.unwrap_or(now_us));
                let target_source_us = anchor_time_us.saturating_add(elapsed);
                let Some(selected) = buffer
                    .iter()
                    .rev()
                    .find(|buffered| buffered.sample.run_time_us <= target_source_us)
                else {
                    continue;
                };
                selected
            } else {
                let target_receipt_us = now_us.saturating_sub(common_delay_us);
                let Some(selected) = buffer
                    .iter()
                    .rev()
                    .find(|buffered| buffered.received_at_us <= target_receipt_us)
                else {
                    continue;
                };
                selected
            };
            let sample = &selected.sample;
            let Some(map) = maps.get_mut(&slot.id) else {
                continue;
            };

            let source_changed =
                input_sources.get(&slot.id).map(String::as_str) != Some(participant);
            if source_changed {
                input_sources.insert(slot.id.clone(), participant.to_owned());
                let baseline = taps
                    .get(participant)
                    .and_then(|events| {
                        events
                            .iter()
                            .rev()
                            .find(|event| event.beat < sample.beat - 0.05)
                    })
                    .map(|event| event.sequence)
                    .unwrap_or(0);
                tap_cursors.insert(slot.id.clone(), baseline);
            }

            let cursor = tap_cursors.get(&slot.id).copied().unwrap_or(0);
            // A first-note sample can become receipt-delay eligible one input
            // tick before the cohort barrier releases. Never consume its
            // reliable edge while capture is still hidden.
            let tap = released
                .then(|| {
                    taps.get(participant).and_then(|events| {
                        events.iter().find(|event| {
                            event.sequence > cursor && event.beat <= sample.beat + 0.001
                        })
                    })
                })
                .flatten();
            if let Some(tap) = tap {
                tap_cursors.insert(slot.id.clone(), tap.sequence);
            }

            let mut mapped_sample = sample.clone();
            if first_release {
                if let Some(anchor) = anchors.get(participant) {
                    mapped_sample.beat = anchor.first_note_beat;
                }
            }
            let mut bytes = mapped_sample.encode();
            let mut flags = sample.flags
                & !(FLAG_PARKED
                    | FLAG_CAPTURE_ENABLED
                    | FLAG_SYNC_RELEASE
                    | FLAG_TAP_PRESSED
                    | FLAG_TAP_RELEASED);
            if released {
                flags |= FLAG_CAPTURE_ENABLED;
            }
            if first_release || (released && source_changed) {
                flags |= FLAG_SYNC_RELEASE;
            }
            let judgement_beat = tap.map(|event| event.judgement_beat).unwrap_or(f32::NAN);
            bytes[12..16].copy_from_slice(&judgement_beat.to_le_bytes());
            let input_offset_ms = anchors
                .get(participant)
                .map(|anchor| anchor.input_offset_ms)
                .unwrap_or(0.0);
            bytes[16..20].copy_from_slice(&input_offset_ms.to_le_bytes());
            if tap.is_some_and(|event| event.pressed) {
                flags |= FLAG_TAP_PRESSED;
            }
            if tap.is_some_and(|event| event.released) {
                flags |= FLAG_TAP_RELEASED;
            }
            bytes[30..32].copy_from_slice(&flags.to_le_bytes());

            let sequence = input_sequences.entry(slot.id.clone()).or_insert(0);
            *sequence = sequence.wrapping_add(1).max(1);
            bytes[8..12].copy_from_slice(&sequence.to_le_bytes());
            map[..8].copy_from_slice(&bytes[..8]);
            map[12..].copy_from_slice(&bytes[12..]);
            std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
            map[8..12].copy_from_slice(&bytes[8..12]);

            if released {
                presentation.insert(participant.to_owned(), mapped_sample);
            }
        }
        if first_release {
            if let Some(epoch) = epoch.as_mut() {
                epoch.released = true;
                *self
                    .presentation_origin_us
                    .lock()
                    .expect("renderer presentation origin poisoned") = Some(epoch.release_at_us);
            }
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
            slot.parked = false;
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
        if let Err(error) = self.reset_input_map(&reset.id) {
            self.set_error(
                &reset.id,
                format!("reset renderer input map failed: {error}"),
            );
        }
        self.input_sources
            .lock()
            .expect("renderer input sources poisoned")
            .remove(&reset.id);
        self.tap_cursors
            .lock()
            .expect("renderer tap cursors poisoned")
            .remove(&reset.id);
        self.presentation_samples
            .lock()
            .expect("renderer presentation samples poisoned")
            .clear();
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
        self.frame_observations
            .lock()
            .expect("renderer observations poisoned")
            .remove(&reset.id);
    }

    /// Detaches a live slot without destroying its child. The hidden renderer
    /// keeps advancing native chart events with capture disabled, allowing a
    /// participant-only rebind during the same race without losing prior VFX.
    pub fn park_slot(&self, slot_id: &str) -> Result<RendererSlot> {
        let id = slot_id.to_ascii_uppercase();
        if !self
            .processes
            .lock()
            .expect("renderer processes poisoned")
            .contains_key(&id)
        {
            bail!("renderer slot has no warm child to park");
        }
        let parked = {
            let mut slots = self.slots.lock().expect("renderer slots poisoned");
            let slot = slots
                .iter_mut()
                .find(|slot| slot.id.eq_ignore_ascii_case(&id))
                .context("unknown renderer slot")?;
            slot.participant_id = None;
            slot.participant_name = None;
            slot.active = false;
            slot.parked = true;
            slot.clone()
        };
        if let Some(map) = self
            .input_maps
            .lock()
            .expect("input maps poisoned")
            .get_mut(&id)
        {
            let mut flags = u16::from_le_bytes(map[30..32].try_into().unwrap_or_default());
            flags &=
                !(FLAG_CAPTURE_ENABLED | FLAG_SYNC_RELEASE | FLAG_TAP_PRESSED | FLAG_TAP_RELEASED);
            flags |= FLAG_PLAYING | FLAG_PARKED;
            map[12..16].copy_from_slice(&f32::NAN.to_le_bytes());
            map[30..32].copy_from_slice(&flags.to_le_bytes());
            let mut sequences = self
                .input_sequences
                .lock()
                .expect("renderer input sequences poisoned");
            let sequence = sequences.entry(id.clone()).or_insert(0);
            *sequence = sequence.wrapping_add(1).max(1);
            std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
            map[8..12].copy_from_slice(&sequence.to_le_bytes());
        }
        self.input_sources
            .lock()
            .expect("renderer input sources poisoned")
            .remove(&id);
        self.tap_cursors
            .lock()
            .expect("renderer tap cursors poisoned")
            .remove(&id);
        self.presentation_samples
            .lock()
            .expect("renderer presentation samples poisoned")
            .clear();
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
        Ok(parked)
    }

    pub fn rebind_parked(
        &self,
        slot_id: &str,
        participant_id: String,
        participant_name: Option<String>,
    ) -> Result<RendererSlot> {
        if participant_id.is_empty() {
            bail!("renderer participant is required");
        }
        let id = slot_id.to_ascii_uppercase();
        if !self
            .processes
            .lock()
            .expect("renderer processes poisoned")
            .contains_key(&id)
        {
            bail!("parked renderer child is no longer running");
        }
        let rebound = {
            let mut slots = self.slots.lock().expect("renderer slots poisoned");
            let slot = slots
                .iter_mut()
                .find(|slot| slot.id.eq_ignore_ascii_case(&id))
                .context("unknown renderer slot")?;
            if !slot.parked {
                bail!("renderer slot is not parked");
            }
            slot.participant_id = Some(participant_id);
            slot.participant_name = participant_name;
            slot.active = true;
            slot.parked = false;
            slot.clone()
        };
        self.input_sources
            .lock()
            .expect("renderer input sources poisoned")
            .remove(&id);
        self.tap_cursors
            .lock()
            .expect("renderer tap cursors poisoned")
            .remove(&id);
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
        Ok(rebound)
    }

    pub fn begin_run(&self) {
        self.render_samples
            .lock()
            .expect("render buffers poisoned")
            .clear();
        self.render_anchors
            .lock()
            .expect("renderer anchors poisoned")
            .clear();
        self.render_taps
            .lock()
            .expect("renderer taps poisoned")
            .clear();
        self.player_states
            .lock()
            .expect("player state buffers poisoned")
            .clear();
        self.input_sources
            .lock()
            .expect("renderer input sources poisoned")
            .clear();
        self.tap_cursors
            .lock()
            .expect("renderer tap cursors poisoned")
            .clear();
        self.presentation_samples
            .lock()
            .expect("renderer presentation samples poisoned")
            .clear();
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
        *self
            .presentation_origin_us
            .lock()
            .expect("renderer presentation origin poisoned") = None;
        let ids = self
            .slots()
            .into_iter()
            .map(|slot| slot.id)
            .collect::<Vec<_>>();
        for id in ids {
            let _ = self.reset_input_map(&id);
        }
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
        self.render_anchors
            .lock()
            .expect("renderer anchors poisoned")
            .remove(participant_id);
        self.render_taps
            .lock()
            .expect("renderer taps poisoned")
            .remove(participant_id);
        self.presentation_samples
            .lock()
            .expect("renderer presentation samples poisoned")
            .remove(participant_id);
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
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
        for slot in slots.iter_mut().filter(|slot| slot.active || slot.parked) {
            if slot.parked {
                if let Some((_, error)) = exited.iter().find(|(id, _)| id == &slot.id) {
                    slot.parked = false;
                    slot.last_error = Some(error.clone());
                } else if !running.get(&slot.id).copied().unwrap_or(false) {
                    slot.parked = false;
                    slot.last_error = Some("parked renderer is no longer running".into());
                }
                slot.healthy = false;
                slot.actual_fps = 0.0;
                continue;
            }
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
        self.render_anchors
            .lock()
            .expect("renderer anchors poisoned")
            .clear();
        self.render_taps
            .lock()
            .expect("renderer taps poisoned")
            .clear();
        self.input_maps.lock().expect("input maps poisoned").clear();
        self.input_sequences
            .lock()
            .expect("renderer input sequences poisoned")
            .clear();
        self.input_sources
            .lock()
            .expect("renderer input sources poisoned")
            .clear();
        self.tap_cursors
            .lock()
            .expect("renderer tap cursors poisoned")
            .clear();
        self.presentation_samples
            .lock()
            .expect("renderer presentation samples poisoned")
            .clear();
        *self
            .sync_epoch
            .lock()
            .expect("renderer sync epoch poisoned") = None;
        *self
            .presentation_origin_us
            .lock()
            .expect("renderer presentation origin poisoned") = None;
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

    fn reset_input_map(&self, slot_id: &str) -> Result<()> {
        let id = slot_id.to_ascii_uppercase();
        if let Some(map) = self
            .input_maps
            .lock()
            .expect("input maps poisoned")
            .get_mut(&id)
        {
            // Reuse the mapping in place. Truncating a still-mapped state file
            // fails on Windows and was the reason STOP followed by ASSIGN could
            // no longer configure that slot.
            map.fill(0);
            map.flush()?;
            return Ok(());
        }
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
            .insert(id, map);
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
        manager.push_sample_at(
            "player-1",
            RenderSample {
                session_id: 0,
                sequence: 1,
                run_time_us: 1_000_000,
                beat: 10.0,
                paddle_angle: 20.0,
                tap_mask: 1,
                flags: 1,
            },
            1_000_000,
        );
        manager.push_sample_at(
            "player-1",
            RenderSample {
                session_id: 0,
                sequence: 2,
                run_time_us: 1_016_667,
                beat: 10.25,
                paddle_angle: 21.0,
                tap_mask: 2,
                flags: 1,
            },
            1_016_667,
        );

        manager.write_aligned_inputs(1_516_667);

        let sample =
            RenderSample::decode(&std::fs::read(manager.state_path("A")).unwrap()).unwrap();
        assert_eq!(sample.sequence, 1);
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
        manager.push_sample_at(
            "player-1",
            RenderSample {
                session_id: 0,
                sequence: 7,
                run_time_us: 1_000_000,
                beat: -8.0,
                paddle_angle: 135.0,
                tap_mask: 0,
                flags: 1,
            },
            1_000_000,
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
        assert_eq!(sample.sequence, 1);
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
                beat: 10.0,
                player_name: "Delayed player".into(),
                ..GameplayState::default()
            }]),
        );

        assert!(manager.aligned_featured_state(1_499).is_none());
        manager.presentation_samples.lock().unwrap().insert(
            "player-1".into(),
            RenderSample {
                session_id: 0,
                sequence: 1,
                run_time_us: 1_000_000,
                beat: 10.0,
                paddle_angle: 0.0,
                tap_mask: 0,
                flags: 1,
            },
        );
        assert_eq!(
            manager.aligned_featured_state(1_500).unwrap().player_name,
            "Delayed player"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn first_note_barrier_removes_cross_source_arrival_jitter() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-cohort-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        for (slot, participant, delay) in [("A", "player-1", 250), ("B", "player-2", 500)] {
            manager
                .configure(
                    slot,
                    RendererRequest {
                        participant_id: Some(participant.into()),
                        participant_name: Some(participant.into()),
                        mode: Some(RendererMode::Full),
                        width: Some(320),
                        height: Some(180),
                        fps: Some(60),
                        delay_ms: Some(delay),
                        featured: Some(slot == "A"),
                    },
                )
                .unwrap();
        }
        manager.push_render_anchor("player-1", 10.0, 80.0);
        manager.push_render_anchor("player-2", 10.0, -45.0);
        for (participant, source_base, receipt_base) in [
            ("player-1", 1_000_000, 10_000_000),
            // Player 2 arrives 120 ms later, but has an unrelated source clock.
            ("player-2", 5_000_000, 10_120_000),
        ] {
            manager.push_sample_at(
                participant,
                RenderSample {
                    session_id: 0,
                    sequence: 1,
                    run_time_us: source_base,
                    beat: 9.75,
                    paddle_angle: 20.0,
                    tap_mask: 0,
                    flags: FLAG_PLAYING,
                },
                receipt_base,
            );
            manager.push_sample_at(
                participant,
                RenderSample {
                    session_id: 0,
                    sequence: 2,
                    run_time_us: source_base + 16_667,
                    beat: 10.0,
                    paddle_angle: 21.0,
                    tap_mask: 0,
                    flags: FLAG_PLAYING,
                },
                receipt_base + 16_667,
            );
        }

        // The slowest configured cache wins for the synchronized cohort.
        manager.write_aligned_inputs(10_136_667);
        assert_eq!(
            manager
                .sync_epoch
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .release_at_us,
            10_636_667
        );
        manager.write_aligned_inputs(10_636_667);
        for slot in ["A", "B"] {
            let sample =
                RenderSample::decode(&std::fs::read(manager.state_path(slot)).unwrap()).unwrap();
            assert_eq!(sample.beat, 10.0);
            assert_ne!(sample.flags & FLAG_CAPTURE_ENABLED, 0);
            assert_ne!(sample.flags & FLAG_SYNC_RELEASE, 0);
        }

        for (participant, source_base, receipt) in [
            ("player-1", 1_000_000, 10_200_000),
            ("player-2", 5_000_000, 10_360_000),
        ] {
            manager.push_sample_at(
                participant,
                RenderSample {
                    session_id: 0,
                    sequence: 3,
                    run_time_us: source_base + 33_334,
                    beat: 10.25,
                    paddle_angle: 22.0,
                    tap_mask: 0,
                    flags: FLAG_PLAYING,
                },
                receipt,
            );
        }
        manager.write_aligned_inputs(10_670_001);
        let a = RenderSample::decode(&std::fs::read(manager.state_path("A")).unwrap()).unwrap();
        let b = RenderSample::decode(&std::fs::read(manager.state_path("B")).unwrap()).unwrap();
        assert_eq!(a.beat, 10.25);
        assert_eq!(a.beat, b.beat);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reliable_tap_edges_are_delivered_once_with_source_judgement() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-taps-{}", rand::random::<u64>()));
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
                    featured: Some(true),
                },
            )
            .unwrap();
        manager.push_render_anchor("player-1", 10.0, 80.0);
        manager.push_sample_at(
            "player-1",
            RenderSample {
                session_id: 0,
                sequence: 1,
                run_time_us: 1_000_000,
                beat: 10.0,
                paddle_angle: 90.0,
                tap_mask: 0,
                flags: FLAG_PLAYING,
            },
            2_000_000,
        );
        // Both edges fall inside one sampled held-key interval.
        manager.push_tap("player-1", 101, 10.0, 9.84, true, false);
        manager.push_tap("player-1", 102, 10.0, 9.86, false, true);
        manager.write_aligned_inputs(2_016_667);
        manager.write_aligned_inputs(2_500_000);
        let hidden = std::fs::read(manager.state_path("A")).unwrap();
        assert_eq!(
            u16::from_le_bytes(hidden[30..32].try_into().unwrap())
                & (FLAG_CAPTURE_ENABLED | FLAG_TAP_PRESSED | FLAG_TAP_RELEASED),
            0,
            "the first raw edge must remain queued until the cohort is visible"
        );

        manager.write_aligned_inputs(2_516_667);
        let first = std::fs::read(manager.state_path("A")).unwrap();
        assert_ne!(
            u16::from_le_bytes(first[30..32].try_into().unwrap()) & FLAG_TAP_PRESSED,
            0
        );
        assert!((f32::from_le_bytes(first[12..16].try_into().unwrap()) - 9.84).abs() < 0.0001);
        assert_eq!(f32::from_le_bytes(first[16..20].try_into().unwrap()), 80.0);

        manager.write_aligned_inputs(2_533_334);
        let second = std::fs::read(manager.state_path("A")).unwrap();
        assert_ne!(
            u16::from_le_bytes(second[30..32].try_into().unwrap()) & FLAG_TAP_RELEASED,
            0
        );
        assert!((f32::from_le_bytes(second[12..16].try_into().unwrap()) - 9.86).abs() < 0.0001);

        manager.write_aligned_inputs(2_550_001);
        let third = std::fs::read(manager.state_path("A")).unwrap();
        assert_eq!(
            u16::from_le_bytes(third[30..32].try_into().unwrap())
                & (FLAG_TAP_PRESSED | FLAG_TAP_RELEASED),
            0
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stopped_input_mapping_can_be_reassigned_without_truncation() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-reassign-{}", rand::random::<u64>()));
        let manager = RendererManager::new(root.clone()).unwrap();
        let request = |participant: &str| RendererRequest {
            participant_id: Some(participant.into()),
            participant_name: Some(participant.into()),
            mode: Some(RendererMode::Full),
            width: Some(320),
            height: Some(180),
            fps: Some(60),
            delay_ms: Some(500),
            featured: Some(true),
        };
        manager.configure("A", request("player-1")).unwrap();
        manager.stop_slot("A");
        let reassigned = manager.configure("A", request("player-2")).unwrap();
        assert!(reassigned.active);
        assert_eq!(reassigned.participant_id.as_deref(), Some("player-2"));
        assert_eq!(
            std::fs::metadata(manager.state_path("A")).unwrap().len(),
            RenderSample::WIRE_SIZE as u64
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn warm_park_and_rebind_preserve_the_live_child() {
        let root =
            std::env::temp_dir().join(format!("bbt-renderer-warm-{}", rand::random::<u64>()));
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
                    featured: Some(true),
                },
            )
            .unwrap();
        let child = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .spawn()
            .unwrap();
        let process_id = child.id();
        manager.processes.lock().unwrap().insert("A".into(), child);

        let parked = manager.park_slot("A").unwrap();
        assert!(parked.parked);
        assert!(!parked.active);
        assert_eq!(
            manager.processes.lock().unwrap()["A"].id(),
            process_id,
            "parking replaced the child and lost its accumulated chart VFX"
        );

        let rebound = manager
            .rebind_parked("A", "player-2".into(), Some("Player 2".into()))
            .unwrap();
        assert!(rebound.active);
        assert!(!rebound.parked);
        assert_eq!(rebound.participant_id.as_deref(), Some("player-2"));
        assert_eq!(manager.processes.lock().unwrap()["A"].id(), process_id);
        manager.stop_all();
        let _ = std::fs::remove_dir_all(root);
    }
}
