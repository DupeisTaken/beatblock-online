use crate::{
    exports::write_exports,
    model::{CompanionConfig, Envelope, GameplayState},
};
use anyhow::Result;
use serde_json::Value;
use std::{io::Write, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, RwLock};

#[derive(Clone)]
pub struct AppState {
    pub local_token: Arc<String>,
    pub gameplay: Arc<RwLock<GameplayState>>,
    pub lobby: Arc<RwLock<Value>>,
    pub config: Arc<RwLock<CompanionConfig>>,
    pub client: Arc<RwLock<Value>>,
    pub events: broadcast::Sender<Envelope>,
    pub remote_tx: mpsc::Sender<Envelope>,
    pub data_dir: Arc<PathBuf>,
}

impl AppState {
    pub async fn ingest(&self, message: Envelope) -> Result<()> {
        self.apply(message, true).await
    }

    pub async fn ingest_remote(&self, message: Envelope) -> Result<()> {
        self.apply(message, false).await
    }

    async fn apply(&self, message: Envelope, forward_remote: bool) -> Result<()> {
        if message.version != 1 {
            anyhow::bail!("unsupported protocol version {}", message.version);
        }
        if forward_remote && crate::game_commands::handle(self, &message).await? {
            return Ok(());
        }
        match message.kind.as_str() {
            "gameplay.snapshot" => {
                if let Ok(next) = serde_json::from_value::<GameplayState>(message.payload.clone()) {
                    *self.gameplay.write().await = next.clone();
                    write_exports(&self.data_dir.join("exports"), &next)?;
                }
            }
            "lobby.snapshot" => {
                *self.lobby.write().await = message.payload.clone();
            }
            "client.hello" => {
                *self.client.write().await = message.payload.clone();
            }
            _ => {}
        }
        let competitive = message.payload.get("lobbyId").and_then(Value::as_str) != Some("offline");
        if forward_remote && competitive && message.kind.starts_with("run.") {
            self.append_journal(&message)?;
        }
        let _ = self.events.send(message.clone());
        if forward_remote
            && competitive
            && (message.kind == "client.hello"
                || message.kind.starts_with("run.")
                || message.kind.starts_with("lobby."))
            && self.remote_tx.try_send(message).is_err()
        {
            tracing::warn!("remote queue is full; event remains available in the local journal");
        }
        Ok(())
    }

    fn append_journal(&self, message: &Envelope) -> Result<()> {
        let directory = self.data_dir.join("journals");
        std::fs::create_dir_all(&directory)?;
        let run_id = message
            .payload
            .get("runId")
            .and_then(Value::as_str)
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
