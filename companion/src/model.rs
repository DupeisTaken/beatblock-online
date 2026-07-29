use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u8 = 3;
pub const DEFAULT_HOST_PORT: u16 = 32145;
pub const MAX_PLAYERS: usize = 16;
pub const MAX_SPECTATORS: usize = 32;
pub const MAX_RENDER_STREAMS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub version: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub sequence: u64,
    #[serde(default, alias = "timestampMs")]
    pub run_time_us: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Correlates in-game control requests with an explicit acknowledgement.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

impl Envelope {
    pub fn new(kind: impl Into<String>, sequence: u64, payload: Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            kind: kind.into(),
            sequence,
            run_time_us: monotonic_fallback_us(),
            run_id: payload
                .get("runId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            request_id: payload
                .get("requestId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            payload,
        }
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.run_time_us / 1_000
    }
}

fn monotonic_fallback_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameplayState {
    pub state: String,
    pub player_name: String,
    pub song_name: String,
    pub lobby_name: String,
    pub accuracy: f64,
    pub combo: u64,
    pub misses: u64,
    pub rank: u64,
    pub progress: f64,
    pub connected: bool,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub beat: f64,
    #[serde(default)]
    pub paddle_angle: f64,
    #[serde(default)]
    pub tap_mask: u16,
    #[serde(default)]
    pub health: f64,
}

impl Default for GameplayState {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            player_name: "Player".into(),
            song_name: "No chart".into(),
            lobby_name: "Offline practice".into(),
            accuracy: 100.0,
            combo: 0,
            misses: 0,
            rank: 1,
            progress: 0.0,
            connected: false,
            updated_at_ms: 0,
            beat: 0.0,
            paddle_angle: 0.0,
            tap_mask: 0,
            health: -1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerConfig {
    pub display_name: String,
    pub host_address: String,
    pub host_port: u16,
    pub requested_role: ParticipantRole,
    pub remember_password: bool,
    pub update_checks: bool,
    pub spectator_delay_ms: u32,
    pub admission_mode: AdmissionMode,
    pub chart_transfer_mode: ChartTransferMode,
    pub game_directory: Option<String>,
    #[serde(default)]
    pub firewall_installed: bool,
    #[serde(default)]
    pub firewall_public: bool,
    #[serde(default = "default_true")]
    pub hud_enabled: bool,
    #[serde(default = "default_true")]
    pub renderer_desktop_mute: bool,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            display_name: "Player".into(),
            host_address: "127.0.0.1".into(),
            host_port: DEFAULT_HOST_PORT,
            requested_role: ParticipantRole::Player,
            remember_password: false,
            update_checks: false,
            spectator_delay_ms: 500,
            admission_mode: AdmissionMode::HostApproval,
            chart_transfer_mode: ChartTransferMode::VerifyOnly,
            game_directory: None,
            firewall_installed: false,
            firewall_public: false,
            hud_enabled: true,
            renderer_desktop_mute: true,
        }
    }
}

impl ManagerConfig {
    /// Keeps persisted and API-supplied settings within the same bounds used
    /// by the room protocol and native UI.
    pub fn validate(&self) -> anyhow::Result<()> {
        let display_name = self.display_name.trim();
        if display_name.is_empty() || display_name.chars().count() > 48 {
            anyhow::bail!("display name must contain 1-48 characters");
        }
        let host_address = self.host_address.trim();
        if host_address.is_empty() || host_address.chars().count() > 2_048 {
            anyhow::bail!("host address must contain 1-2048 characters");
        }
        if !(250..=1_500).contains(&self.spectator_delay_ms) {
            anyhow::bail!("spectator delay must be between 250 and 1500 milliseconds");
        }
        if self
            .game_directory
            .as_ref()
            .is_some_and(|path| path.contains('\0') || path.chars().count() > 32_767)
        {
            anyhow::bail!("game directory is invalid");
        }
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

pub type CompanionConfig = ManagerConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantRole {
    Player,
    Spectator,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionMode {
    PasswordOnly,
    HostApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartTransferMode {
    VerifyOnly,
    HostTransfer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomLifecycle {
    Forming,
    ChartLocked,
    Ready,
    Countdown,
    Playing,
    Results,
    SetComplete,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunValidity {
    Pending,
    Valid,
    Invalid,
    Dnf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreTotals {
    pub hits: u64,
    pub misses: u64,
    pub barelies: u64,
    pub combo: u64,
    pub max_combo: u64,
    pub current_max_hits: u64,
    pub max_hits: u64,
    pub mine_hits: u64,
}

impl ScoreTotals {
    pub fn accuracy(&self) -> f64 {
        if self.current_max_hits == 0 {
            return 100.0;
        }
        let numerator =
            self.current_max_hits as f64 - self.misses as f64 - self.barelies as f64 / 4.0;
        ((numerator.max(0.0) / self.current_max_hits as f64 * 100.0) * 100.0).floor() / 100.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub session_id: String,
    pub display_name: String,
    pub role: ParticipantRole,
    pub admitted: bool,
    pub connected: bool,
    pub ready: bool,
    pub verified: bool,
    pub progress: f64,
    pub accuracy: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    pub set_total: f64,
    pub totals: ScoreTotals,
    pub validity: RunValidity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
    pub last_sequence: Option<u64>,
    #[serde(default)]
    pub commentator_access: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartLock {
    pub hash: String,
    pub package_name: String,
    pub song_name: String,
    pub variant: String,
    pub expected_max_hits: u64,
    pub official: bool,
    pub transfer_mode: ChartTransferMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetlistEntry {
    pub id: String,
    pub chart: ChartLock,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshot {
    pub id: String,
    pub name: String,
    pub host_session_id: String,
    pub lifecycle: RoomLifecycle,
    pub admission_mode: AdmissionMode,
    #[serde(default = "default_true")]
    pub allow_chart_transfers: bool,
    /// Host-owned convenience policy. It only asks the host for an offer after
    /// local matching fails; package acceptance and executable confirmation
    /// remain participant-owned.
    #[serde(default)]
    pub auto_request_chart_transfers: bool,
    /// Competitive integrity verdicts are opt-out per room. Missing fields
    /// from older protocol-v3 snapshots retain strict behavior.
    #[serde(default = "default_true")]
    pub validity_checks_enabled: bool,
    /// Hosts require the exact Beatblock runtime build by default. This is
    /// independent of the Online protocol version and can be relaxed for
    /// casual or compatibility-testing rooms.
    #[serde(default = "default_true")]
    pub require_same_game_build: bool,
    pub participants: Vec<Participant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<ChartLock>,
    pub setlist: Vec<SetlistEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_setlist_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_start_time_ms: Option<u64>,
    pub force_start: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererMode {
    Full,
    Clean,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererSlot {
    pub id: String,
    pub participant_id: Option<String>,
    pub participant_name: Option<String>,
    pub mode: RendererMode,
    pub participant_appearance: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    #[serde(default)]
    pub actual_fps: f32,
    pub delay_ms: u32,
    pub featured: bool,
    pub active: bool,
    pub frame_sequence: u64,
    pub dropped_frames: u64,
    #[serde(default)]
    pub healthy: bool,
    #[serde(default)]
    pub last_frame_at_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_isolation: Option<AudioIsolationState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioIsolationState {
    pub status: String,
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoplayAudioState {
    pub enabled: bool,
    pub healthy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub featured_slot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_isolation: Option<AudioIsolationState>,
}

/// The authoritative, health-free renderer configuration distributed by the
/// host. Each machine combines this plan with its own local renderer health so
/// a slow Commentator never makes the host's slot look unhealthy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSlotPlan {
    pub id: String,
    pub participant_id: Option<String>,
    pub participant_name: Option<String>,
    pub render_source_id: Option<u32>,
    pub mode: RendererMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub delay_ms: u32,
    pub featured: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastPlan {
    pub revision: u64,
    pub updated_at_ms: u64,
    pub slots: Vec<BroadcastSlotPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoplay_audio_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoplay_clock_slot: Option<String>,
}

impl BroadcastPlan {
    pub fn empty() -> Self {
        Self {
            revision: 0,
            updated_at_ms: 0,
            slots: (0..MAX_RENDER_STREAMS)
                .map(|index| BroadcastSlotPlan {
                    id: ((b'A' + index as u8) as char).to_string(),
                    participant_id: None,
                    participant_name: None,
                    render_source_id: None,
                    mode: RendererMode::Full,
                    width: 1280,
                    height: 720,
                    fps: 60,
                    delay_ms: 500,
                    featured: index == 0,
                    active: false,
                })
                .collect(),
            autoplay_audio_enabled: None,
            autoplay_clock_slot: None,
        }
    }

    pub fn from_slots(
        revision: u64,
        updated_at_ms: u64,
        slots: &[RendererSlot],
        autoplay_audio_enabled: bool,
    ) -> Self {
        let autoplay_clock_slot = autoplay_audio_enabled
            .then(|| slots.iter().find(|slot| slot.active && slot.featured))
            .flatten()
            .map(|slot| slot.id.clone());
        Self {
            revision,
            updated_at_ms,
            slots: slots
                .iter()
                .map(|slot| BroadcastSlotPlan {
                    id: slot.id.clone(),
                    participant_id: slot.participant_id.clone(),
                    participant_name: slot.participant_name.clone(),
                    render_source_id: slot.participant_id.as_deref().map(render_source_id),
                    mode: slot.mode,
                    width: slot.width,
                    height: slot.height,
                    fps: slot.fps,
                    delay_ms: slot.delay_ms,
                    featured: slot.featured,
                    active: slot.active,
                })
                .collect(),
            autoplay_audio_enabled: autoplay_audio_enabled.then_some(true),
            autoplay_clock_slot,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentatorMirrorStatus {
    pub enabled: bool,
    pub healthy_slots: u32,
    pub error: Option<String>,
    pub updated_at_ms: u64,
}

/// A stable, non-zero source id carried by protocol-v3 render datagrams.
/// Collisions are resolved when a plan is built by comparing participant ids;
/// the SHA-derived value avoids exposing UUID text in every 60 Hz packet.
pub fn render_source_id(session_id: &str) -> u32 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(session_id.as_bytes());
    u32::from_le_bytes(digest[..4].try_into().expect("SHA-256 prefix")).max(1)
}

impl RendererSlot {
    pub fn defaults(id: &str, featured: bool) -> Self {
        Self {
            id: id.into(),
            participant_id: None,
            participant_name: None,
            mode: RendererMode::Full,
            participant_appearance: false,
            width: 1280,
            height: 720,
            fps: 60,
            actual_fps: 0.0,
            delay_ms: 500,
            featured,
            active: false,
            frame_sequence: 0,
            dropped_frames: 0,
            healthy: false,
            last_frame_at_ms: None,
            last_error: None,
            audio_isolation: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderSample {
    pub session_id: u32,
    pub sequence: u32,
    pub run_time_us: u64,
    pub beat: f32,
    pub paddle_angle: f32,
    pub tap_mask: u16,
    pub flags: u16,
}

impl RenderSample {
    pub const WIRE_SIZE: usize = 32;

    pub fn encode(&self) -> [u8; Self::WIRE_SIZE] {
        let mut out = [0u8; Self::WIRE_SIZE];
        out[0] = PROTOCOL_VERSION;
        out[4..8].copy_from_slice(&self.session_id.to_le_bytes());
        out[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        out[12..20].copy_from_slice(&self.run_time_us.to_le_bytes());
        out[20..24].copy_from_slice(&self.beat.to_le_bytes());
        out[24..28].copy_from_slice(&self.paddle_angle.to_le_bytes());
        out[28..30].copy_from_slice(&self.tap_mask.to_le_bytes());
        out[30..32].copy_from_slice(&self.flags.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes.len() != Self::WIRE_SIZE || bytes[0] != PROTOCOL_VERSION {
            anyhow::bail!("unsupported render datagram");
        }
        Ok(Self {
            session_id: u32::from_le_bytes(bytes[4..8].try_into()?),
            sequence: u32::from_le_bytes(bytes[8..12].try_into()?),
            run_time_us: u64::from_le_bytes(bytes[12..20].try_into()?),
            beat: f32::from_le_bytes(bytes[20..24].try_into()?),
            paddle_angle: f32::from_le_bytes(bytes[24..28].try_into()?),
            tap_mask: u16::from_le_bytes(bytes[28..30].try_into()?),
            flags: u16::from_le_bytes(bytes[30..32].try_into()?),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartHashRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRoomRequest {
    pub name: String,
    pub password: String,
    pub port: Option<u16>,
    pub admission_mode: Option<AdmissionMode>,
    pub host_participating: Option<bool>,
    pub validity_checks_enabled: Option<bool>,
    pub require_same_game_build: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRoomRequest {
    pub address: String,
    pub password: String,
    pub display_name: String,
    pub role: ParticipantRole,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyRequest {
    pub ready: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub session_id: String,
    pub admit: bool,
    pub role: ParticipantRole,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererRequest {
    pub participant_id: Option<String>,
    pub participant_name: Option<String>,
    pub mode: Option<RendererMode>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub delay_ms: Option<u32>,
    pub featured: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_protocol_fields_are_omitted_instead_of_serialized_as_null() {
        // TypeBox optional fields mean "absent" on the wire. Keeping Rust's
        // representation identical prevents older Lua/TS peers from treating
        // a present null as a real chart, rank, or correlation identifier.
        let envelope = serde_json::to_value(Envelope::new("runtime.ready", 1, json!({})))
            .expect("serialize envelope");
        assert!(envelope.get("runId").is_none());
        assert!(envelope.get("requestId").is_none());

        let participant = Participant {
            session_id: "host".into(),
            display_name: "Host".into(),
            role: ParticipantRole::Host,
            admitted: true,
            connected: true,
            ready: false,
            verified: false,
            progress: 0.0,
            accuracy: 100.0,
            rank: None,
            set_total: 0.0,
            totals: ScoreTotals::default(),
            validity: RunValidity::Pending,
            invalid_reason: None,
            last_sequence: None,
            commentator_access: false,
        };
        let participant_json = serde_json::to_value(&participant).expect("serialize participant");
        assert!(participant_json.get("rank").is_none());
        assert!(participant_json.get("invalidReason").is_none());

        let room = RoomSnapshot {
            id: "room".into(),
            name: "Room".into(),
            host_session_id: "host".into(),
            lifecycle: RoomLifecycle::Forming,
            admission_mode: AdmissionMode::HostApproval,
            allow_chart_transfers: true,
            auto_request_chart_transfers: false,
            validity_checks_enabled: true,
            require_same_game_build: true,
            participants: vec![participant],
            chart: None,
            setlist: vec![],
            current_setlist_index: None,
            scheduled_start_time_ms: None,
            force_start: false,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let room_json = serde_json::to_value(room).expect("serialize room");
        for field in ["chart", "currentSetlistIndex", "scheduledStartTimeMs"] {
            assert!(room_json.get(field).is_none(), "{field} must be omitted");
        }
    }
}
