use beatblock_together_companion::{
    app_state::AppState,
    game_commands,
    model::{CompanionConfig, Envelope, GameplayState},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, RwLock};

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bbt-{name}-{}", rand::random::<u64>()))
}

fn state(root: PathBuf, chart_hash: &str) -> AppState {
    let (events, _) = broadcast::channel(32);
    let (remote_tx, _remote_rx) = mpsc::channel(8);
    AppState {
        local_token: Arc::new("test-token".into()),
        gameplay: Arc::new(RwLock::new(GameplayState::default())),
        lobby: Arc::new(RwLock::new(json!({
            "id":"lobby-1",
            "chart":{"hash":chart_hash}
        }))),
        config: Arc::new(RwLock::new(CompanionConfig::default())),
        client: Arc::new(RwLock::new(json!({}))),
        events,
        remote_tx,
        data_dir: Arc::new(root),
    }
}

fn command(path: &str) -> Envelope {
    Envelope {
        version: 1,
        kind: "lobby.chart_verify_request".into(),
        sequence: 1,
        timestamp_ms: 1,
        payload: json!({
            "path": path,
            "levelPath": "Custom Levels/Test/",
            "variant": "Hard"
        }),
    }
}

#[tokio::test]
async fn game_chart_command_returns_a_positive_verification_to_lua() {
    let root = temporary("game-command");
    let chart = root.join("chart");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("manifest.json"), b"competition chart").unwrap();
    let canonical = beatblock_together_companion::chart_hash::canonical_chart_hash(&chart).unwrap();
    let app = state(root.clone(), &canonical.hash);
    let mut events = app.events.subscribe();
    assert!(
        game_commands::handle(&app, &command(chart.to_str().unwrap()))
            .await
            .unwrap()
    );
    let response = events.recv().await.unwrap();
    assert_eq!(response.kind, "chart.verification");
    assert_eq!(response.payload["verified"], true);
    assert_eq!(response.payload["levelPath"], "Custom Levels/Test/");
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn game_chart_command_explains_a_package_mismatch() {
    let root = temporary("game-command-mismatch");
    let chart = root.join("chart");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("manifest.json"), b"altered chart").unwrap();
    let app = state(root.clone(), &"f".repeat(64));
    let mut events = app.events.subscribe();
    game_commands::handle(&app, &command(chart.to_str().unwrap()))
        .await
        .unwrap();
    let response = events.recv().await.unwrap();
    assert_eq!(response.kind, "chart.verification");
    assert_eq!(response.payload["verified"], false);
    assert!(response.payload["reason"]
        .as_str()
        .unwrap()
        .contains("does not match"));
    let _ = std::fs::remove_dir_all(root);
}
