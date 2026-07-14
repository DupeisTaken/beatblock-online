use anyhow::{Context, Result};
use beatblock_together_companion::{
    app_state::AppState,
    http, ipc,
    model::{CompanionConfig, GameplayState},
    remote, tray,
};
use clap::Parser;
use directories::ProjectDirs;
use rand::RngCore;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, RwLock};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(long, default_value_t = 8974)]
    port: u16,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long, default_value = "../web/dist")]
    web_dir: PathBuf,
    #[arg(long)]
    no_tray: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("beatblock_together_companion=info".parse()?),
        )
        .init();
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(|| {
        ProjectDirs::from("org", "BeatblockTogether", "BeatblockTogether")
            .map(|dirs| dirs.data_local_dir().to_owned())
            .unwrap_or_else(|| PathBuf::from("companion-data"))
    });
    std::fs::create_dir_all(data_dir.join("exports"))?;
    let token_path = data_dir.join("local-token.txt");
    let local_token = if token_path.exists() {
        std::fs::read_to_string(&token_path)?.trim().to_owned()
    } else {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        std::fs::write(&token_path, &token)?;
        token
    };
    let config = if data_dir.join("config.json").exists() {
        serde_json::from_slice(&std::fs::read(data_dir.join("config.json"))?).unwrap_or_default()
    } else {
        CompanionConfig::default()
    };
    let (event_tx, _) = broadcast::channel(2048);
    let (remote_tx, remote_rx) = mpsc::channel(4096);
    let state = AppState {
        local_token: Arc::new(local_token.clone()),
        gameplay: Arc::new(RwLock::new(GameplayState::default())),
        lobby: Arc::new(RwLock::new(
            json!({ "id":"offline", "code":"LOCAL", "name":"Offline practice", "lifecycle":"forming", "players":[] }),
        )),
        config: Arc::new(RwLock::new(config)),
        client: Arc::new(RwLock::new(
            json!({ "clientVersion":"0.1.0-alpha.1", "gameBuildHash":"unknown", "distribution":"standalone", "mods":[] }),
        )),
        events: event_tx,
        remote_tx,
        data_dir: Arc::new(data_dir.clone()),
    };
    let console_url = format!("http://127.0.0.1:{}/?token={}", args.port, local_token);
    tracing::info!(%console_url, "local broadcast console ready");
    if !args.no_tray {
        tray::run(console_url.clone(), data_dir.join("exports"));
    }
    tokio::spawn(ipc::run_tcp(state.clone()));
    #[cfg(windows)]
    tokio::spawn(ipc::run_named_pipe(state.clone()));
    tokio::spawn(remote::run(state.clone(), remote_rx));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port))
        .await
        .context("bind local HTTP API")?;
    axum::serve(listener, http::router(state, args.web_dir)).await?;
    Ok(())
}
