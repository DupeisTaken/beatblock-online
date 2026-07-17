use crate::{
    exports::ExportPublisher,
    game_commands,
    journal::JournalPublisher,
    model::{
        AdmissionMode, ChartLock, CompanionConfig, Envelope, GameplayState, ParticipantRole,
        RendererRequest, RoomSnapshot, PROTOCOL_VERSION,
    },
    network::{NetworkEvent, NetworkHub},
    renderer::RendererManager,
    room::{unix_ms, RoomEngine},
    storage::Storage,
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{broadcast, mpsc, RwLock};

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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bbt-app-{label}-{}", rand::random::<u64>()))
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
        let (events, _) = broadcast::channel(8192);
        let (network_events_tx, network_events_rx) = mpsc::channel(8192);
        let network = Arc::new(NetworkHub::new(network_events_tx));
        let renderer = Arc::new(RendererManager::new(data_dir.clone())?);
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
                    "clientVersion":"0.3.0-alpha.2",
                    "gameBuildHash":"unknown",
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
        self.apply_host_message(message).await
    }

    async fn apply_local(&self, message: Envelope) -> Result<()> {
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
            "client.hello" | "input.tap" | "render.keyframe" | "chart.status"
        ) && !self.is_host.load(Ordering::Relaxed)
        {
            self.network.broadcast(message.clone()).await;
        }
        let _ = self.events.send(message);
        Ok(())
    }

    async fn apply_host_message(&self, mut message: Envelope) -> Result<()> {
        localize_host_schedule(&mut message, unix_ms());
        self.validate(&message)?;
        if message.kind == "room.snapshot" || message.kind == "lobby.snapshot" {
            let snapshot: RoomSnapshot = serde_json::from_value(message.payload.clone())?;
            *self.lobby.write().await = serde_json::to_value(&snapshot)?;
            self.room.write().await.snapshot = snapshot;
            self.exports.publish_room(
                self.room.read().await.snapshot.clone(),
                self.renderer.slots(),
            );
        }
        self.apply_common(&message).await?;
        let _ = self.events.send(message);
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
                    self.require_admitted_peer(&session_id).await?;
                    self.validate(&envelope)?;
                    self.apply_common(&envelope).await?;
                    let coalesce_room = envelope.kind.starts_with("run.");
                    if coalesce_room {
                        let room_id = self.room.read().await.snapshot.id.clone();
                        self.storage.queue_event(&room_id, &envelope)?;
                        self.apply_run_to_room(&session_id, &envelope).await?;
                        self.sync_renderer_player(&session_id).await;
                    } else if envelope.kind == "client.ready" {
                        let ready = envelope
                            .payload
                            .get("ready")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        self.room.write().await.set_ready(&session_id, ready)?;
                    } else if envelope.kind == "chart.status" {
                        let verified = envelope
                            .payload
                            .get("verified")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let reason = envelope
                            .payload
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        self.room
                            .write()
                            .await
                            .set_verified(&session_id, verified, reason)?;
                    }
                    let _ = self.events.send(envelope);
                    if coalesce_room {
                        self.mark_room_dirty();
                    } else {
                        self.broadcast_room().await?;
                    }
                } else {
                    if envelope.kind == "room.removed" {
                        let reason = envelope
                            .payload
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("Removed from the room")
                            .to_owned();
                        self.leave_room().await?;
                        self.emit_error(reason);
                    } else {
                        self.apply_host_message(envelope).await?;
                    }
                }
            }
            NetworkEvent::RenderSample { session_id, sample } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.require_admitted_peer(&session_id).await?;
                    self.renderer.push_sample(&session_id, sample);
                }
            }
            NetworkEvent::Disconnected { session_id, reason } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.room.write().await.disconnect(&session_id);
                    self.renderer.stop_participant(&session_id);
                    self.broadcast_room().await?;
                } else {
                    let normalized_reason = reason.to_ascii_lowercase();
                    let terminal_disconnect = normalized_reason.contains("rejected")
                        || normalized_reason.contains("removed from the room")
                        || normalized_reason.contains("room closed")
                        || normalized_reason.contains("runtime stopped");
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

    async fn apply_run_to_room(&self, session_id: &str, message: &Envelope) -> Result<()> {
        let mut room = self.room.write().await;
        match message.kind.as_str() {
            "run.started" => {
                let max_hits = message
                    .payload
                    .get("maxHits")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                room.start_run(session_id, max_hits)?;
            }
            "run.score_delta" | "score.mutation" => {
                let sequence = message
                    .payload
                    .get("runSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(message.sequence);
                room.ingest_score(session_id, sequence, &message.payload)?;
            }
            "run.invalid" => {
                room.invalidate(
                    session_id,
                    message
                        .payload
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("Run invalidated")
                        .into(),
                    message
                        .payload
                        .get("dnf")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )?;
            }
            "run.finished" => {
                let run_id = message
                    .run_id
                    .as_deref()
                    .or_else(|| message.payload.get("runId").and_then(Value::as_str))
                    .unwrap_or("unassigned");
                room.finish_run(session_id, run_id)?;
            }
            _ => {}
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

    pub async fn host_room(
        &self,
        room_name: String,
        password: String,
        port: u16,
        admission_mode: AdmissionMode,
    ) -> Result<SocketAddr> {
        self.cancel_reconnect();
        self.cancel_nat_renewal();
        self.release_nat_mapping().await;
        *self.reconnect_request.write().await = None;
        *self.connection_status.write().await = "starting".into();
        let host_name = self.config.read().await.display_name.clone();
        let room = RoomEngine::host(room_name, host_name, admission_mode);
        let session_id = room.snapshot.host_session_id.clone();
        let local_address = match self.network.start_host(port, password).await {
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
        config.display_name = display_name;
        config.requested_role = role;
        if let Some(address) = address {
            config.host_address = address.ip().to_string();
            config.host_port = address.port();
        }
        let bytes = serde_json::to_vec_pretty(&*config)?;
        std::fs::write(self.data_dir.join("config.json"), bytes)?;
        Ok(())
    }

    /// Hosting changes the preferred display name and UDP port without
    /// replacing the user's last remote host address with a loopback address.
    pub async fn save_host_profile(&self, display_name: String, port: u16) -> Result<()> {
        let mut config = self.config.write().await;
        config.display_name = display_name;
        config.requested_role = ParticipantRole::Host;
        config.host_port = port;
        let bytes = serde_json::to_vec_pretty(&*config)?;
        std::fs::write(self.data_dir.join("config.json"), bytes)?;
        Ok(())
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
        let session = match self
            .network
            .join(address, password, display_name, role)
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
            self.renderer.stop_participant(session_id);
            self.network
                .disconnect_peer(session_id, "Join request rejected by host", false)
                .await;
        } else {
            self.room.write().await.admit(session_id, true, role)?;
            if role == ParticipantRole::Spectator {
                self.renderer.stop_participant(session_id);
            }
        }
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
        self.renderer.stop_participant(session_id);
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
        if active_hash != previous_hash {
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
        self.exports.publish_room(
            self.room.read().await.snapshot.clone(),
            self.renderer.slots(),
        );
        Ok(configured)
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

    async fn relaunch_active_renderers(&self) {
        for slot in self.renderer.active_slots() {
            if let Err(error) = self.launch_renderer_slot(&slot.id).await {
                self.renderer.set_error(&slot.id, error.to_string());
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
                    state.network.join(
                        request.address,
                        &request.password,
                        &request.display_name,
                        request.role,
                    ),
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
                "history":self.storage.history()?,
                "settings":self.config.read().await.clone(),
                "diagnostics":{
                    "protocolVersion":crate::model::PROTOCOL_VERSION,
                    "runtimeVersion":env!("CARGO_PKG_VERSION"),
                    "peerCount":self.network.peer_count().await,
                    "rendererBudgetWarning":self.renderer.budget_warning(),
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
