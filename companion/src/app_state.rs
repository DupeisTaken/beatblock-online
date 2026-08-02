use crate::{
    compatibility::GameBuildIdentity,
    exports::ExportPublisher,
    game_commands,
    journal::JournalPublisher,
    model::{
        render_source_id, AdmissionMode, BroadcastPlan, ChartLock, ChartTransferMode,
        CommentatorMirrorStatus, CompanionConfig, Envelope, GameplayState, ParticipantRole,
        RendererRequest, RoomSnapshot, PROTOCOL_VERSION,
    },
    network::{ChartTransferHeader, NetworkEvent, NetworkHub},
    renderer::{RenderScoreState, RendererManager},
    room::{unix_ms, RoomEngine},
    storage::Storage,
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{broadcast, mpsc, RwLock, Semaphore};

const EVENT_SUBSCRIBER_CAPACITY: usize = 2_048;
const NETWORK_EVENT_CAPACITY: usize = 2_048;
const MAX_PEER_RUN_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_PEER_STATE_PAYLOAD_BYTES: usize = 4 * 1024;
const MAX_PEER_TELEMETRY_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_QUEUED_TRANSFER_BUILDS: usize = 4;

pub struct HostRoomOptions {
    pub room_name: String,
    pub password: String,
    pub port: u16,
    pub admission_mode: AdmissionMode,
    pub host_participating: bool,
    pub validity_checks_enabled: bool,
    pub require_same_game_build: bool,
    pub modifiers: crate::model::RoomModifiers,
}

fn validated_room_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        anyhow::bail!("room name must contain 1-80 characters");
    }
    Ok(name.to_owned())
}

fn transferable_chart_for_participant(
    snapshot: &RoomSnapshot,
    session_id: &str,
    requested_hash: &str,
) -> Result<ChartLock> {
    let participant = snapshot
        .participants
        .iter()
        .find(|participant| participant.session_id == session_id)
        .context("local participant is not in the room roster")?;
    if !participant.admitted || !participant.connected {
        anyhow::bail!("join and receive host admission before requesting a chart");
    }
    if participant.role == ParticipantRole::Spectator {
        anyhow::bail!("spectators do not need the locked gameplay chart");
    }
    if participant.verified {
        anyhow::bail!("the local chart already matches the host lock");
    }
    let chart = snapshot
        .chart
        .as_ref()
        .context("the host has not locked a chart")?;
    if chart.hash != requested_hash {
        anyhow::bail!("the chart transfer request is stale; use the current host lock");
    }
    if !snapshot.allow_chart_transfers
        || chart.official
        || chart.transfer_mode != ChartTransferMode::HostTransfer
    {
        anyhow::bail!("this chart is local-only or the host disabled transfers");
    }
    Ok(chart.clone())
}

/// Builds the first authoritative room image before networking exposes it.
/// Keeping host participation in this constructor path prevents a transient
/// playing-host snapshot when the owner chooses to direct the room.
fn initial_host_room(
    room_name: String,
    host_name: String,
    admission_mode: AdmissionMode,
    host_participating: bool,
    validity_checks_enabled: bool,
    require_same_game_build: bool,
    modifiers: crate::model::RoomModifiers,
) -> Result<RoomEngine> {
    let mut room = RoomEngine::host(room_name, host_name, admission_mode);
    if !host_participating {
        room.set_host_participating(false)?;
    }
    room.set_validity_checks(validity_checks_enabled)?;
    room.set_same_game_build_required(require_same_game_build)?;
    room.set_modifiers(modifiers)?;
    Ok(room)
}

/// Persists settings through a same-directory, durable replacement so a
/// crash cannot leave a truncated config file.
pub(crate) fn write_config_atomically(data_dir: &Path, config: &CompanionConfig) -> Result<()> {
    config.validate()?;
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("config.json");
    let temporary = data_dir.join(format!(".config-{}.tmp", uuid::Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec_pretty(config)?;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .context("create temporary runtime config")?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        crate::exports::replace_file(&temporary, &path).context("activate runtime config")
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerMessageClass {
    Run,
    State,
    Telemetry,
}

/// Classifies every peer-to-host control message before any shared state is
/// touched. Host-authored snapshots and unknown future message kinds are
/// rejected by default instead of accidentally flowing through apply_common.
fn classify_peer_message(message: &Envelope) -> Result<PeerMessageClass> {
    let (class, payload_limit) = match message.kind.as_str() {
        "run.started" | "run.score_delta" | "score.mutation" | "run.invalid" | "run.finished" => {
            (PeerMessageClass::Run, MAX_PEER_RUN_PAYLOAD_BYTES)
        }
        "client.ready"
        | "chart.status"
        | "chart.transfer_request"
        | "chart.transfer_decision"
        | "broadcast.subscribe"
        | "broadcast.mirror_status" => (PeerMessageClass::State, MAX_PEER_STATE_PAYLOAD_BYTES),
        "client.hello" | "input.tap" | "render.anchor" | "render.keyframe" => (
            PeerMessageClass::Telemetry,
            MAX_PEER_TELEMETRY_PAYLOAD_BYTES,
        ),
        _ => anyhow::bail!("peer message kind {} is not allowed", message.kind),
    };
    if serde_json::to_vec(&message.payload)?.len() > payload_limit {
        anyhow::bail!("peer {} payload exceeds its safety limit", message.kind);
    }
    if message.kind == "run.invalid"
        && message
            .payload
            .get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| reason.chars().count() > 512)
    {
        anyhow::bail!("run invalidation reason exceeds its safety limit");
    }
    Ok(class)
}

/// Host timestamps are meaningful only in the host's wall-clock domain. Keep
/// the authoritative remaining countdown, but express the target in the local
/// runtime's clock before forwarding it to the local game.
fn localize_host_schedule(message: &mut Envelope, local_now_ms: u64) {
    let start_field = match message.kind.as_str() {
        "room.start_scheduled" | "lobby.start_scheduled" => "serverStartTimeMs",
        "room.snapshot" | "lobby.snapshot" => "scheduledStartTimeMs",
        _ => return,
    };
    let Some(server_time_ms) = message.payload.get("serverTimeMs").and_then(Value::as_u64) else {
        return;
    };
    let Some(server_start_ms) = message.payload.get(start_field).and_then(Value::as_u64) else {
        return;
    };
    let remaining_ms = server_start_ms.saturating_sub(server_time_ms);
    if let Some(payload) = message.payload.as_object_mut() {
        payload.insert(
            start_field.into(),
            json!(local_now_ms.saturating_add(remaining_ms)),
        );
    }
}

#[derive(Clone)]
struct ReconnectRequest {
    address: SocketAddr,
    password: String,
    display_name: String,
    role: ParticipantRole,
    game_build: Option<GameBuildIdentity>,
}

impl ReconnectRequest {
    async fn join(&self, network: &NetworkHub) -> Result<String> {
        network
            .join_with_game_build(
                self.address,
                &self.password,
                &self.display_name,
                self.role,
                self.game_build.clone(),
            )
            .await
    }
}

struct TemporaryFileCleanup {
    path: PathBuf,
    armed: bool,
}

impl TemporaryFileCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }
}

impl Drop for TemporaryFileCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bbt-app-{label}-{}", rand::random::<u64>()))
    }

    fn chart_lock(digit: char, name: &str) -> ChartLock {
        ChartLock {
            hash: digit.to_string().repeat(64),
            package_name: name.into(),
            song_name: name.into(),
            variant: "Hard".into(),
            expected_max_hits: 100,
            official: false,
            transfer_mode: ChartTransferMode::VerifyOnly,
        }
    }

    fn active_renderer_request(participant_id: String) -> RendererRequest {
        RendererRequest {
            participant_id: Some(participant_id),
            participant_name: Some("Host".into()),
            mode: Some(crate::model::RendererMode::Full),
            width: Some(320),
            height: Some(180),
            fps: Some(60),
            delay_ms: Some(500),
            featured: Some(true),
        }
    }

    #[tokio::test]
    async fn local_hello_uses_the_displayed_upstream_build_hash() {
        let root = temporary("game-attestation");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Beatblock.exe"), b"fixture").unwrap();
        let config = CompanionConfig {
            game_directory: Some(game.to_string_lossy().into_owned()),
            ..CompanionConfig::default()
        };
        let (state, _) = AppState::new(root.clone(), "token".into(), config).unwrap();
        state
            .ingest(Envelope::new(
                "client.hello",
                1,
                json!({
                    "instanceId":"fixture-game",
                    "clientVersion":env!("CARGO_PKG_VERSION"),
                    "gameVersion":"1.7.1a (Early Access)[D40B7083]",
                    "distribution":"standalone",
                    "mods":[]
                }),
            ))
            .await
            .unwrap();

        let client = state.client.read().await;
        assert_eq!(
            client.get("gameBuildId").and_then(Value::as_str),
            Some("d40b7083")
        );
        assert_eq!(
            client.get("gameBuildSource").and_then(Value::as_str),
            Some("displayed_build_hash")
        );
        drop(client);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_replacement_is_atomic_and_validated() {
        let root = temporary("config");
        let mut config = CompanionConfig {
            display_name: "Durable Player".into(),
            ..CompanionConfig::default()
        };
        write_config_atomically(&root, &config).unwrap();
        let persisted: CompanionConfig =
            serde_json::from_slice(&std::fs::read(root.join("config.json")).unwrap()).unwrap();
        assert_eq!(persisted.display_name, "Durable Player");
        assert!(std::fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".config-")));

        config.display_name.clear();
        assert!(write_config_atomically(&root, &config).is_err());
        let unchanged: CompanionConfig =
            serde_json::from_slice(&std::fs::read(root.join("config.json")).unwrap()).unwrap();
        assert_eq!(unchanged.display_name, "Durable Player");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn room_names_follow_protocol_bounds() {
        assert_eq!(validated_room_name("  Finals  ").unwrap(), "Finals");
        assert!(validated_room_name("").is_err());
        assert!(validated_room_name(&"界".repeat(81)).is_err());
    }

    #[tokio::test]
    async fn replacing_the_same_chart_after_results_relaunches_active_renderers() {
        let root = temporary("same-chart-relaunch");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let room = RoomEngine::host(
            "Same chart".into(),
            "Host".into(),
            AdmissionMode::PasswordOnly,
        );
        let host = room.snapshot.host_session_id.clone();
        *state.room.write().await = room;
        *state.local_session_id.write().await = Some(host.clone());
        state.is_host.store(true, Ordering::Release);

        let chart = chart_lock('a', "Repeat");
        *state.selected_chart_path.write().await = Some("Custom Levels/Repeat/".into());
        state.lock_chart(chart.clone(), false).await.unwrap();
        state
            .renderer
            .configure("A", active_renderer_request(host))
            .unwrap();
        state
            .renderer
            .set_error("A", "sentinel: renderer was not relaunched");
        state.room.write().await.snapshot.lifecycle = crate::model::RoomLifecycle::Results;

        state.lock_chart(chart, false).await.unwrap();

        assert_eq!(
            state.room.read().await.snapshot.lifecycle,
            crate::model::RoomLifecycle::ChartLocked
        );
        assert!(state
            .renderer
            .slot("A")
            .unwrap()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("Beatblock installation path is unavailable")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn appending_a_chart_does_not_relaunch_the_unchanged_active_renderer() {
        let root = temporary("setlist-append-no-relaunch");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let room = RoomEngine::host("Setlist".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        *state.room.write().await = room;
        *state.local_session_id.write().await = Some(host.clone());
        state.is_host.store(true, Ordering::Release);

        let first = chart_lock('a', "First");
        *state.selected_chart_path.write().await = Some("Custom Levels/First/".into());
        state.lock_chart(first.clone(), true).await.unwrap();
        state
            .renderer
            .configure("A", active_renderer_request(host))
            .unwrap();
        state.renderer.set_error("A", "sentinel: keep active child");
        state.room.write().await.snapshot.lifecycle = crate::model::RoomLifecycle::Results;
        *state.selected_chart_path.write().await = Some("Custom Levels/Second/".into());

        state
            .lock_chart(chart_lock('b', "Second"), true)
            .await
            .unwrap();

        let snapshot = state.room.read().await.snapshot.clone();
        assert_eq!(snapshot.chart.as_ref().unwrap().hash, first.hash);
        assert_eq!(snapshot.setlist.len(), 2);
        assert_eq!(
            state.renderer.slot("A").unwrap().last_error.as_deref(),
            Some("sentinel: keep active child")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn director_host_role_is_set_in_the_initial_room_image() {
        let room = initial_host_room(
            "Directed finals".into(),
            "Operator".into(),
            AdmissionMode::HostApproval,
            false,
            false,
            true,
            crate::model::RoomModifiers::default(),
        )
        .unwrap();
        let host = room
            .snapshot
            .participants
            .iter()
            .find(|participant| participant.session_id == room.snapshot.host_session_id)
            .unwrap();
        assert_eq!(host.role, ParticipantRole::Spectator);
        assert!(host.ready);
        assert!(host.verified);
        assert!(!room.snapshot.validity_checks_enabled);

        let playing = initial_host_room(
            "Playing finals".into(),
            "Operator".into(),
            AdmissionMode::HostApproval,
            true,
            true,
            false,
            crate::model::RoomModifiers::default(),
        )
        .unwrap();
        assert_eq!(playing.snapshot.participants[0].role, ParticipantRole::Host);
        assert!(!playing.snapshot.participants[0].ready);
        assert!(playing.snapshot.validity_checks_enabled);
        assert!(!playing.snapshot.require_same_game_build);
    }

    #[test]
    fn transfer_requests_require_an_admitted_mismatched_player_and_exact_lock() {
        let mut room = RoomEngine::host(
            "Transfer".into(),
            "Host".into(),
            AdmissionMode::HostApproval,
        );
        room.lock_chart(
            ChartLock {
                hash: "a".repeat(64),
                package_name: "Chart".into(),
                song_name: "Signal".into(),
                variant: "Hard".into(),
                expected_max_hits: 100,
                official: false,
                transfer_mode: ChartTransferMode::HostTransfer,
            },
            false,
        )
        .unwrap();
        let peer = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();

        assert!(
            transferable_chart_for_participant(&room.snapshot, &peer, &"a".repeat(64)).is_err()
        );
        room.admit(&peer, true, ParticipantRole::Player).unwrap();
        assert_eq!(
            transferable_chart_for_participant(&room.snapshot, &peer, &"a".repeat(64))
                .unwrap()
                .hash,
            "a".repeat(64)
        );
        assert!(
            transferable_chart_for_participant(&room.snapshot, &peer, &"b".repeat(64)).is_err()
        );

        room.set_verified(&peer, true, None).unwrap();
        assert!(
            transferable_chart_for_participant(&room.snapshot, &peer, &"a".repeat(64)).is_err()
        );
        room.set_verified(&peer, false, Some("mismatch".into()))
            .unwrap();
        room.set_role(&peer, ParticipantRole::Spectator).unwrap();
        assert!(
            transferable_chart_for_participant(&room.snapshot, &peer, &"a".repeat(64)).is_err()
        );
    }

    #[tokio::test]
    async fn strict_room_reconnect_preserves_game_build_identity() {
        let build =
            GameBuildIdentity::from_displayed_version("1.7.1a (Early Access)[d40b7083]").unwrap();
        let (host_events, _host_receiver) = mpsc::channel(8);
        let host = NetworkHub::new(host_events);
        let address = host
            .start_host_with_game_build(0, "correct horse".into(), Some(build.clone()), true)
            .await
            .unwrap();
        let (client_events, _client_receiver) = mpsc::channel(8);
        let client = NetworkHub::new(client_events);
        let request = ReconnectRequest {
            address,
            password: "correct horse".into(),
            display_name: "Reconnecting player".into(),
            role: ParticipantRole::Player,
            game_build: Some(build),
        };

        request.join(&client).await.unwrap();
        client.shutdown().await;
        host.shutdown().await;
    }

    #[tokio::test]
    async fn pending_peer_has_no_gameplay_or_renderer_authority() {
        let root = temporary("admission");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let mut room = RoomEngine::host(
            "Approval".into(),
            "Host".into(),
            AdmissionMode::HostApproval,
        );
        let pending = room
            .request_join("Pending", ParticipantRole::Player)
            .unwrap();
        *state.room.write().await = room;
        assert!(state.require_admitted_peer(&pending).await.is_err());
        state
            .room
            .write()
            .await
            .admit(&pending, true, ParticipantRole::Player)
            .unwrap();
        assert!(state.require_admitted_peer(&pending).await.is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn host_rejects_ineligible_network_chart_requests_before_packaging() {
        let root = temporary("transfer-request-authority");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let mut room = RoomEngine::host(
            "Transfer".into(),
            "Host".into(),
            AdmissionMode::HostApproval,
        );
        room.lock_chart(
            ChartLock {
                hash: "a".repeat(64),
                package_name: "Chart".into(),
                song_name: "Signal".into(),
                variant: "Hard".into(),
                expected_max_hits: 100,
                official: false,
                transfer_mode: ChartTransferMode::HostTransfer,
            },
            false,
        )
        .unwrap();
        let peer = room
            .request_join("Player", ParticipantRole::Player)
            .unwrap();
        *state.room.write().await = room;
        state.is_host.store(true, Ordering::Release);

        let request = || NetworkEvent::Envelope {
            session_id: peer.clone(),
            envelope: Envelope::new(
                "chart.transfer_request",
                0,
                json!({"chartHash":"a".repeat(64)}),
            ),
        };
        assert!(state.handle_network_event(request()).await.is_err());
        assert!(state.active_transfer_builds.read().await.is_empty());

        state
            .room
            .write()
            .await
            .admit(&peer, true, ParticipantRole::Spectator)
            .unwrap();
        let spectator_error = state
            .handle_network_event(request())
            .await
            .unwrap_err()
            .to_string();
        assert!(spectator_error.contains("spectators"));
        assert!(state.active_transfer_builds.read().await.is_empty());

        {
            let mut room = state.room.write().await;
            room.set_role(&peer, ParticipantRole::Player).unwrap();
            room.set_verified(&peer, true, None).unwrap();
        }
        let verified_error = state
            .handle_network_event(request())
            .await
            .unwrap_err()
            .to_string();
        assert!(verified_error.contains("already matches"));
        assert!(state.active_transfer_builds.read().await.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejected_score_marks_invalid_without_failing_local_ipc_ingest() {
        let root = temporary("nonfatal-invalid-score");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let mut room =
            RoomEngine::host("Strict".into(), "Host".into(), AdmissionMode::PasswordOnly);
        let host = room.snapshot.host_session_id.clone();
        room.lock_chart(
            ChartLock {
                hash: "a".repeat(64),
                package_name: "Chart".into(),
                song_name: "Signal".into(),
                variant: "Hard".into(),
                expected_max_hits: 100,
                official: false,
                transfer_mode: ChartTransferMode::VerifyOnly,
            },
            false,
        )
        .unwrap();
        room.set_verified(&host, true, None).unwrap();
        room.set_ready(&host, true).unwrap();
        room.schedule_start(false, 2_000).unwrap();
        *state.local_session_id.write().await = Some(host.clone());
        *state.room.write().await = room;
        state.is_host.store(true, Ordering::Release);

        state
            .ingest(Envelope::new(
                "run.started",
                1,
                json!({"runId":"strict-run","maxHits":100}),
            ))
            .await
            .unwrap();
        state
            .ingest(Envelope::new(
                "run.score_delta",
                2,
                json!({
                    "runId":"strict-run",
                    "runSequence":0,
                    "totals":{"hits":101,"misses":0,"barelies":0,"combo":101,"maxCombo":101,"currentMaxHits":100,"maxHits":100,"mineHits":0}
                }),
            ))
            .await
            .unwrap();

        let room = state.room.read().await;
        let participant = room.player(&host).unwrap();
        assert_eq!(participant.validity, crate::model::RunValidity::Invalid);
        assert!(participant
            .invalid_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Rejected run.score_delta")));
        drop(room);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn repeated_score_dirtiness_publishes_one_room_snapshot_per_tick() {
        let root = temporary("room-coalesce");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        *state.room.write().await = RoomEngine::host(
            "Coalesced".into(),
            "Host".into(),
            AdmissionMode::PasswordOnly,
        );
        let mut events = state.events.subscribe();
        state.mark_room_dirty();
        state.mark_room_dirty();
        state.mark_room_dirty();
        assert!(state.flush_room_updates().await.unwrap());
        assert!(!state.flush_room_updates().await.unwrap());
        assert_eq!(events.recv().await.unwrap().kind, "room.snapshot");
        assert!(events.try_recv().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persisted_snapshot_never_reappears_as_a_live_ghost_room() {
        let root = temporary("recovery");
        std::fs::create_dir_all(&root).unwrap();
        {
            let storage = Storage::open(root.join("runtime.sqlite3")).unwrap();
            let room = RoomEngine::host(
                "Recovered".into(),
                "Host".into(),
                AdmissionMode::PasswordOnly,
            );
            storage.save_room(&room.snapshot, 120_000).unwrap();
        }
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        assert_eq!(state.room.blocking_read().snapshot.id, "offline");
        assert!(!state.is_host.load(Ordering::Relaxed));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remote_countdown_is_converted_into_the_local_clock_domain() {
        let mut scheduled = Envelope::new(
            "room.start_scheduled",
            0,
            json!({
                "serverStartTimeMs":1_005_000_u64,
                "serverTimeMs":1_000_000_u64
            }),
        );
        localize_host_schedule(&mut scheduled, 50_000);
        assert_eq!(
            scheduled
                .payload
                .get("serverStartTimeMs")
                .and_then(Value::as_u64),
            Some(55_000)
        );

        let mut snapshot = Envelope::new(
            "room.snapshot",
            0,
            json!({
                "scheduledStartTimeMs":2_003_000_u64,
                "serverTimeMs":2_000_000_u64
            }),
        );
        localize_host_schedule(&mut snapshot, 90_000);
        assert_eq!(
            snapshot
                .payload
                .get("scheduledStartTimeMs")
                .and_then(Value::as_u64),
            Some(93_000)
        );
    }

    #[tokio::test]
    async fn peer_cannot_inject_host_state_or_publish_noop_telemetry() {
        let root = temporary("peer-allowlist");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let mut room = RoomEngine::host(
            "Approval".into(),
            "Host".into(),
            AdmissionMode::HostApproval,
        );
        let peer = room.request_join("Peer", ParticipantRole::Player).unwrap();
        room.admit(&peer, true, ParticipantRole::Player).unwrap();
        *state.room.write().await = room;
        state.is_host.store(true, Ordering::Release);
        let original_client = state.client.read().await.clone();
        let mut events = state.events.subscribe();

        state
            .handle_network_event(NetworkEvent::Envelope {
                session_id: peer.clone(),
                envelope: Envelope::new(
                    "client.hello",
                    1,
                    json!({"clientVersion":"forged","mods":["unbounded"]}),
                ),
            })
            .await
            .unwrap();
        assert_eq!(*state.client.read().await, original_client);
        assert!(events.try_recv().is_err());

        let room_id = state.room.read().await.snapshot.id.clone();
        let injected = state
            .handle_network_event(NetworkEvent::Envelope {
                session_id: peer,
                envelope: Envelope::new("room.snapshot", 2, json!({"id":"attacker-owned"})),
            })
            .await;
        assert!(injected.is_err());
        assert_eq!(state.room.read().await.snapshot.id, room_id);
        assert!(events.try_recv().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn peer_payload_limits_are_applied_before_dispatch() {
        let oversized = Envelope::new(
            "run.score_delta",
            1,
            json!({"padding":"x".repeat(MAX_PEER_RUN_PAYLOAD_BYTES + 1)}),
        );
        assert!(classify_peer_message(&oversized).is_err());
        assert!(classify_peer_message(&Envelope::new("gameplay.snapshot", 1, json!({}),)).is_err());
        assert_eq!(
            classify_peer_message(&Envelope::new(
                "render.anchor",
                2,
                json!({"firstNoteBeat":0}),
            ))
            .unwrap(),
            PeerMessageClass::Telemetry
        );
        assert_eq!(
            classify_peer_message(&Envelope::new(
                "input.tap",
                3,
                json!({"beat":1,"pressed":true}),
            ))
            .unwrap(),
            PeerMessageClass::Telemetry
        );
        assert_eq!(
            classify_peer_message(&Envelope::new(
                "render.keyframe",
                4,
                json!({"accuracy":99.25,"totals":{"maxHits":100}}),
            ))
            .unwrap(),
            PeerMessageClass::Telemetry
        );
        assert!(classify_peer_message(&Envelope::new(
            "render.tap",
            5,
            json!({"participantId":"forged"}),
        ))
        .is_err());
    }

    #[tokio::test]
    async fn executable_transfer_requires_exact_pending_offer_confirmation() {
        let root = temporary("transfer-consent");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let header = ChartTransferHeader {
            request_id: "request-1".into(),
            name: "chart.zip".into(),
            size: 12,
            archive_sha256: "a".repeat(64),
            chart_hash: "b".repeat(64),
            contains_executable_content: true,
        };
        *state.pending_inbound_transfer_offer.write().await =
            Some(("host-session".into(), header.clone()));

        assert!(state
            .decide_chart_transfer("wrong-request", true, false, true)
            .await
            .is_err());
        assert!(state.pending_inbound_transfer_offer.read().await.is_some());
        assert!(state
            .decide_chart_transfer("request-1", true, false, false)
            .await
            .is_err());
        assert!(state.pending_inbound_transfer_offer.read().await.is_some());
        state
            .decide_chart_transfer("request-1", true, false, true)
            .await
            .unwrap();
        assert!(state.pending_inbound_transfer_offer.read().await.is_none());
        assert!(state
            .network
            .authorize_incoming_chart_transfer("host-session", header, true)
            .await
            .is_err());
        state.network.clear_incoming_chart_transfers().await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn trusted_non_executable_offer_is_not_published_as_a_manual_prompt() {
        let root = temporary("trusted-transfer");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let room_id = state.room.read().await.snapshot.id.clone();
        state.trusted_transfer_rooms.write().await.insert(room_id);
        let header = ChartTransferHeader {
            request_id: "trusted-request".into(),
            name: "chart.zip".into(),
            size: 12,
            archive_sha256: "a".repeat(64),
            chart_hash: "b".repeat(64),
            contains_executable_content: false,
        };
        let mut events = state.events.subscribe();

        state
            .apply_host_message(
                "host-session",
                Envelope::new("chart.transfer_offer", 1, json!(header)),
            )
            .await
            .unwrap();

        assert!(state.pending_inbound_transfer_offer.read().await.is_none());
        assert!(events.try_recv().is_err());
        state.network.clear_incoming_chart_transfers().await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn peer_transfer_decision_is_exact_and_cleans_rejected_archive() {
        let root = temporary("transfer-decision");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let mut room = RoomEngine::host(
            "Approval".into(),
            "Host".into(),
            AdmissionMode::HostApproval,
        );
        let peer = room.request_join("Peer", ParticipantRole::Player).unwrap();
        room.admit(&peer, true, ParticipantRole::Player).unwrap();
        *state.room.write().await = room;
        state.is_host.store(true, Ordering::Release);
        let archive = root.join("outgoing.zip");
        std::fs::write(&archive, b"archive").unwrap();
        let header = ChartTransferHeader {
            request_id: "request-1".into(),
            name: "chart.zip".into(),
            size: 7,
            archive_sha256: "a".repeat(64),
            chart_hash: "b".repeat(64),
            contains_executable_content: false,
        };
        state
            .pending_transfer_offers
            .write()
            .await
            .insert(peer.clone(), (archive.clone(), header));

        let mismatched = state
            .handle_network_event(NetworkEvent::Envelope {
                session_id: peer.clone(),
                envelope: Envelope::new(
                    "chart.transfer_decision",
                    1,
                    json!({"requestId":"wrong","accept":false}),
                ),
            })
            .await;
        assert!(mismatched.is_err());
        assert!(archive.exists());
        assert!(state
            .pending_transfer_offers
            .read()
            .await
            .contains_key(&peer));

        state
            .handle_network_event(NetworkEvent::Envelope {
                session_id: peer.clone(),
                envelope: Envelope::new(
                    "chart.transfer_decision",
                    2,
                    json!({"requestId":"request-1","accept":false}),
                ),
            })
            .await
            .unwrap();
        assert!(!archive.exists());
        assert!(!state
            .pending_transfer_offers
            .read()
            .await
            .contains_key(&peer));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn username_conflict_clears_rejected_player_state_and_surfaces_error() {
        let root = temporary("username-conflict");
        let (state, _) =
            AppState::new(root.clone(), "token".into(), CompanionConfig::default()).unwrap();
        let mut room = RoomEngine::host(
            "Conflict".into(),
            "Host".into(),
            AdmissionMode::PasswordOnly,
        );
        let local_session = room
            .request_join("Duplicate", ParticipantRole::Player)
            .unwrap();
        *state.room.write().await = room;
        *state.local_session_id.write().await = Some(local_session);
        *state.connection_status.write().await = "connected".into();
        let mut events = state.events.subscribe();

        state
            .handle_network_event(NetworkEvent::Disconnected {
                session_id: "host-session".into(),
                reason: "username taken".into(),
            })
            .await
            .unwrap();

        assert!(state.local_session_id.read().await.is_none());
        assert_eq!(state.connection_status.read().await.as_str(), "offline");
        assert_eq!(state.room.read().await.snapshot.id, "offline");
        let mut surfaced = false;
        while let Ok(event) = events.try_recv() {
            if event.kind == "runtime.error"
                && event.payload.get("message").and_then(Value::as_str) == Some("username taken")
            {
                surfaced = true;
            }
        }
        assert!(
            surfaced,
            "the joining player did not receive the username conflict"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub local_token: Arc<std::sync::RwLock<String>>,
    pub gameplay: Arc<RwLock<GameplayState>>,
    pub room: Arc<RwLock<RoomEngine>>,
    pub lobby: Arc<RwLock<Value>>,
    pub config: Arc<RwLock<CompanionConfig>>,
    pub client: Arc<RwLock<Value>>,
    pub events: broadcast::Sender<Envelope>,
    pub network: Arc<NetworkHub>,
    pub renderer: Arc<RendererManager>,
    pub exports: ExportPublisher,
    pub storage: Arc<Storage>,
    pub journals: JournalPublisher,
    pub data_dir: Arc<PathBuf>,
    pub is_host: Arc<AtomicBool>,
    pub local_session_id: Arc<RwLock<Option<String>>>,
    pub connection_status: Arc<RwLock<String>>,
    pub host_join_address: Arc<RwLock<Option<SocketAddr>>>,
    pub nat_method: Arc<RwLock<Option<String>>>,
    mapped_host_port: Arc<RwLock<Option<u16>>>,
    pub shutdown_requested: Arc<AtomicBool>,
    room_dirty: Arc<AtomicBool>,
    pub selected_chart_path: Arc<RwLock<Option<String>>>,
    pub chart_paths: Arc<RwLock<HashMap<String, String>>>,
    pub broadcast_plan: Arc<RwLock<BroadcastPlan>>,
    broadcast_revision: Arc<AtomicU64>,
    commentator_subscribers: Arc<RwLock<HashSet<String>>>,
    commentator_statuses: Arc<RwLock<HashMap<String, CommentatorMirrorStatus>>>,
    local_mirror_enabled: Arc<AtomicBool>,
    pending_transfer_offers: Arc<RwLock<HashMap<String, (PathBuf, ChartTransferHeader)>>>,
    pending_inbound_transfer_offer: Arc<RwLock<Option<(String, ChartTransferHeader)>>>,
    trusted_transfer_rooms: Arc<RwLock<HashSet<String>>>,
    active_transfer_builds: Arc<RwLock<HashSet<String>>>,
    transfer_build_slots: Arc<Semaphore>,
    transfer_install_slot: Arc<Semaphore>,
    last_auto_match_hash: Arc<RwLock<Option<String>>>,
    last_auto_request_hash: Arc<RwLock<Option<String>>>,
    ipc_client_id: Arc<RwLock<Option<String>>>,
    control_in_flight: Arc<AtomicBool>,
    reconnect_request: Arc<RwLock<Option<ReconnectRequest>>>,
    nat_renewal_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    reconnect_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl AppState {
    pub fn new(
        data_dir: PathBuf,
        local_token: String,
        config: CompanionConfig,
    ) -> Result<(Self, mpsc::Receiver<NetworkEvent>)> {
        std::fs::create_dir_all(data_dir.join("exports"))?;
        let runtime_db = data_dir.join("runtime.sqlite3");
        let legacy_db = data_dir.join("manager.sqlite3");
        if !runtime_db.exists() && legacy_db.exists() {
            std::fs::copy(&legacy_db, &runtime_db)
                .context("migrate Manager history to runtime storage")?;
        }
        let storage = Arc::new(Storage::open(runtime_db)?);
        // Match summaries are user history, but raw replay telemetry follows
        // the documented 30-day retention ceiling on every runtime start.
        storage.prune_raw_events(unix_ms().saturating_sub(30 * 86_400_000))?;
        // A serialized room is useful for history, but it is not a live room:
        // QUIC credentials and ownership are intentionally never persisted.
        // Surfacing it as active after a crash created an unusable ghost room.
        let _recovered_for_history = storage.recover_room(unix_ms())?;
        let room = RoomEngine::offline();
        let lobby = serde_json::to_value(&room.snapshot)?;
        let (events, _) = broadcast::channel(EVENT_SUBSCRIBER_CAPACITY);
        let (network_events_tx, network_events_rx) = mpsc::channel(NETWORK_EVENT_CAPACITY);
        let network = Arc::new(NetworkHub::new(network_events_tx));
        let renderer = Arc::new(RendererManager::new(data_dir.clone())?);
        renderer.set_desktop_mute_enabled(config.renderer_desktop_mute);
        let exports = ExportPublisher::new(data_dir.join("exports"))?;
        let journals = JournalPublisher::new(data_dir.join("journals"))?;
        Ok((
            Self {
                local_token: Arc::new(std::sync::RwLock::new(local_token)),
                gameplay: Arc::new(RwLock::new(GameplayState::default())),
                room: Arc::new(RwLock::new(room)),
                lobby: Arc::new(RwLock::new(lobby)),
                config: Arc::new(RwLock::new(config)),
                client: Arc::new(RwLock::new(json!({
                    "clientVersion":env!("CARGO_PKG_VERSION"),
                    "gameVersion":Value::Null,
                    "gameBuildId":Value::Null,
                    "gameBuildSource":Value::Null,
                    "distribution":"standalone",
                    "mods":[]
                }))),
                events,
                network,
                renderer,
                exports,
                storage,
                journals,
                data_dir: Arc::new(data_dir),
                is_host: Arc::new(AtomicBool::new(false)),
                local_session_id: Arc::new(RwLock::new(None)),
                connection_status: Arc::new(RwLock::new("offline".into())),
                host_join_address: Arc::new(RwLock::new(None)),
                nat_method: Arc::new(RwLock::new(None)),
                mapped_host_port: Arc::new(RwLock::new(None)),
                shutdown_requested: Arc::new(AtomicBool::new(false)),
                room_dirty: Arc::new(AtomicBool::new(false)),
                selected_chart_path: Arc::new(RwLock::new(None)),
                chart_paths: Arc::new(RwLock::new(HashMap::new())),
                broadcast_plan: Arc::new(RwLock::new(BroadcastPlan::empty())),
                broadcast_revision: Arc::new(AtomicU64::new(0)),
                commentator_subscribers: Arc::new(RwLock::new(HashSet::new())),
                commentator_statuses: Arc::new(RwLock::new(HashMap::new())),
                local_mirror_enabled: Arc::new(AtomicBool::new(false)),
                pending_transfer_offers: Arc::new(RwLock::new(HashMap::new())),
                pending_inbound_transfer_offer: Arc::new(RwLock::new(None)),
                trusted_transfer_rooms: Arc::new(RwLock::new(HashSet::new())),
                active_transfer_builds: Arc::new(RwLock::new(HashSet::new())),
                transfer_build_slots: Arc::new(Semaphore::new(1)),
                transfer_install_slot: Arc::new(Semaphore::new(1)),
                last_auto_match_hash: Arc::new(RwLock::new(None)),
                last_auto_request_hash: Arc::new(RwLock::new(None)),
                ipc_client_id: Arc::new(RwLock::new(None)),
                control_in_flight: Arc::new(AtomicBool::new(false)),
                reconnect_request: Arc::new(RwLock::new(None)),
                nat_renewal_task: Arc::new(Mutex::new(None)),
                reconnect_task: Arc::new(Mutex::new(None)),
            },
            network_events_rx,
        ))
    }

    pub async fn run_network_events(&self, mut receiver: mpsc::Receiver<NetworkEvent>) {
        while let Some(event) = receiver.recv().await {
            if let Err(error) = self.handle_network_event(event).await {
                tracing::warn!(%error, "network event rejected");
            }
        }
    }

    /// A runtime is owned by the first Beatblock process that identifies
    /// itself. Reconnects from that process retain access; another game copy is
    /// rejected instead of receiving the owner's room state and control replies.
    pub async fn claim_ipc_client(&self, client_id: &str) -> bool {
        if client_id.is_empty() || client_id.len() > 96 {
            return false;
        }
        let mut owner = self.ipc_client_id.write().await;
        match owner.as_deref() {
            None => {
                *owner = Some(client_id.to_owned());
                true
            }
            Some(existing) => existing == client_id,
        }
    }

    /// Included in local-only diagnostics so installer and in-game connection
    /// failures can distinguish "runtime has no game" from an ownership clash.
    pub async fn ipc_client_id(&self) -> Option<String> {
        self.ipc_client_id.read().await.clone()
    }

    pub fn try_begin_control(&self) -> bool {
        self.control_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    }

    pub fn end_control(&self) {
        self.control_in_flight.store(false, Ordering::Release);
    }

    pub async fn ingest(&self, message: Envelope) -> Result<()> {
        self.apply_local(message).await
    }

    pub async fn ingest_remote(&self, message: Envelope) -> Result<()> {
        let session_id = self
            .local_session_id
            .read()
            .await
            .clone()
            .context("host session is not connected")?;
        self.apply_host_message(&session_id, message).await
    }

    async fn apply_local(&self, mut message: Envelope) -> Result<()> {
        if message.kind == "client.hello" {
            self.normalize_local_game_build(&mut message).await?;
        }
        self.validate(&message)?;
        if message.kind == "client.ping" {
            let _ = self.events.send(Envelope::new(
                "runtime.heartbeat",
                message.sequence,
                json!({"requestId":message.request_id,"runtimeTimeMs":unix_ms()}),
            ));
            return Ok(());
        }
        if game_commands::handle(self, &message).await? {
            return Ok(());
        }
        self.apply_common(&message).await?;
        let session = self.local_session_id.read().await.clone();
        if matches!(
            message.kind.as_str(),
            "input.tap" | "render.anchor" | "render.keyframe"
        ) {
            if let Some(session_id) = session.as_deref() {
                self.ingest_render_event(
                    session_id,
                    &message,
                    self.is_host.load(Ordering::Relaxed),
                )
                .await?;
            }
        }
        if message.kind == "render.sample" {
            let sample = crate::model::RenderSample {
                session_id: 0,
                sequence: message.sequence as u32,
                run_time_us: message.run_time_us,
                beat: message
                    .payload
                    .get("beat")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
                paddle_angle: message
                    .payload
                    .get("paddleAngle")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
                tap_mask: message
                    .payload
                    .get("tapMask")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16,
                flags: message
                    .payload
                    .get("flags")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u16,
            };
            if let Some(session_id) = session.as_deref() {
                self.renderer.push_sample(session_id, sample.clone());
                if !self.is_host.load(Ordering::Relaxed) {
                    let _ = self.network.send_render_sample(&sample).await;
                }
            }
        }
        if message.kind.starts_with("run.") {
            self.append_journal(&message)?;
            if let Some(session_id) = session.as_deref() {
                if self.is_host.load(Ordering::Relaxed) {
                    self.apply_run_to_room(session_id, &message).await?;
                    self.sync_renderer_player(session_id).await;
                    self.mark_room_dirty();
                } else {
                    self.network.broadcast(message.clone()).await;
                }
            }
        } else if matches!(
            message.kind.as_str(),
            "client.hello" | "input.tap" | "render.anchor" | "render.keyframe" | "chart.status"
        ) && !self.is_host.load(Ordering::Relaxed)
        {
            self.network.broadcast(message.clone()).await;
        }
        let _ = self.events.send(message);
        Ok(())
    }

    /// Beatblock already exposes the exact upstream build token in the version
    /// it draws on its menu. Normalize that value once and use it for room
    /// interoperability rather than maintaining an executable allowlist.
    async fn normalize_local_game_build(&self, message: &mut Envelope) -> Result<()> {
        let payload = message
            .payload
            .as_object_mut()
            .context("client.hello payload must be an object")?;
        let displayed = payload
            .get("gameVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let identity = match GameBuildIdentity::from_displayed_version(displayed) {
            Ok(identity) => identity,
            Err(display_error) => {
                let directory = self
                    .config
                    .read()
                    .await
                    .game_directory
                    .clone()
                    .context(
                        "Beatblock build identity and configured game directory are unavailable",
                    )
                    .with_context(|| display_error.to_string())?;
                tokio::task::spawn_blocking(move || {
                    GameBuildIdentity::from_game_directory(Path::new(&directory))
                })
                .await
                .context("Beatblock build identity worker stopped")??
            }
        };
        payload.insert(
            "gameVersion".into(),
            json!(identity.displayed_version.clone()),
        );
        payload.insert("gameBuildId".into(), json!(identity.build_id.clone()));
        payload.insert(
            "gameBuildSource".into(),
            serde_json::to_value(&identity.source)?,
        );
        payload.insert("gameBuild".into(), serde_json::to_value(identity)?);
        Ok(())
    }

    async fn local_game_build_identity(&self) -> Result<GameBuildIdentity> {
        let client = self.client.read().await;
        let value = client
            .get("gameBuild")
            .cloned()
            .context("launch Beatblock and open Online before hosting or joining")?;
        serde_json::from_value(value).context("read the normalized Beatblock build identity")
    }

    /// Converts ordered, source-authored render events into the same cached
    /// timeline as UDP motion samples. Hosts relay only these compact events to
    /// authorized Commentators; ordinary spectators never receive 60 Hz state.
    async fn ingest_render_event(
        &self,
        participant_id: &str,
        message: &Envelope,
        relay: bool,
    ) -> Result<()> {
        let relayed_kind = match message.kind.as_str() {
            "input.tap" | "render.tap" => {
                let beat = message
                    .payload
                    .get("beat")
                    .and_then(Value::as_f64)
                    .context("tap beat is required")? as f32;
                let judgement_beat = message
                    .payload
                    .get("judgementBeat")
                    .and_then(Value::as_f64)
                    .unwrap_or(beat as f64) as f32;
                let pressed = message
                    .payload
                    .get("pressed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let released = message
                    .payload
                    .get("released")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.renderer.push_tap(
                    participant_id,
                    message.sequence,
                    beat,
                    judgement_beat,
                    pressed,
                    released,
                );
                Some("render.tap")
            }
            "render.anchor" => {
                let first_note_beat = message
                    .payload
                    .get("firstNoteBeat")
                    .and_then(Value::as_f64)
                    .context("render anchor firstNoteBeat is required")?
                    as f32;
                let input_offset_ms = message
                    .payload
                    .get("inputOffsetMs")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32;
                self.renderer
                    .push_render_anchor(participant_id, first_note_beat, input_offset_ms);
                Some("render.anchor")
            }
            "render.keyframe" => {
                let totals = serde_json::from_value(
                    message
                        .payload
                        .get("totals")
                        .cloned()
                        .context("render keyframe totals are required")?,
                )?;
                let accuracy = message
                    .payload
                    .get("accuracy")
                    .and_then(Value::as_f64)
                    .unwrap_or_else(|| crate::model::ScoreTotals::accuracy(&totals));
                let average_offset = message
                    .payload
                    .get("averageOffset")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                if !accuracy.is_finite()
                    || !(0.0..=100.0).contains(&accuracy)
                    || !average_offset.is_finite()
                {
                    anyhow::bail!("render keyframe contains invalid score values");
                }
                self.renderer.push_score_state(
                    participant_id,
                    RenderScoreState {
                        sequence: message.sequence,
                        run_time_us: message.run_time_us,
                        accuracy: accuracy as f32,
                        average_offset: average_offset as f32,
                        totals,
                        results: message
                            .payload
                            .get("results")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                );
                Some("render.keyframe")
            }
            _ => None,
        };

        if let (true, Some(relayed_kind)) = (relay, relayed_kind) {
            let assigned =
                self.broadcast_plan.read().await.slots.iter().any(|slot| {
                    slot.active && slot.participant_id.as_deref() == Some(participant_id)
                });
            // Anchors are emitted once per run. Cache them on every authorized
            // mirror so a later assignment does not wait for an event that has
            // already passed.
            if assigned || relayed_kind == "render.anchor" {
                let subscribers = self
                    .commentator_subscribers
                    .read()
                    .await
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                let mut payload = message.payload.clone();
                if let Some(payload) = payload.as_object_mut() {
                    payload.insert("participantId".into(), json!(participant_id));
                }
                for subscriber in subscribers {
                    let mut relayed =
                        Envelope::new(relayed_kind, message.sequence, payload.clone());
                    // Renderer keyframes are aligned against compact samples in
                    // the source player's monotonic clock, not the relay host's.
                    relayed.run_time_us = message.run_time_us;
                    let _ = self.network.send_to(&subscriber, relayed).await;
                }
            }
        }
        Ok(())
    }

    async fn apply_host_message(&self, session_id: &str, mut message: Envelope) -> Result<()> {
        localize_host_schedule(&mut message, unix_ms());
        self.validate(&message)?;
        let mut publish_message = true;
        if message.kind == "room.start_scheduled" {
            self.renderer.begin_run();
        }
        if matches!(
            message.kind.as_str(),
            "render.tap" | "render.anchor" | "render.keyframe"
        ) {
            let participant_id = message
                .payload
                .get("participantId")
                .and_then(Value::as_str)
                .context("relayed render event requires participantId")?
                .to_owned();
            self.ingest_render_event(&participant_id, &message, false)
                .await?;
            publish_message = false;
        } else if message.kind == "chart.transfer_offer" {
            let offer: ChartTransferHeader = serde_json::from_value(message.payload.clone())?;
            offer.validate()?;
            *self.pending_inbound_transfer_offer.write().await =
                Some((session_id.to_owned(), offer.clone()));
            let room_id = self.room.read().await.snapshot.id.clone();
            if self.trusted_transfer_rooms.read().await.contains(&room_id)
                && !offer.contains_executable_content
            {
                self.network
                    .authorize_incoming_chart_transfer(session_id, offer.clone(), false)
                    .await?;
                *self.pending_inbound_transfer_offer.write().await = None;
                self.network
                    .broadcast(Envelope::new(
                        "chart.transfer_decision",
                        0,
                        json!({
                            "requestId":offer.request_id,
                            "accept":true,
                            "executableContentConfirmed":false
                        }),
                    ))
                    .await;
                // The offer has already been authorized and answered. Do not
                // surface a stale manual Accept/Reject prompt to the game UI.
                publish_message = false;
            }
        } else if message.kind == "broadcast.plan" {
            let plan: BroadcastPlan = serde_json::from_value(message.payload.clone())?;
            let current_revision = self.broadcast_plan.read().await.revision;
            if plan.revision > current_revision {
                *self.broadcast_plan.write().await = plan;
                self.apply_broadcast_plan_locally().await;
            }
        } else if message.kind == "broadcast.revoked" {
            self.local_mirror_enabled.store(false, Ordering::Release);
            self.renderer.stop_all();
            self.exports.publish_broadcast_metadata(
                self.broadcast_plan.read().await.revision,
                "commentator_mirror",
                false,
            );
            self.publish_renderer_snapshot();
        }
        if message.kind == "room.snapshot" || message.kind == "lobby.snapshot" {
            let snapshot: RoomSnapshot = serde_json::from_value(message.payload.clone())?;
            snapshot.modifiers.validate()?;
            *self.lobby.write().await = serde_json::to_value(&snapshot)?;
            self.room.write().await.snapshot = snapshot.clone();
            self.exports.publish_room(
                self.room.read().await.snapshot.clone(),
                self.renderer.slots(),
            );
            if self.local_mirror_enabled.load(Ordering::Acquire) {
                let local = self.local_session_id.read().await.clone();
                let still_allowed = local.is_some_and(|session_id| {
                    snapshot.participants.iter().any(|participant| {
                        participant.session_id == session_id
                            && participant.admitted
                            && participant.role == ParticipantRole::Spectator
                            && participant.commentator_access
                    })
                });
                if !still_allowed {
                    self.local_mirror_enabled.store(false, Ordering::Release);
                    self.renderer.stop_all();
                    self.publish_renderer_snapshot();
                }
            }
            self.try_auto_match_locked_chart().await;
        }
        self.apply_common(&message).await?;
        if publish_message {
            let _ = self.events.send(message);
        }
        Ok(())
    }

    async fn apply_common(&self, message: &Envelope) -> Result<()> {
        match message.kind.as_str() {
            "gameplay.snapshot" => {
                let next = serde_json::from_value::<GameplayState>(message.payload.clone())?;
                *self.gameplay.write().await = next.clone();
                self.exports.publish_gameplay(next.clone());
                if let Some(session_id) = self.local_session_id.read().await.as_deref() {
                    self.renderer.push_player_state(session_id, next);
                }
            }
            "room.snapshot" | "lobby.snapshot" => {
                *self.lobby.write().await = message.payload.clone();
            }
            "client.hello" => {
                *self.client.write().await = message.payload.clone();
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_network_event(&self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::Authenticated {
                session_id,
                display_name,
                role,
                hosting,
                ..
            } => {
                if hosting {
                    if let Err(error) = self.room.write().await.request_join_with_id(
                        session_id.clone(),
                        &display_name,
                        role,
                    ) {
                        self.network
                            .disconnect_peer(&session_id, &error.to_string(), false)
                            .await;
                        return Err(error);
                    }
                    self.broadcast_room().await?;
                } else {
                    *self.local_session_id.write().await = Some(session_id);
                    *self.connection_status.write().await = "connected".into();
                }
            }
            NetworkEvent::Envelope {
                session_id,
                envelope,
            } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.validate(&envelope)?;
                    let class = match classify_peer_message(&envelope) {
                        Ok(class) => class,
                        Err(error) => {
                            self.network
                                .disconnect_peer(
                                    &session_id,
                                    "Peer sent a forbidden or oversized control message",
                                    false,
                                )
                                .await;
                            return Err(error);
                        }
                    };
                    self.require_admitted_peer(&session_id).await?;
                    if class == PeerMessageClass::Telemetry {
                        if matches!(
                            envelope.kind.as_str(),
                            "input.tap" | "render.anchor" | "render.keyframe"
                        ) {
                            self.ingest_render_event(&session_id, &envelope, true)
                                .await?;
                        }
                        // Other telemetry has no authoritative host consumer.
                        // Keep it out of shared state and the UI event stream.
                        return Ok(());
                    }
                    if class == PeerMessageClass::Run {
                        let room_id = self.room.read().await.snapshot.id.clone();
                        self.storage.queue_event(&room_id, &envelope)?;
                        self.apply_run_to_room(&session_id, &envelope).await?;
                        self.sync_renderer_player(&session_id).await;
                        let _ = self.events.send(envelope);
                        self.mark_room_dirty();
                        return Ok(());
                    }
                    let mut room_changed = false;
                    let publish_event = true;
                    if envelope.kind == "client.ready" {
                        let ready = envelope
                            .payload
                            .get("ready")
                            .and_then(Value::as_bool)
                            .context("client.ready requires a boolean ready field")?;
                        self.room.write().await.set_ready(&session_id, ready)?;
                        room_changed = true;
                    } else if envelope.kind == "chart.status" {
                        let verified = envelope
                            .payload
                            .get("verified")
                            .and_then(Value::as_bool)
                            .context("chart.status requires a boolean verified field")?;
                        let reason = envelope
                            .payload
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(|reason| reason.chars().take(512).collect::<String>());
                        self.room
                            .write()
                            .await
                            .set_verified(&session_id, verified, reason)?;
                        room_changed = true;
                    } else if envelope.kind == "chart.transfer_request" {
                        let room = self.room.read().await.snapshot.clone();
                        let requested_hash = envelope
                            .payload
                            .get("chartHash")
                            .and_then(Value::as_str)
                            .context("chart transfer request requires chartHash")?;
                        // Re-check transfer eligibility at the authoritative
                        // host boundary before any path access or archive work.
                        // The earlier generic admission check is insufficient:
                        // spectators and already-verified players are admitted
                        // but must not be able to consume packaging capacity.
                        let chart =
                            transferable_chart_for_participant(&room, &session_id, requested_hash)?;
                        let selected = PathBuf::from(
                            self.selected_chart_path
                                .read()
                                .await
                                .clone()
                                .context("the host chart package path is unavailable")?,
                        );
                        if self
                            .pending_transfer_offers
                            .read()
                            .await
                            .contains_key(&session_id)
                        {
                            anyhow::bail!("this peer already has an active chart transfer offer");
                        }
                        let mut active_builds = self.active_transfer_builds.write().await;
                        if active_builds.contains(&session_id) {
                            anyhow::bail!("a chart transfer offer is already being prepared");
                        }
                        if active_builds.len() >= MAX_QUEUED_TRANSFER_BUILDS {
                            anyhow::bail!("the chart transfer packaging queue is full");
                        }
                        active_builds.insert(session_id.clone());
                        drop(active_builds);
                        let state = self.clone();
                        let recipient = session_id.clone();
                        tokio::spawn(async move {
                            let result = state
                                .prepare_chart_transfer_offer(recipient.clone(), chart, selected)
                                .await;
                            state
                                .active_transfer_builds
                                .write()
                                .await
                                .remove(&recipient);
                            if let Err(error) = result {
                                let _ = state.events.send(Envelope::new(
                                    "chart.transfer_failed",
                                    0,
                                    json!({"message":error.to_string()}),
                                ));
                            }
                        });
                    } else if envelope.kind == "chart.transfer_decision" {
                        let accept = envelope
                            .payload
                            .get("accept")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let request_id = envelope
                            .payload
                            .get("requestId")
                            .and_then(Value::as_str)
                            .context("transfer requestId is required")?;
                        let mut pending_offers = self.pending_transfer_offers.write().await;
                        let pending = pending_offers
                            .get(&session_id)
                            .cloned()
                            .context("there is no active transfer offer for this peer")?;
                        if pending.1.request_id != request_id {
                            anyhow::bail!("transfer decision does not match the active offer");
                        }
                        if accept {
                            let executable_confirmed = envelope
                                .payload
                                .get("executableContentConfirmed")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            if pending.1.contains_executable_content && !executable_confirmed {
                                anyhow::bail!(
                                    "script or executable content requires separate confirmation"
                                );
                            }
                        }
                        let pending = pending_offers
                            .remove(&session_id)
                            .context("active transfer offer disappeared")?;
                        drop(pending_offers);
                        if accept {
                            let network = self.network.clone();
                            let events = self.events.clone();
                            let recipient = session_id.clone();
                            tokio::spawn(async move {
                                let sent = tokio::time::timeout(
                                    std::time::Duration::from_secs(120),
                                    network.send_chart_transfer(&recipient, &pending.1, &pending.0),
                                )
                                .await;
                                if let Err(error) = sent
                                    .map_err(|_| anyhow::anyhow!("chart transfer timed out"))
                                    .and_then(|result| result)
                                {
                                    let _ = events.send(Envelope::new(
                                        "chart.transfer_failed",
                                        0,
                                        json!({"message":error.to_string()}),
                                    ));
                                }
                                let _ = tokio::fs::remove_file(&pending.0).await;
                            });
                        } else {
                            let _ = tokio::fs::remove_file(&pending.0).await;
                        }
                    } else if envelope.kind == "broadcast.subscribe" {
                        let enabled = envelope
                            .payload
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let allowed = self.room.read().await.snapshot.participants.iter().any(
                            |participant| {
                                participant.session_id == session_id
                                    && participant.admitted
                                    && participant.role == ParticipantRole::Spectator
                                    && participant.commentator_access
                            },
                        );
                        if enabled && !allowed {
                            anyhow::bail!("peer is not an authorized Commentator");
                        }
                        if enabled {
                            self.commentator_subscribers
                                .write()
                                .await
                                .insert(session_id.clone());
                            let plan = self.broadcast_plan.read().await.clone();
                            self.network
                                .send_to(
                                    &session_id,
                                    Envelope::new("broadcast.plan", plan.revision, json!(plan)),
                                )
                                .await?;
                        } else {
                            self.commentator_subscribers
                                .write()
                                .await
                                .remove(&session_id);
                        }
                    } else if envelope.kind == "broadcast.mirror_status" {
                        let allowed = self.room.read().await.snapshot.participants.iter().any(
                            |participant| {
                                participant.session_id == session_id
                                    && participant.commentator_access
                            },
                        );
                        if !allowed {
                            anyhow::bail!("peer is not an authorized Commentator");
                        }
                        let mut status: CommentatorMirrorStatus =
                            serde_json::from_value(envelope.payload.clone())?;
                        status.error = status.error.map(|value| value.chars().take(160).collect());
                        self.commentator_statuses
                            .write()
                            .await
                            .insert(session_id.clone(), status);
                    }
                    if publish_event {
                        let _ = self.events.send(envelope);
                    }
                    if room_changed {
                        self.broadcast_room().await?;
                    }
                } else if envelope.kind == "room.removed" {
                    let reason = envelope
                        .payload
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("Removed from the room")
                        .to_owned();
                    self.leave_room().await?;
                    self.emit_error(reason);
                } else {
                    self.apply_host_message(&session_id, envelope).await?;
                }
            }
            NetworkEvent::RenderSample { session_id, sample } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.require_admitted_peer(&session_id).await?;
                    self.renderer.push_sample(&session_id, sample.clone());
                    let assigned = self.broadcast_plan.read().await.slots.iter().any(|slot| {
                        slot.active && slot.participant_id.as_deref() == Some(session_id.as_str())
                    });
                    if assigned {
                        let subscribers = self
                            .commentator_subscribers
                            .read()
                            .await
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>();
                        if !subscribers.is_empty() {
                            let mut relayed = sample;
                            relayed.session_id = render_source_id(&session_id);
                            let _ = self
                                .network
                                .send_render_sample_to(&subscribers, &relayed)
                                .await;
                        }
                    }
                } else if self.local_mirror_enabled.load(Ordering::Acquire) {
                    let participant_id = self
                        .broadcast_plan
                        .read()
                        .await
                        .slots
                        .iter()
                        .find(|slot| slot.render_source_id == Some(sample.session_id))
                        .and_then(|slot| slot.participant_id.clone());
                    if let Some(participant_id) = participant_id {
                        self.renderer.push_sample(&participant_id, sample);
                    }
                }
            }
            NetworkEvent::ChartTransferProgress {
                request_id,
                received,
                total,
                ..
            } => {
                let percent = received.saturating_mul(100).checked_div(total).unwrap_or(0);
                let _ = self.events.send(Envelope::new(
                    "chart.transfer_progress",
                    0,
                    json!({
                        "requestId":request_id,
                        "receivedBytes":received,
                        "totalBytes":total,
                        "percent":percent
                    }),
                ));
            }
            NetworkEvent::ChartTransferReceived {
                header,
                path,
                executable_content_confirmed,
                ..
            } => {
                if self.is_host.load(Ordering::Relaxed) {
                    let _ = tokio::fs::remove_file(path).await;
                    anyhow::bail!("the host does not accept chart transfers");
                }
                // Archive inspection and canonical hashing are blocking work.
                // Keep them off the sole network event consumer, and serialize
                // installs to prevent several large packages competing for CPU
                // and disk at once.
                let state = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = state
                        .install_received_chart_transfer(header, path, executable_content_confirmed)
                        .await
                    {
                        let _ = state.events.send(Envelope::new(
                            "chart.transfer_failed",
                            0,
                            json!({"message":error.to_string()}),
                        ));
                    }
                });
            }
            NetworkEvent::Disconnected { session_id, reason } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.room.write().await.disconnect(&session_id);
                    self.stop_participant_renderer(&session_id).await?;
                    if let Some((path, _)) = self
                        .pending_transfer_offers
                        .write()
                        .await
                        .remove(&session_id)
                    {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                    self.commentator_subscribers
                        .write()
                        .await
                        .remove(&session_id);
                    self.commentator_statuses.write().await.remove(&session_id);
                    self.broadcast_room().await?;
                } else {
                    let normalized_reason = reason.to_ascii_lowercase();
                    let terminal_disconnect = normalized_reason.contains("rejected")
                        || normalized_reason.contains("removed from the room")
                        || normalized_reason.contains("room closed")
                        || normalized_reason.contains("runtime stopped")
                        || normalized_reason == "username taken";
                    if terminal_disconnect {
                        self.leave_room().await?;
                        self.emit_error(reason);
                        return Ok(());
                    }
                    let connected_session = self.local_session_id.read().await.clone();
                    let room_closed = self.room.read().await.snapshot.lifecycle
                        == crate::model::RoomLifecycle::Closed;
                    if connected_session.is_some()
                        && !room_closed
                        && self.reconnect_request.read().await.is_some()
                    {
                        *self.connection_status.write().await = "reconnecting".into();
                        self.spawn_reconnect();
                    } else {
                        *self.connection_status.write().await = "offline".into();
                    }
                    self.publish_runtime_snapshot().await?;
                    self.emit_error(reason);
                }
            }
            NetworkEvent::Diagnostic(reason) => {
                // Peer authentication failures, malformed datagrams, and a
                // remote writer closing are diagnostics. They must never fail
                // an unrelated local host/join/control operation in the GUI.
                tracing::warn!(%reason, "peer transport diagnostic");
            }
        }
        Ok(())
    }

    async fn install_received_chart_transfer(
        &self,
        header: ChartTransferHeader,
        path: PathBuf,
        executable_content_confirmed: bool,
    ) -> Result<()> {
        let _cleanup = TemporaryFileCleanup::new(path.clone());
        let _install_slot = self
            .transfer_install_slot
            .clone()
            .acquire_owned()
            .await
            .context("chart transfer installer stopped")?;
        let expected = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .clone()
            .context("received a chart after the room lock was cleared")?;
        if expected.hash != header.chart_hash {
            anyhow::bail!("received chart does not match the active lock");
        }
        let cache = self.data_dir.join("chart-cache");
        let archive_sha = header.archive_sha256.clone();
        let chart_hash = header.chart_hash.clone();
        let archive = path;
        let installed = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
            let installed = crate::transfer::install_received_package(
                &archive,
                &archive_sha,
                &cache,
                executable_content_confirmed,
            )?;
            let canonical = crate::chart_hash::canonical_chart_hash_cached(
                &installed,
                cache.join(".hash-index"),
            )?;
            if canonical.hash != chart_hash {
                anyhow::bail!(
                    "transferred archive passed SHA-256 but its canonical chart hash is wrong"
                );
            }
            std::fs::write(
                installed.join(".bbt-chart-hash"),
                format!("{}\n", canonical.hash),
            )?;
            Ok(installed)
        })
        .await??;
        let installed_text = installed.to_string_lossy().into_owned();
        *self.selected_chart_path.write().await = Some(installed_text.clone());
        self.chart_paths
            .write()
            .await
            .insert(header.chart_hash.clone(), installed_text);
        self.set_local_verified(true, None).await?;
        let _ = self.events.send(Envelope::new(
            "chart.transfer_complete",
            0,
            json!({"requestId":header.request_id,"chartHash":header.chart_hash}),
        ));
        Ok(())
    }

    async fn prepare_chart_transfer_offer(
        &self,
        session_id: String,
        chart: ChartLock,
        selected: PathBuf,
    ) -> Result<()> {
        let _build_slot = self
            .transfer_build_slots
            .clone()
            .acquire_owned()
            .await
            .context("chart transfer packager stopped")?;
        if !self.is_host.load(Ordering::Acquire) {
            anyhow::bail!("room closed while waiting to prepare the chart package");
        }
        let queued_snapshot = self.room.read().await.snapshot.clone();
        transferable_chart_for_participant(&queued_snapshot, &session_id, &chart.hash)
            .context("chart transfer eligibility changed while waiting for the packager")?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let outgoing = self
            .data_dir
            .join("transfer-outgoing")
            .join(format!("{request_id}.zip"));
        let chart_hash = chart.hash.clone();
        let (path, offer) = tokio::task::spawn_blocking(move || {
            let path = crate::transfer::archive_chart_directory(&selected, &outgoing)?;
            let offer = crate::transfer::inspect_offer(&path, "room host")?;
            Ok::<_, anyhow::Error>((path, offer))
        })
        .await??;
        let mut cleanup = TemporaryFileCleanup::new(path.clone());
        if !self.is_host.load(Ordering::Acquire) {
            anyhow::bail!("room closed while preparing the chart package");
        }
        let active_snapshot = self.room.read().await.snapshot.clone();
        transferable_chart_for_participant(&active_snapshot, &session_id, &chart_hash)
            .context("chart transfer eligibility changed while preparing its package")?;
        let header = ChartTransferHeader {
            request_id,
            name: offer.name,
            size: offer.size,
            archive_sha256: offer.sha256,
            chart_hash,
            contains_executable_content: offer.contains_executable_content,
        };
        header.validate()?;
        self.pending_transfer_offers
            .write()
            .await
            .insert(session_id.clone(), (path, header.clone()));
        if let Err(error) = self
            .network
            .send_to(
                &session_id,
                Envelope::new("chart.transfer_offer", 0, json!(header)),
            )
            .await
        {
            self.pending_transfer_offers
                .write()
                .await
                .remove(&session_id);
            return Err(error);
        }
        cleanup.armed = false;
        Ok(())
    }

    async fn apply_run_to_room(&self, session_id: &str, message: &Envelope) -> Result<()> {
        let mut room = self.room.write().await;
        let run_id = message
            .run_id
            .as_deref()
            .or_else(|| message.payload.get("runId").and_then(Value::as_str));
        let result = (|| -> Result<()> {
            match message.kind.as_str() {
                "run.started" => {
                    let run_id = run_id.context("run.started requires a runId")?;
                    let max_hits = message
                        .payload
                        .get("maxHits")
                        .and_then(Value::as_u64)
                        .context("run.started requires an integer maxHits")?;
                    room.start_run(session_id, run_id, max_hits)
                }
                "run.score_delta" | "score.mutation" => {
                    let run_id = run_id.context("score update requires a runId")?;
                    let sequence = message
                        .payload
                        .get("runSequence")
                        .and_then(Value::as_u64)
                        .unwrap_or(message.sequence);
                    room.ingest_score(session_id, run_id, sequence, &message.payload)
                }
                "run.invalid" => {
                    let run_id = run_id.context("run.invalid requires a runId")?;
                    let reason = message
                        .payload
                        .get("reason")
                        .and_then(Value::as_str)
                        .context("run.invalid requires a string reason")?;
                    let dnf = message
                        .payload
                        .get("dnf")
                        .and_then(Value::as_bool)
                        .context("run.invalid requires a boolean dnf field")?;
                    room.invalidate(session_id, run_id, reason.into(), dnf)
                }
                "run.finished" => {
                    let run_id = run_id.context("run.finished requires a runId")?;
                    room.finish_run(session_id, run_id)
                }
                _ => Ok(()),
            }
        })();
        if let Err(error) = result {
            let recorded = room.reject_run_event(session_id, &message.kind, &error.to_string());
            tracing::warn!(
                kind = %message.kind,
                %session_id,
                %error,
                invalidated = recorded,
                "run event rejected without disconnecting IPC"
            );
        }
        Ok(())
    }

    async fn sync_renderer_player(&self, session_id: &str) {
        let room = self.room.read().await.snapshot.clone();
        let Some(player) = room
            .participants
            .iter()
            .find(|p| p.session_id == session_id)
        else {
            return;
        };
        let state = GameplayState {
            state: format!("{:?}", room.lifecycle).to_lowercase(),
            player_name: player.display_name.clone(),
            song_name: room
                .chart
                .as_ref()
                .map(|c| c.song_name.clone())
                .unwrap_or_else(|| "No chart".into()),
            lobby_name: room.name,
            accuracy: player.accuracy,
            combo: player.totals.combo,
            misses: player.totals.misses,
            rank: player.rank.unwrap_or(0) as u64,
            progress: player.progress,
            connected: player.connected,
            updated_at_ms: crate::room::unix_ms(),
            beat: 0.0,
            paddle_angle: 0.0,
            tap_mask: 0,
            health: -1.0,
        };
        self.renderer.push_player_state(session_id, state);
    }

    pub async fn host_room(&self, options: HostRoomOptions) -> Result<SocketAddr> {
        let HostRoomOptions {
            room_name,
            password,
            port,
            admission_mode,
            host_participating,
            validity_checks_enabled,
            require_same_game_build,
            modifiers,
        } = options;
        let room_name = validated_room_name(&room_name)?;
        self.cancel_reconnect();
        self.cancel_nat_renewal();
        self.release_nat_mapping().await;
        *self.reconnect_request.write().await = None;
        *self.connection_status.write().await = "starting".into();
        let host_name = self.config.read().await.display_name.clone();
        let room = initial_host_room(
            room_name,
            host_name,
            admission_mode,
            host_participating,
            validity_checks_enabled,
            require_same_game_build,
            modifiers,
        )?;
        let session_id = room.snapshot.host_session_id.clone();
        let game_build = if require_same_game_build {
            Some(self.local_game_build_identity().await?)
        } else {
            self.local_game_build_identity().await.ok()
        };
        let local_address = match self
            .network
            .start_host_with_game_build(port, password, game_build, require_same_game_build)
            .await
        {
            Ok(address) => address,
            Err(error) => {
                self.reset_offline_state().await;
                return Err(error);
            }
        };
        let mapping = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            crate::nat::map_host_port(local_address.port()),
        )
        .await
        .ok()
        .and_then(Result::ok);
        *self.mapped_host_port.write().await = mapping.as_ref().map(|_| local_address.port());
        let address = mapping
            .as_ref()
            .map(|mapping| mapping.external_address)
            .unwrap_or(local_address);
        *self.host_join_address.write().await = Some(address);
        *self.nat_method.write().await = mapping
            .as_ref()
            .map(|mapping| mapping.method.to_owned())
            .or_else(|| Some("LAN / manual port forwarding".into()));
        *self.room.write().await = room;
        *self.local_session_id.write().await = Some(session_id);
        self.is_host.store(true, Ordering::Relaxed);
        *self.connection_status.write().await = "hosting".into();
        if let Err(error) = self.sync_room_state().await {
            self.release_nat_mapping().await;
            self.network.shutdown_with_reason("Room setup failed").await;
            self.reset_offline_state().await;
            return Err(error).context("persist initial host room state");
        }
        if let Err(error) = self.publish_runtime_snapshot().await {
            self.release_nat_mapping().await;
            self.network.shutdown_with_reason("Room setup failed").await;
            self.reset_offline_state().await;
            return Err(error).context("publish initial host room state");
        }
        self.spawn_nat_renewal(
            local_address.port(),
            self.room.read().await.snapshot.id.clone(),
        );
        Ok(address)
    }

    pub async fn save_profile(
        &self,
        display_name: String,
        address: Option<SocketAddr>,
        role: ParticipantRole,
    ) -> Result<()> {
        let mut config = self.config.write().await;
        let mut next = config.clone();
        next.display_name = display_name.trim().to_owned();
        next.requested_role = role;
        if let Some(address) = address {
            next.host_address = address.ip().to_string();
            next.host_port = address.port();
        }
        write_config_atomically(&self.data_dir, &next)?;
        *config = next;
        Ok(())
    }

    /// Hosting changes the preferred display name and UDP port without
    /// replacing the user's last remote host address with a loopback address.
    pub async fn save_host_profile(&self, display_name: String, port: u16) -> Result<()> {
        let mut config = self.config.write().await;
        let mut next = config.clone();
        next.display_name = display_name.trim().to_owned();
        next.requested_role = ParticipantRole::Host;
        next.host_port = port;
        write_config_atomically(&self.data_dir, &next)?;
        *config = next;
        Ok(())
    }

    pub async fn replace_config(&self, mut config: CompanionConfig) -> Result<CompanionConfig> {
        config.display_name = config.display_name.trim().to_owned();
        config.host_address = config.host_address.trim().to_owned();
        write_config_atomically(&self.data_dir, &config)?;
        *self.config.write().await = config.clone();
        Ok(config)
    }

    pub async fn join_room(
        &self,
        address: SocketAddr,
        password: &str,
        display_name: &str,
        role: ParticipantRole,
    ) -> Result<String> {
        self.cancel_reconnect();
        self.cancel_nat_renewal();
        self.release_nat_mapping().await;
        self.is_host.store(false, Ordering::Relaxed);
        *self.connection_status.write().await = "connecting".into();
        let game_build = self.local_game_build_identity().await.ok();
        let session = match self
            .network
            .join_with_game_build(address, password, display_name, role, game_build.clone())
            .await
        {
            Ok(session) => session,
            Err(error) => {
                self.reset_offline_state().await;
                return Err(error);
            }
        };
        *self.reconnect_request.write().await = Some(ReconnectRequest {
            address,
            password: password.to_owned(),
            display_name: display_name.to_owned(),
            role,
            game_build,
        });
        *self.local_session_id.write().await = Some(session.clone());
        *self.connection_status.write().await = "connected".into();
        if let Err(error) = self.publish_runtime_snapshot().await {
            self.network
                .shutdown_with_reason("Room connection setup failed")
                .await;
            self.network.clear_client_resume().await;
            self.reset_offline_state().await;
            return Err(error).context("publish initial joined-room state");
        }
        Ok(session)
    }

    pub async fn admit(&self, session_id: &str, admit: bool, role: ParticipantRole) -> Result<()> {
        self.require_host()?;
        if !admit {
            let _ = self
                .network
                .send_to(
                    session_id,
                    Envelope::new(
                        "room.removed",
                        0,
                        json!({"reason":"Join request rejected by host"}),
                    ),
                )
                .await;
            self.room.write().await.kick(session_id)?;
            self.stop_participant_renderer(session_id).await?;
            self.commentator_subscribers
                .write()
                .await
                .remove(session_id);
            self.commentator_statuses.write().await.remove(session_id);
            self.network
                .disconnect_peer(session_id, "Join request rejected by host", false)
                .await;
        } else {
            self.room.write().await.admit(session_id, true, role)?;
            if role == ParticipantRole::Spectator {
                self.stop_participant_renderer(session_id).await?;
            }
        }
        self.broadcast_room().await
    }

    pub async fn set_participant_role(
        &self,
        session_id: &str,
        role: ParticipantRole,
    ) -> Result<()> {
        self.require_host()?;
        if session_id == self.room.read().await.snapshot.host_session_id {
            anyhow::bail!("use the host play control to change host participation");
        }
        self.room.write().await.set_role(session_id, role)?;
        if role == ParticipantRole::Spectator {
            self.stop_participant_renderer(session_id).await?;
        } else {
            self.commentator_subscribers
                .write()
                .await
                .remove(session_id);
            self.commentator_statuses.write().await.remove(session_id);
            let _ = self
                .network
                .send_to(
                    session_id,
                    Envelope::new(
                        "broadcast.revoked",
                        0,
                        json!({"reason":"Commentator access ended when the room role changed"}),
                    ),
                )
                .await;
        }
        self.broadcast_room().await
    }

    pub async fn set_host_participating(&self, participating: bool) -> Result<()> {
        self.require_host()?;
        let host_session_id = self.room.read().await.snapshot.host_session_id.clone();
        self.room
            .write()
            .await
            .set_host_participating(participating)?;
        if !participating {
            // A directing host must never remain attached to an OBS renderer
            // slot after leaving the race roster.
            self.stop_participant_renderer(&host_session_id).await?;
        }
        self.broadcast_room().await
    }

    pub async fn set_validity_checks(&self, enabled: bool) -> Result<()> {
        self.require_host()?;
        self.room.write().await.set_validity_checks(enabled)?;
        self.broadcast_room().await
    }

    pub async fn set_same_game_build_required(&self, required: bool) -> Result<()> {
        self.require_host()?;
        if required {
            anyhow::bail!(
                "Same Build cannot be re-enabled for an active room; create a new room to restore strict matching"
            );
        }
        self.room
            .write()
            .await
            .set_same_game_build_required(false)?;
        self.network.relax_host_game_build_policy().await;
        self.broadcast_room().await
    }

    pub async fn set_room_modifiers(&self, modifiers: crate::model::RoomModifiers) -> Result<()> {
        self.require_host()?;
        self.room.write().await.set_modifiers(modifiers)?;
        self.broadcast_room().await
    }

    pub async fn set_auto_request_chart_transfers(&self, enabled: bool) -> Result<()> {
        self.require_host()?;
        self.room
            .write()
            .await
            .set_auto_request_chart_transfers(enabled)?;
        self.broadcast_room().await
    }

    pub async fn kick(&self, session_id: &str) -> Result<()> {
        self.require_host()?;
        let _ = self
            .network
            .send_to(
                session_id,
                Envelope::new(
                    "room.removed",
                    0,
                    json!({"reason":"Removed from the room by the host"}),
                ),
            )
            .await;
        self.room.write().await.kick(session_id)?;
        self.stop_participant_renderer(session_id).await?;
        self.commentator_subscribers
            .write()
            .await
            .remove(session_id);
        self.commentator_statuses.write().await.remove(session_id);
        self.network
            .disconnect_peer(session_id, "Removed from the room by the host", false)
            .await;
        self.broadcast_room().await
    }

    pub async fn leave_room(&self) -> Result<()> {
        self.cancel_reconnect();
        self.cancel_nat_renewal();
        self.release_nat_mapping().await;
        if self.is_host.load(Ordering::Relaxed) {
            self.network.shutdown().await;
        } else {
            self.network
                .shutdown_with_reason("Participant left room")
                .await;
        }
        *self.reconnect_request.write().await = None;
        self.network.clear_client_resume().await;
        self.renderer.stop_all();
        self.local_mirror_enabled.store(false, Ordering::Release);
        self.commentator_subscribers.write().await.clear();
        self.commentator_statuses.write().await.clear();
        *self.broadcast_plan.write().await = BroadcastPlan::empty();
        self.pending_transfer_offers.write().await.clear();
        *self.pending_inbound_transfer_offer.write().await = None;
        self.trusted_transfer_rooms.write().await.clear();
        self.active_transfer_builds.write().await.clear();
        self.network.clear_incoming_chart_transfers().await;
        *self.last_auto_match_hash.write().await = None;
        *self.last_auto_request_hash.write().await = None;
        let _ = std::fs::remove_dir_all(self.data_dir.join("transfer-outgoing"));
        self.is_host.store(false, Ordering::Relaxed);
        *self.local_session_id.write().await = None;
        *self.connection_status.write().await = "offline".into();
        *self.host_join_address.write().await = None;
        *self.nat_method.write().await = None;
        *self.room.write().await = RoomEngine::offline();
        *self.selected_chart_path.write().await = None;
        self.chart_paths.write().await.clear();
        self.sync_room_state().await?;
        self.publish_runtime_snapshot().await
    }

    pub async fn lock_chart(&self, chart: ChartLock, append_to_setlist: bool) -> Result<()> {
        self.require_host()?;
        let previous_hash = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .as_ref()
            .map(|chart| chart.hash.clone());
        if let Some(path) = self.selected_chart_path.read().await.clone() {
            self.chart_paths
                .write()
                .await
                .insert(chart.hash.clone(), path);
        }
        self.room
            .write()
            .await
            .lock_chart(chart, append_to_setlist)?;
        let active_hash = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .as_ref()
            .map(|chart| chart.hash.clone());
        *self.selected_chart_path.write().await = if let Some(hash) = active_hash.as_ref() {
            self.chart_paths.read().await.get(hash).cloned()
        } else {
            None
        };
        self.broadcast_room().await?;
        // A direct selection always starts a fresh chart lifecycle, even when
        // the host picks the same package and variant after Results. Its
        // renderer children must therefore restart to clear their completed
        // game state. Appending to a setlist is different: it must leave the
        // currently active chart and its children untouched.
        if !append_to_setlist || active_hash != previous_hash {
            self.relaunch_active_renderers().await;
        }
        Ok(())
    }

    pub async fn advance_setlist(&self) -> Result<()> {
        self.require_host()?;
        self.room.write().await.advance_setlist()?;
        if let Some(hash) = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .as_ref()
            .map(|chart| chart.hash.clone())
        {
            *self.selected_chart_path.write().await =
                self.chart_paths.read().await.get(&hash).cloned();
        }
        self.broadcast_room().await?;
        self.relaunch_active_renderers().await;
        Ok(())
    }

    pub async fn remove_setlist(&self, index: usize) -> Result<()> {
        self.require_host()?;
        let previous_hash = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .as_ref()
            .map(|chart| chart.hash.clone());
        self.room.write().await.remove_setlist(index)?;
        let active_hash = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .as_ref()
            .map(|chart| chart.hash.clone());
        *self.selected_chart_path.write().await = if let Some(hash) = active_hash.as_ref() {
            self.chart_paths.read().await.get(hash).cloned()
        } else {
            None
        };
        self.broadcast_room().await?;
        if active_hash != previous_hash {
            if active_hash.is_some() {
                self.relaunch_active_renderers().await;
            } else {
                self.renderer.stop_all();
                self.publish_renderer_snapshot();
            }
        }
        Ok(())
    }

    pub async fn set_local_verified(&self, verified: bool, reason: Option<String>) -> Result<()> {
        let session = self
            .local_session_id
            .read()
            .await
            .clone()
            .context("not connected to a room")?;
        if self.is_host.load(Ordering::Relaxed) {
            self.room
                .write()
                .await
                .set_verified(&session, verified, reason)?;
            self.broadcast_room().await
        } else {
            self.network
                .broadcast(Envelope::new(
                    "chart.status",
                    0,
                    json!({"verified":verified,"reason":reason}),
                ))
                .await;
            Ok(())
        }
    }

    /// Requests the exact current custom chart as an admitted, mismatched
    /// participant. The host still creates an offer and the existing
    /// participant-owned consent checks decide whether any bytes may arrive.
    pub async fn request_chart_transfer(&self, requested_hash: &str) -> Result<()> {
        if self.is_host.load(Ordering::Relaxed) {
            anyhow::bail!("the host already owns the locked chart package");
        }
        let session_id = self
            .local_session_id
            .read()
            .await
            .clone()
            .context("not connected to a room")?;
        let snapshot = self.room.read().await.snapshot.clone();
        let chart = transferable_chart_for_participant(&snapshot, &session_id, requested_hash)?;
        self.network
            .broadcast(Envelope::new(
                "chart.transfer_request",
                0,
                json!({"chartHash":chart.hash}),
            ))
            .await;
        Ok(())
    }

    /// Checks known local paths and BBT-managed imports before presenting the
    /// transfer fallback. A hash is scanned once per lock revision so the
    /// 20 Hz room snapshot stream never causes repeated filesystem work. When
    /// the host opts into automatic requests, a failed scan sends at most one
    /// request for that exact lock.
    async fn try_auto_match_locked_chart(&self) {
        let snapshot = self.room.read().await.snapshot.clone();
        let Some(chart) = snapshot.chart.clone() else {
            return;
        };
        let Some(session_id) = self.local_session_id.read().await.clone() else {
            return;
        };
        let needs_match = snapshot.participants.iter().any(|participant| {
            participant.session_id == session_id
                && participant.role != ParticipantRole::Spectator
                && !participant.verified
        });
        if !needs_match {
            return;
        }

        let already_scanned =
            self.last_auto_match_hash.read().await.as_deref() == Some(chart.hash.as_str());
        if !already_scanned {
            *self.last_auto_match_hash.write().await = Some(chart.hash.clone());
            let mut candidates = Vec::new();
            if let Some(path) = self.chart_paths.read().await.get(&chart.hash).cloned() {
                candidates.push(PathBuf::from(path));
            }
            if let Some(path) = self.selected_chart_path.read().await.clone() {
                candidates.push(PathBuf::from(path));
            }
            let cache = self.data_dir.join("chart-cache");
            if let Ok(entries) = std::fs::read_dir(&cache) {
                for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
                    if std::fs::read_to_string(entry.path().join(".bbt-chart-hash"))
                        .ok()
                        .is_some_and(|value| value.trim() == chart.hash)
                    {
                        candidates.push(entry.path());
                    }
                }
            }
            let expected = chart.hash.clone();
            let hash_cache = self.data_dir.join("chart-hash-index");
            let matched = tokio::task::spawn_blocking(move || {
                candidates.into_iter().find(|candidate| {
                    crate::chart_hash::canonical_chart_hash_cached(candidate, &hash_cache)
                        .ok()
                        .is_some_and(|actual| actual.hash == expected)
                })
            })
            .await
            .ok()
            .flatten();
            if let Some(path) = matched {
                let text = path.to_string_lossy().into_owned();
                *self.selected_chart_path.write().await = Some(text.clone());
                self.chart_paths
                    .write()
                    .await
                    .insert(chart.hash.clone(), text);
                self.network
                    .broadcast(Envelope::new(
                        "chart.status",
                        0,
                        json!({"verified":true,"reason":null}),
                    ))
                    .await;
                let _ = self.events.send(Envelope::new(
                    "chart.verification",
                    0,
                    json!({"verified":true,"hash":chart.hash,"automatic":true}),
                ));
                return;
            }
        }

        let already_requested =
            self.last_auto_request_hash.read().await.as_deref() == Some(chart.hash.as_str());
        if snapshot.auto_request_chart_transfers && !already_requested {
            match self.request_chart_transfer(&chart.hash).await {
                Ok(()) => {
                    *self.last_auto_request_hash.write().await = Some(chart.hash.clone());
                    let _ = self.events.send(Envelope::new(
                        "chart.transfer_requested",
                        0,
                        json!({"chartHash":chart.hash,"automatic":true}),
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, "automatic chart transfer request was not sent");
                }
            }
        }
    }

    pub async fn set_local_ready(&self, ready: bool) -> Result<()> {
        let session = self
            .local_session_id
            .read()
            .await
            .clone()
            .context("not connected to a room")?;
        if self.is_host.load(Ordering::Relaxed) {
            self.room.write().await.set_ready(&session, ready)?;
            self.broadcast_room().await
        } else {
            self.network
                .broadcast(Envelope::new("client.ready", 0, json!({"ready":ready})))
                .await;
            Ok(())
        }
    }

    pub async fn start_room(&self, force: bool) -> Result<u64> {
        self.require_host()?;
        let start = self.room.write().await.schedule_start(force, 5_000)?;
        self.renderer.begin_run();
        self.network
            .broadcast(Envelope::new(
                "room.start_scheduled",
                0,
                json!({
                    "serverStartTimeMs":start,
                    "serverTimeMs":unix_ms(),
                    "force":force
                }),
            ))
            .await;
        self.broadcast_room().await?;
        Ok(start)
    }

    pub async fn close_room(&self) -> Result<()> {
        self.require_host()?;
        self.cancel_reconnect();
        self.cancel_nat_renewal();
        self.release_nat_mapping().await;
        self.room.write().await.close();
        let room_publication = self.broadcast_room().await;
        self.renderer.stop_all();
        self.local_mirror_enabled.store(false, Ordering::Release);
        self.commentator_subscribers.write().await.clear();
        self.commentator_statuses.write().await.clear();
        *self.broadcast_plan.write().await = BroadcastPlan::empty();
        self.pending_transfer_offers.write().await.clear();
        *self.pending_inbound_transfer_offer.write().await = None;
        self.trusted_transfer_rooms.write().await.clear();
        self.active_transfer_builds.write().await.clear();
        *self.last_auto_match_hash.write().await = None;
        *self.last_auto_request_hash.write().await = None;
        let _ = std::fs::remove_dir_all(self.data_dir.join("transfer-outgoing"));
        self.network
            .shutdown_with_reason("Room closed by host")
            .await;
        *self.reconnect_request.write().await = None;
        self.is_host.store(false, Ordering::Relaxed);
        *self.local_session_id.write().await = None;
        *self.connection_status.write().await = "offline".into();
        *self.host_join_address.write().await = None;
        *self.nat_method.write().await = None;
        *self.selected_chart_path.write().await = None;
        self.chart_paths.write().await.clear();
        let runtime_publication = self.publish_runtime_snapshot().await;
        room_publication?;
        runtime_publication
    }

    pub async fn configure_renderer(
        &self,
        slot: &str,
        request: RendererRequest,
    ) -> Result<crate::model::RendererSlot> {
        self.require_host()?;
        // A fresh Beatblock child can reproduce the chart exactly only when it
        // observes the pre-roll and every timed VFX event. Starting or
        // reconfiguring one after the synchronized countdown has begun would
        // collapse earlier eases into a single frame.
        if matches!(
            self.room.read().await.snapshot.lifecycle,
            crate::model::RoomLifecycle::Countdown | crate::model::RoomLifecycle::Playing
        ) {
            anyhow::bail!("configure renderer slots before the synchronized start");
        }
        let restart = request.participant_id.is_some()
            || request.mode.is_some()
            || request.width.is_some()
            || request.height.is_some()
            || request.fps.is_some()
            || request.delay_ms.is_some();
        if let Some(participant_id) = request
            .participant_id
            .as_deref()
            .filter(|participant_id| !participant_id.is_empty())
        {
            let room = self.room.read().await;
            let participant = room
                .snapshot
                .participants
                .iter()
                .find(|participant| participant.session_id == participant_id)
                .context("renderer participant is not in this room")?;
            if !participant.admitted || !participant.connected {
                anyhow::bail!("renderer participant must be admitted and connected");
            }
            if participant.role == ParticipantRole::Spectator {
                anyhow::bail!("renderer slots can only follow active players");
            }
        }
        let configured = self.renderer.configure(slot, request)?;
        if let Some(participant_id) = configured.participant_id.as_deref() {
            self.sync_renderer_player(participant_id).await;
        }
        if configured.active && restart {
            if self.room.read().await.snapshot.chart.is_none() {
                self.renderer
                    .set_error(slot, "assigned; waiting for the host to lock a chart");
            } else if let Err(error) = self.launch_renderer_slot(slot).await {
                self.renderer.set_error(slot, error.to_string());
                return Err(error);
            }
        }
        if self.renderer.autoplay_enabled() {
            if !self.renderer.has_active_featured_slot() {
                self.renderer.disable_autoplay_with_error(
                    "Autoplay Mix was disabled because no featured renderer is active",
                );
            } else if let Err(error) = self.launch_autoplay_renderer().await {
                self.renderer
                    .disable_autoplay_with_error(format!("Autoplay Mix launch failed: {error}"));
                return Err(error);
            }
        }
        self.exports.publish_room(
            self.room.read().await.snapshot.clone(),
            self.renderer.slots(),
        );
        self.refresh_broadcast_plan().await?;
        Ok(configured)
    }

    pub async fn stop_renderer_slot(&self, slot: &str) -> Result<()> {
        self.require_host()?;
        if self.renderer.slot(slot).is_none() {
            anyhow::bail!("unknown renderer slot");
        }
        self.renderer.stop_slot(slot);
        if self.renderer.autoplay_enabled() && !self.renderer.has_active_featured_slot() {
            self.renderer.disable_autoplay_with_error(
                "Autoplay Mix was disabled because the featured renderer stopped",
            );
        }
        self.refresh_broadcast_plan().await?;
        self.publish_renderer_snapshot();
        Ok(())
    }

    async fn stop_participant_renderer(&self, participant_id: &str) -> Result<()> {
        self.renderer.stop_participant(participant_id);
        if self.renderer.autoplay_enabled() && !self.renderer.has_active_featured_slot() {
            self.renderer.disable_autoplay_with_error(
                "Autoplay Mix was disabled because the featured player left the renderer plan",
            );
        }
        // Renderer slots are also the authoritative commentator plan. Publish
        // the cleared assignment immediately so remote OBS mirrors do not keep
        // rendering a participant who left the racing roster.
        self.refresh_broadcast_plan().await?;
        self.publish_renderer_snapshot();
        Ok(())
    }

    pub async fn set_commentator_access(&self, session_id: &str, enabled: bool) -> Result<()> {
        self.require_host()?;
        self.room
            .write()
            .await
            .set_commentator_access(session_id, enabled)?;
        if !enabled {
            self.commentator_subscribers
                .write()
                .await
                .remove(session_id);
            self.commentator_statuses.write().await.remove(session_id);
            let _ = self
                .network
                .send_to(
                    session_id,
                    Envelope::new(
                        "broadcast.revoked",
                        0,
                        json!({"reason":"Commentator access was revoked by the host"}),
                    ),
                )
                .await;
        }
        self.broadcast_room().await?;
        if enabled {
            let plan = self.broadcast_plan.read().await.clone();
            self.network
                .send_to(
                    session_id,
                    Envelope::new("broadcast.plan", plan.revision, json!(plan)),
                )
                .await?;
        }
        self.publish_runtime_snapshot().await
    }

    async fn refresh_broadcast_plan(&self) -> Result<()> {
        let revision = self.broadcast_revision.fetch_add(1, Ordering::AcqRel) + 1;
        let plan = BroadcastPlan::from_slots(
            revision,
            unix_ms(),
            &self.renderer.slots(),
            self.renderer.autoplay_enabled(),
        );
        *self.broadcast_plan.write().await = plan.clone();
        let recipients = self
            .room
            .read()
            .await
            .snapshot
            .participants
            .iter()
            .filter(|participant| {
                participant.connected
                    && participant.admitted
                    && participant.role == ParticipantRole::Spectator
                    && participant.commentator_access
            })
            .map(|participant| participant.session_id.clone())
            .collect::<Vec<_>>();
        for recipient in recipients {
            let _ = self
                .network
                .send_to(
                    &recipient,
                    Envelope::new("broadcast.plan", plan.revision, json!(plan.clone())),
                )
                .await;
        }
        let _ = self
            .events
            .send(Envelope::new("broadcast.plan", plan.revision, json!(plan)));
        self.exports
            .publish_broadcast_metadata(revision, "host", false);
        Ok(())
    }

    pub async fn decide_chart_transfer(
        &self,
        request_id: &str,
        accept: bool,
        trust_room: bool,
        executable_content_confirmed: bool,
    ) -> Result<()> {
        if self.is_host.load(Ordering::Relaxed) {
            anyhow::bail!("only a receiving participant can decide a chart transfer");
        }
        let (host_session, offer) = self
            .pending_inbound_transfer_offer
            .read()
            .await
            .clone()
            .context("there is no chart transfer offer awaiting a decision")?;
        if offer.request_id != request_id {
            anyhow::bail!("chart transfer decision does not match the active offer");
        }
        if accept && offer.contains_executable_content && !executable_content_confirmed {
            anyhow::bail!("script or executable content requires separate confirmation");
        }
        if trust_room && (!accept || offer.contains_executable_content) {
            anyhow::bail!(
                "only an accepted non-executable chart offer can be trusted automatically"
            );
        }
        // Consume the UI offer only after validating the exact request. The
        // network authorization is then an exact, one-use capability checked
        // before the receiver creates a temporary file.
        *self.pending_inbound_transfer_offer.write().await = None;
        if accept {
            self.network
                .authorize_incoming_chart_transfer(
                    &host_session,
                    offer,
                    executable_content_confirmed,
                )
                .await?;
        }
        if trust_room {
            let room_id = self.room.read().await.snapshot.id.clone();
            self.trusted_transfer_rooms.write().await.insert(room_id);
        }
        self.network
            .broadcast(Envelope::new(
                "chart.transfer_decision",
                0,
                json!({
                    "requestId":request_id,
                    "accept":accept,
                    "trustRoom":trust_room,
                    "executableContentConfirmed":executable_content_confirmed
                }),
            ))
            .await;
        Ok(())
    }

    pub async fn set_local_broadcast_mirror(&self, enabled: bool) -> Result<()> {
        if self.is_host.load(Ordering::Relaxed) {
            anyhow::bail!("the host already owns the authoritative broadcast renderers");
        }
        let session_id = self
            .local_session_id
            .read()
            .await
            .clone()
            .context("join a room before enabling the Commentator mirror")?;
        let allowed = self
            .room
            .read()
            .await
            .snapshot
            .participants
            .iter()
            .any(|participant| {
                participant.session_id == session_id
                    && participant.admitted
                    && participant.role == ParticipantRole::Spectator
                    && participant.commentator_access
            });
        if !allowed {
            anyhow::bail!("Commentator access has not been granted by the host");
        }
        self.local_mirror_enabled.store(enabled, Ordering::Release);
        self.exports.publish_broadcast_metadata(
            self.broadcast_plan.read().await.revision,
            "commentator_mirror",
            enabled,
        );
        self.network
            .broadcast(Envelope::new(
                "broadcast.subscribe",
                0,
                json!({"enabled":enabled}),
            ))
            .await;
        if enabled {
            self.apply_broadcast_plan_locally().await;
        } else {
            self.renderer.stop_all();
            self.publish_renderer_snapshot();
        }
        self.publish_mirror_status().await;
        self.publish_runtime_snapshot().await
    }

    async fn apply_broadcast_plan_locally(&self) {
        if !self.local_mirror_enabled.load(Ordering::Acquire) {
            return;
        }
        let plan = self.broadcast_plan.read().await.clone();
        self.renderer.stop_all();
        for slot in plan.slots.into_iter().filter(|slot| slot.active) {
            let request = RendererRequest {
                participant_id: slot.participant_id.clone(),
                participant_name: slot.participant_name.clone(),
                mode: Some(slot.mode),
                width: Some(slot.width),
                height: Some(slot.height),
                fps: Some(slot.fps),
                delay_ms: Some(slot.delay_ms),
                featured: Some(slot.featured),
            };
            match self.renderer.configure(&slot.id, request) {
                Ok(_) if self.selected_chart_path.read().await.is_some() => {
                    if let Err(error) = self.launch_renderer_slot(&slot.id).await {
                        self.renderer.set_error(&slot.id, error.to_string());
                    }
                }
                Ok(_) => self.renderer.set_error(
                    &slot.id,
                    "Host plan received; locate or accept the locked chart to enable video",
                ),
                Err(error) => self.renderer.set_error(&slot.id, error.to_string()),
            }
        }
        if plan.autoplay_audio_enabled == Some(true) {
            if let Err(error) = self.launch_autoplay_renderer().await {
                self.renderer.disable_autoplay_with_error(format!(
                    "Commentator Autoplay Mix could not start: {error}"
                ));
                tracing::warn!(%error, "commentator autoplay renderer failed");
            }
        }
        self.publish_renderer_snapshot();
        self.publish_mirror_status().await;
    }

    async fn publish_mirror_status(&self) {
        if self.is_host.load(Ordering::Relaxed) {
            return;
        }
        let slots = self.renderer.slots();
        let error = slots
            .iter()
            .filter_map(|slot| slot.last_error.as_deref())
            .next()
            .map(|value| value.chars().take(160).collect::<String>())
            .or_else(|| {
                self.renderer
                    .autoplay_state()
                    .error
                    .map(|value| value.chars().take(160).collect::<String>())
            });
        let status = CommentatorMirrorStatus {
            enabled: self.local_mirror_enabled.load(Ordering::Acquire),
            healthy_slots: slots
                .iter()
                .filter(|slot| slot.active && slot.healthy)
                .count() as u32,
            error,
            updated_at_ms: unix_ms(),
        };
        self.network
            .broadcast(Envelope::new("broadcast.mirror_status", 0, json!(status)))
            .await;
    }

    async fn launch_renderer_slot(&self, slot: &str) -> Result<()> {
        let config = self.config.read().await.clone();
        let game_directory = config
            .game_directory
            .context("Beatblock installation path is unavailable")?;
        let game = PathBuf::from(game_directory).join("Beatblock.exe");
        if !game.is_file() {
            anyhow::bail!(
                "Beatblock renderer executable was not found at {}",
                game.display()
            );
        }
        let chart = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .clone()
            .context("renderer is assigned and waiting for a locked chart")?;
        let profile = crate::renderer::prepare_renderer_profile(&self.data_dir)?;
        let chart_path = self
            .selected_chart_path
            .read()
            .await
            .clone()
            .unwrap_or_else(|| chart.package_name.clone());
        self.renderer
            .launch_slot(slot, &game, &profile, &chart_path, &chart.variant)
    }

    async fn launch_autoplay_renderer(&self) -> Result<()> {
        let config = self.config.read().await.clone();
        let game_directory = config
            .game_directory
            .context("Beatblock installation path is unavailable")?;
        let game = PathBuf::from(game_directory).join("Beatblock.exe");
        if !game.is_file() {
            anyhow::bail!(
                "Beatblock autoplay renderer executable was not found at {}",
                game.display()
            );
        }
        let chart = self
            .room
            .read()
            .await
            .snapshot
            .chart
            .clone()
            .context("Autoplay Mix is waiting for the host to lock a chart")?;
        let profile = crate::renderer::prepare_autoplay_profile(&self.data_dir)?;
        let chart_path = self
            .selected_chart_path
            .read()
            .await
            .clone()
            .unwrap_or_else(|| chart.package_name.clone());
        self.renderer
            .launch_autoplay(&game, &profile, &chart_path, &chart.variant)
    }

    pub async fn set_autoplay_audio(&self, enabled: bool) -> Result<()> {
        self.require_host()?;
        if matches!(
            self.room.read().await.snapshot.lifecycle,
            crate::model::RoomLifecycle::Countdown | crate::model::RoomLifecycle::Playing
        ) {
            anyhow::bail!("configure Autoplay Mix before the synchronized start");
        }
        if enabled && !self.renderer.has_active_featured_slot() {
            anyhow::bail!("Autoplay Mix requires an active featured renderer");
        }
        if enabled {
            self.launch_autoplay_renderer().await?;
        } else {
            self.renderer.disable_autoplay();
        }
        self.refresh_broadcast_plan().await?;
        self.publish_renderer_snapshot();
        self.publish_runtime_snapshot().await
    }

    async fn relaunch_active_renderers(&self) {
        for slot in self.renderer.active_slots() {
            if let Err(error) = self.launch_renderer_slot(&slot.id).await {
                self.renderer.set_error(&slot.id, error.to_string());
            }
        }
        if self.renderer.autoplay_enabled() {
            if let Err(error) = self.launch_autoplay_renderer().await {
                self.renderer
                    .disable_autoplay_with_error(format!("Autoplay Mix relaunch failed: {error}"));
                tracing::warn!(%error, "relaunch autoplay renderer failed");
            }
        }
    }

    fn spawn_nat_renewal(&self, port: u16, room_id: String) {
        self.cancel_nat_renewal();
        let state = self.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
                if !state.is_host.load(Ordering::Relaxed)
                    || state.room.read().await.snapshot.id != room_id
                {
                    break;
                }
                if let Ok(mapping) = crate::nat::map_host_port(port).await {
                    *state.mapped_host_port.write().await = Some(port);
                    *state.host_join_address.write().await = Some(mapping.external_address);
                    *state.nat_method.write().await = Some(mapping.method.to_owned());
                    let _ = state.publish_runtime_snapshot().await;
                }
            }
        });
        *self
            .nat_renewal_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(task);
    }

    fn spawn_reconnect(&self) {
        let mut reconnect_task = self
            .reconnect_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if reconnect_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let state = self.clone();
        *reconnect_task = Some(tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            for delay in [1_u64, 2, 4, 7, 7, 7] {
                let delay = std::time::Duration::from_secs(delay);
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining <= delay {
                    break;
                }
                tokio::time::sleep(delay).await;
                if state.connection_status.read().await.as_str() != "reconnecting" {
                    return;
                }
                let Some(request) = state.reconnect_request.read().await.clone() else {
                    return;
                };
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(
                    remaining.min(std::time::Duration::from_secs(5)),
                    request.join(&state.network),
                )
                .await
                {
                    Ok(Ok(session_id)) => {
                        *state.local_session_id.write().await = Some(session_id);
                        *state.connection_status.write().await = "connected".into();
                        let _ = state.publish_runtime_snapshot().await;
                        return;
                    }
                    Ok(Err(error)) => tracing::warn!(%error, "room reconnect attempt failed"),
                    Err(_) => tracing::warn!("room reconnect attempt timed out"),
                }
            }
            state.emit_error("Could not reconnect within the 30-second room grace period".into());
            // Detach our own completed handle before leave_room cancels any
            // externally running reconnect task.
            state
                .reconnect_task
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            let _ = state.leave_room().await;
        }));
    }

    fn cancel_nat_renewal(&self) {
        if let Some(task) = self
            .nat_renewal_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
    }

    fn cancel_reconnect(&self) {
        if let Some(task) = self
            .reconnect_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
    }

    pub fn cancel_background_tasks(&self) {
        self.cancel_reconnect();
        self.cancel_nat_renewal();
    }

    pub async fn release_nat_mapping(&self) {
        let Some(port) = self.mapped_host_port.write().await.take() else {
            return;
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            crate::nat::unmap_host_port(port),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, port, "UPnP mapping cleanup failed"),
            Err(_) => tracing::warn!(port, "UPnP mapping cleanup timed out"),
        }
    }

    /// Clears local ownership after a failed host/join transaction without
    /// attempting another database write that could mask the original error.
    async fn reset_offline_state(&self) {
        self.cancel_background_tasks();
        self.renderer.stop_all();
        self.local_mirror_enabled.store(false, Ordering::Release);
        self.commentator_subscribers.write().await.clear();
        self.commentator_statuses.write().await.clear();
        *self.broadcast_plan.write().await = BroadcastPlan::empty();
        self.pending_transfer_offers.write().await.clear();
        *self.pending_inbound_transfer_offer.write().await = None;
        self.trusted_transfer_rooms.write().await.clear();
        self.active_transfer_builds.write().await.clear();
        self.network.clear_incoming_chart_transfers().await;
        *self.last_auto_match_hash.write().await = None;
        *self.last_auto_request_hash.write().await = None;
        let _ = std::fs::remove_dir_all(self.data_dir.join("transfer-outgoing"));
        self.is_host.store(false, Ordering::Relaxed);
        *self.reconnect_request.write().await = None;
        *self.local_session_id.write().await = None;
        *self.connection_status.write().await = "offline".into();
        *self.host_join_address.write().await = None;
        *self.nat_method.write().await = None;
        *self.mapped_host_port.write().await = None;
        *self.selected_chart_path.write().await = None;
        self.chart_paths.write().await.clear();
        let room = RoomEngine::offline();
        *self.lobby.write().await = serde_json::to_value(&room.snapshot).unwrap_or_default();
        *self.room.write().await = room;
    }

    async fn broadcast_room(&self) -> Result<()> {
        self.sync_room_state().await?;
        let snapshot = self.room.read().await.snapshot.clone();
        let mut payload = serde_json::to_value(snapshot)?;
        if let Some(object) = payload.as_object_mut() {
            object.insert("serverTimeMs".into(), json!(unix_ms()));
        }
        self.network
            .broadcast(Envelope::new("room.snapshot", 0, payload))
            .await;
        Ok(())
    }

    /// Marks high-rate score state for the next 20 Hz publication tick. Score
    /// validation and journaling still happen immediately; only repeated full
    /// room serialization, SQLite snapshots, exports, and peer fan-out coalesce.
    fn mark_room_dirty(&self) {
        self.room_dirty.store(true, Ordering::Release);
    }

    pub async fn flush_room_updates(&self) -> Result<bool> {
        if !self.room_dirty.swap(false, Ordering::AcqRel) {
            return Ok(false);
        }
        if let Err(error) = self.broadcast_room().await {
            self.room_dirty.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(true)
    }

    /// Expires reconnect grace periods as one atomic maintenance operation.
    ///
    /// Keeping room state, transport sessions, and publication together avoids
    /// leaving resumable network tokens behind after the room has removed a
    /// participant.
    pub async fn expire_due_disconnects(&self, now_ms: u64) -> Result<usize> {
        let (expired, expired_unstarted) = {
            let mut room = self.room.write().await;
            let expired = room.expire_due_disconnects(now_ms);
            let expired_unstarted = room.expire_due_unstarted_runs(now_ms);
            (expired, expired_unstarted)
        };
        for session_id in &expired {
            self.network
                .disconnect_peer(session_id, "Reconnect grace period expired", false)
                .await;
        }
        if !expired.is_empty() || expired_unstarted {
            self.broadcast_room().await?;
        }
        Ok(expired.len())
    }

    async fn sync_room_state(&self) -> Result<()> {
        let snapshot = self.room.read().await.snapshot.clone();
        *self.lobby.write().await = serde_json::to_value(&snapshot)?;
        let storage = self.storage.clone();
        let stored_snapshot = snapshot.clone();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::task::spawn_blocking(move || storage.save_room(&stored_snapshot, 120_000)),
        )
        .await
        .context("room storage update timed out")?
        .context("room storage worker stopped")??;
        self.exports
            .publish_room(snapshot.clone(), self.renderer.slots());
        let _ = self.events.send(Envelope::new(
            "room.snapshot",
            0,
            serde_json::to_value(snapshot)?,
        ));
        Ok(())
    }

    fn require_host(&self) -> Result<()> {
        if !self.is_host.load(Ordering::Relaxed) {
            anyhow::bail!("this action is available only to the room host");
        }
        Ok(())
    }

    async fn require_admitted_peer(&self, session_id: &str) -> Result<()> {
        let room = self.room.read().await;
        let participant = room
            .snapshot
            .participants
            .iter()
            .find(|participant| participant.session_id == session_id)
            .context("network peer is not in the room roster")?;
        if !participant.admitted || !participant.connected {
            anyhow::bail!("network peer is awaiting host admission");
        }
        Ok(())
    }

    pub fn require_host_control(&self) -> Result<()> {
        self.require_host()
    }

    pub async fn publish_room(&self) -> Result<()> {
        self.broadcast_room().await
    }

    pub async fn publish_runtime_snapshot(&self) -> Result<()> {
        let chart_cache_bytes = crate::transfer::cache_size(&self.data_dir.join("chart-cache"));
        let client = self.client.read().await.clone();
        let _ = self.events.send(Envelope::new(
            "runtime.snapshot",
            0,
            json!({
                "connection":self.connection_status.read().await.clone(),
                "joinAddress":self.host_join_address.read().await.clone(),
                "natMethod":self.nat_method.read().await.clone(),
                "hosting":self.is_host.load(Ordering::Relaxed),
                "sessionId":self.local_session_id.read().await.clone(),
                "room":self.room.read().await.snapshot.clone(),
                "renderers":self.renderer.slots(),
                "autoplayAudio":self.renderer.autoplay_state(),
                "broadcastPlan":self.broadcast_plan.read().await.clone(),
                "commentatorStatuses":self.commentator_statuses.read().await.clone(),
                "mirrorEnabled":self.local_mirror_enabled.load(Ordering::Acquire),
                "broadcastAuthority":if self.is_host.load(Ordering::Relaxed) {
                    "host"
                } else {
                    "commentator_mirror"
                },
                "chartCacheBytes":chart_cache_bytes,
                "chartCacheSizeLabel":format!("{:.1} MB / 2 GB", chart_cache_bytes as f64 / 1_048_576.0),
                "history":self.storage.history()?,
                "settings":self.config.read().await.clone(),
                "diagnostics":{
                    "protocolVersion":crate::model::PROTOCOL_VERSION,
                    "runtimeVersion":env!("CARGO_PKG_VERSION"),
                    "testedBeatblockVersion":crate::compatibility::TESTED_BEATBLOCK_VERSION,
                    "testedBeatblockBuildId":crate::compatibility::TESTED_BEATBLOCK_BUILD_ID,
                    "detectedBeatblockVersion":client.get("gameVersion").cloned().unwrap_or(Value::Null),
                    "detectedBeatblockBuildId":client.get("gameBuildId").cloned().unwrap_or(Value::Null),
                    "detectedBeatblockBuildSource":client.get("gameBuildSource").cloned().unwrap_or(Value::Null),
                    "peerCount":self.network.peer_count().await,
                    "rendererBudgetWarning":self.renderer.budget_warning(),
                    "rendererDesktopMute":self.renderer.desktop_mute_enabled(),
                    "firewallInstalled":self.config.read().await.firewall_installed,
                    "firewallPublic":self.config.read().await.firewall_public
                },
            }),
        ));
        Ok(())
    }

    pub fn publish_renderer_snapshot(&self) {
        let _ = self.events.send(Envelope::new(
            "renderer.snapshot",
            0,
            json!({
                "renderers": self.renderer.slots(),
                "autoplayAudio": self.renderer.autoplay_state(),
                "budgetWarning": self.renderer.budget_warning(),
            }),
        ));
    }

    fn validate(&self, message: &Envelope) -> Result<()> {
        if message.version != PROTOCOL_VERSION {
            anyhow::bail!("unsupported protocol version {}", message.version);
        }
        if message.kind.is_empty() {
            anyhow::bail!("message type is required");
        }
        Ok(())
    }

    fn emit_error(&self, message: String) {
        let _ = self.events.send(Envelope::new(
            "runtime.error",
            0,
            json!({"message":message}),
        ));
    }

    fn append_journal(&self, message: &Envelope) -> Result<()> {
        self.journals.publish(message)
    }

    pub fn journal_events(&self) -> Vec<Envelope> {
        self.journals.flush();
        let directory = self.data_dir.join("journals");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ndjson"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .flat_map(|path| {
                std::fs::read_to_string(path)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}
