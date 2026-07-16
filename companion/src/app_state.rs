use crate::{
    exports::{write_exports, write_room_exports},
    game_commands,
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
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{broadcast, mpsc, RwLock};

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
    pub storage: Arc<Storage>,
    pub data_dir: Arc<PathBuf>,
    pub is_host: Arc<AtomicBool>,
    pub local_session_id: Arc<RwLock<Option<String>>>,
    pub connection_status: Arc<RwLock<String>>,
    pub shutdown_requested: Arc<AtomicBool>,
    pub selected_chart_path: Arc<RwLock<Option<String>>>,
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
        let recovered = storage.recover_room(unix_ms())?;
        let mut room = RoomEngine::offline();
        if let Some(snapshot) = recovered {
            room.snapshot = snapshot;
        }
        let lobby = serde_json::to_value(&room.snapshot)?;
        let (events, _) = broadcast::channel(8192);
        let (network_events_tx, network_events_rx) = mpsc::channel(8192);
        let network = Arc::new(NetworkHub::new(network_events_tx));
        let renderer = Arc::new(RendererManager::new(data_dir.clone())?);
        Ok((
            Self {
                local_token: Arc::new(std::sync::RwLock::new(local_token)),
                gameplay: Arc::new(RwLock::new(GameplayState::default())),
                room: Arc::new(RwLock::new(room)),
                lobby: Arc::new(RwLock::new(lobby)),
                config: Arc::new(RwLock::new(config)),
                client: Arc::new(RwLock::new(json!({
                    "clientVersion":"0.3.0-alpha.1",
                    "gameBuildHash":"unknown",
                    "distribution":"standalone",
                    "mods":[]
                }))),
                events,
                network,
                renderer,
                storage,
                data_dir: Arc::new(data_dir),
                is_host: Arc::new(AtomicBool::new(false)),
                local_session_id: Arc::new(RwLock::new(None)),
                connection_status: Arc::new(RwLock::new("offline".into())),
                shutdown_requested: Arc::new(AtomicBool::new(false)),
                selected_chart_path: Arc::new(RwLock::new(None)),
            },
            network_events_rx,
        ))
    }

    pub async fn run_network_events(&self, mut receiver: mpsc::Receiver<NetworkEvent>) {
        while let Some(event) = receiver.recv().await {
            if let Err(error) = self.handle_network_event(event).await {
                tracing::warn!(%error, "network event rejected");
                self.emit_error(error.to_string());
            }
        }
    }

    pub async fn ingest(&self, message: Envelope) -> Result<()> {
        self.apply_local(message).await
    }

    pub async fn ingest_remote(&self, message: Envelope) -> Result<()> {
        self.apply_host_message(message).await
    }

    async fn apply_local(&self, message: Envelope) -> Result<()> {
        self.validate(&message)?;
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
                    self.broadcast_room().await?;
                } else {
                    self.network.broadcast(message.clone()).await;
                }
            }
        } else if matches!(
            message.kind.as_str(),
            "client.hello" | "input.tap" | "render.keyframe" | "chart.status"
        ) {
            if !self.is_host.load(Ordering::Relaxed) {
                self.network.broadcast(message.clone()).await;
            }
        }
        let _ = self.events.send(message);
        Ok(())
    }

    async fn apply_host_message(&self, message: Envelope) -> Result<()> {
        self.validate(&message)?;
        if message.kind == "room.snapshot" || message.kind == "lobby.snapshot" {
            let snapshot: RoomSnapshot = serde_json::from_value(message.payload.clone())?;
            *self.lobby.write().await = serde_json::to_value(&snapshot)?;
            self.room.write().await.snapshot = snapshot;
            write_room_exports(
                &self.data_dir.join("exports"),
                &self.room.read().await.snapshot,
                &self.renderer.slots(),
            )?;
        } else if message.kind == "room.start_scheduled" || message.kind == "lobby.start_scheduled"
        {
            let _ = self.events.send(message.clone());
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
                write_exports(&self.data_dir.join("exports"), &next)?;
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
                    self.room.write().await.request_join_with_id(
                        session_id.clone(),
                        &display_name,
                        role,
                    )?;
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
                    self.apply_common(&envelope).await?;
                    if envelope.kind.starts_with("run.") {
                        let room_id = self.room.read().await.snapshot.id.clone();
                        self.storage.append_event(&room_id, &envelope)?;
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
                    self.broadcast_room().await?;
                } else {
                    self.apply_host_message(envelope).await?;
                }
            }
            NetworkEvent::RenderSample { session_id, sample } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.renderer.push_sample(&session_id, sample);
                }
            }
            NetworkEvent::Disconnected { session_id, reason } => {
                if self.is_host.load(Ordering::Relaxed) {
                    self.room.write().await.disconnect(&session_id);
                    self.broadcast_room().await?;
                } else {
                    *self.connection_status.write().await = "reconnecting".into();
                }
                self.emit_error(reason);
            }
            NetworkEvent::Error(reason) => self.emit_error(reason),
        }
        Ok(())
    }

    async fn apply_run_to_room(&self, session_id: &str, message: &Envelope) -> Result<()> {
        let mut room = self.room.write().await;
        match message.kind.as_str() {
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
        let host_name = self.config.read().await.display_name.clone();
        let room = RoomEngine::host(room_name, host_name, admission_mode);
        let session_id = room.snapshot.host_session_id.clone();
        let address = self.network.start_host(port, password).await?;
        *self.room.write().await = room;
        *self.local_session_id.write().await = Some(session_id);
        self.is_host.store(true, Ordering::Relaxed);
        *self.connection_status.write().await = "hosting".into();
        self.sync_room_state().await?;
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

    pub async fn join_room(
        &self,
        address: SocketAddr,
        password: &str,
        display_name: &str,
        role: ParticipantRole,
    ) -> Result<String> {
        self.is_host.store(false, Ordering::Relaxed);
        *self.connection_status.write().await = "connecting".into();
        let session = self
            .network
            .join(address, password, display_name, role)
            .await?;
        *self.local_session_id.write().await = Some(session.clone());
        Ok(session)
    }

    pub async fn admit(&self, session_id: &str, admit: bool, role: ParticipantRole) -> Result<()> {
        self.require_host()?;
        self.room.write().await.admit(session_id, admit, role)?;
        self.broadcast_room().await
    }

    pub async fn lock_chart(&self, chart: ChartLock, append_to_setlist: bool) -> Result<()> {
        self.require_host()?;
        self.room
            .write()
            .await
            .lock_chart(chart, append_to_setlist)?;
        self.broadcast_room().await
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
                json!({"serverStartTimeMs":start,"force":force}),
            ))
            .await;
        self.broadcast_room().await?;
        Ok(start)
    }

    pub async fn close_room(&self) -> Result<()> {
        self.require_host()?;
        self.room.write().await.close();
        self.broadcast_room().await?;
        self.network.shutdown().await;
        *self.connection_status.write().await = "offline".into();
        Ok(())
    }

    pub async fn configure_renderer(
        &self,
        slot: &str,
        request: RendererRequest,
    ) -> Result<crate::model::RendererSlot> {
        self.require_host()?;
        let configured = self.renderer.configure(slot, request)?;
        if configured.active {
            let config = self.config.read().await.clone();
            let chart = self.room.read().await.snapshot.chart.clone();
            if let (Some(game_directory), Some(chart)) = (config.game_directory, chart) {
                let game = PathBuf::from(game_directory).join("Beatblock.exe");
                let profile = crate::renderer::prepare_renderer_profile(&self.data_dir)?;
                let chart_path = self
                    .selected_chart_path
                    .read()
                    .await
                    .clone()
                    .unwrap_or_else(|| chart.package_name.clone());
                self.renderer
                    .launch_slot(slot, &game, &profile, &chart_path, &chart.variant)?;
            }
        }
        write_room_exports(
            &self.data_dir.join("exports"),
            &self.room.read().await.snapshot,
            &self.renderer.slots(),
        )?;
        Ok(configured)
    }

    async fn broadcast_room(&self) -> Result<()> {
        self.sync_room_state().await?;
        let snapshot = self.room.read().await.snapshot.clone();
        self.network
            .broadcast(Envelope::new(
                "room.snapshot",
                0,
                serde_json::to_value(snapshot)?,
            ))
            .await;
        Ok(())
    }

    async fn sync_room_state(&self) -> Result<()> {
        let snapshot = self.room.read().await.snapshot.clone();
        *self.lobby.write().await = serde_json::to_value(&snapshot)?;
        self.storage.save_room(&snapshot, 120_000)?;
        write_room_exports(
            &self.data_dir.join("exports"),
            &snapshot,
            &self.renderer.slots(),
        )?;
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

    pub fn require_host_control(&self) -> Result<()> {
        self.require_host()
    }

    pub async fn publish_room(&self) -> Result<()> {
        self.broadcast_room().await
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
        let directory = self.data_dir.join("journals");
        std::fs::create_dir_all(&directory)?;
        let run_id = message
            .run_id
            .as_deref()
            .or_else(|| message.payload.get("runId").and_then(Value::as_str))
            .unwrap_or("unassigned");
        let safe_id: String = run_id
            .chars()
            .filter(|value| value.is_ascii_alphanumeric() || *value == '-' || *value == '_')
            .take(96)
            .collect();
        let path = directory.join(format!(
            "{}.ndjson",
            if safe_id.is_empty() {
                "unassigned"
            } else {
                &safe_id
            }
        ));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        serde_json::to_writer(&mut file, message)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    pub fn journal_events(&self) -> Vec<Envelope> {
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
