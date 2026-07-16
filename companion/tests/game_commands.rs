use beatblock_together_companion::{
    app_state::AppState,
    game_commands,
    model::{
        AdmissionMode, ChartLock, ChartTransferMode, CompanionConfig, Envelope, ParticipantRole,
        RendererRequest,
    },
    room::RoomEngine,
};
use serde_json::json;
use std::{path::PathBuf, sync::atomic::Ordering};

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bbt-{name}-{}", rand::random::<u64>()))
}

async fn state(root: PathBuf, chart_hash: &str) -> AppState {
    let app = AppState::new(root, "test-token".into(), CompanionConfig::default())
        .unwrap()
        .0;
    let mut room = RoomEngine::host("Test".into(), "Host".into(), AdmissionMode::PasswordOnly);
    room.snapshot.chart = Some(ChartLock {
        hash: chart_hash.into(),
        package_name: "chart".into(),
        song_name: "Test".into(),
        variant: "Hard".into(),
        expected_max_hits: 1,
        official: false,
        transfer_mode: ChartTransferMode::VerifyOnly,
    });
    let session = room.snapshot.host_session_id.clone();
    *app.room.write().await = room;
    *app.local_session_id.write().await = Some(session);
    app.is_host.store(true, Ordering::Relaxed);
    app
}

fn command(path: &str) -> Envelope {
    chart_command("lobby.chart_verify_request", path, "Hard", 1)
}

fn chart_command(kind: &str, path: &str, variant: &str, expected_max_hits: u64) -> Envelope {
    Envelope::new(
        kind,
        1,
        json!({
            "path": path,
            "levelPath": "Custom Levels/Test/",
            "variant": variant,
            "expectedMaxHits": expected_max_hits
        }),
    )
}

#[tokio::test]
async fn game_chart_command_returns_a_positive_verification_to_lua() {
    let root = temporary("game-command");
    let chart = root.join("chart");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("manifest.json"), b"competition chart").unwrap();
    let canonical = beatblock_together_companion::chart_hash::canonical_chart_hash(&chart).unwrap();
    let app = state(root.clone(), &canonical.hash).await;
    let mut events = app.events.subscribe();
    assert!(
        game_commands::handle(&app, &command(chart.to_str().unwrap()))
            .await
            .unwrap()
    );
    let response = loop {
        let event = events.recv().await.unwrap();
        if event.kind == "chart.verification" {
            break event;
        }
    };
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
    let app = state(root.clone(), &"f".repeat(64)).await;
    let mut events = app.events.subscribe();
    game_commands::handle(&app, &command(chart.to_str().unwrap()))
        .await
        .unwrap();
    let response = loop {
        let event = events.recv().await.unwrap();
        if event.kind == "chart.verification" {
            break event;
        }
    };
    assert_eq!(response.kind, "chart.verification");
    assert_eq!(response.payload["verified"], false);
    assert!(response.payload["reason"]
        .as_str()
        .unwrap()
        .contains("does not match"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn game_chart_command_rejects_variant_and_note_count_mismatches() {
    let root = temporary("game-command-variant");
    let chart = root.join("chart");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("manifest.json"), b"competition chart").unwrap();
    let canonical = beatblock_together_companion::chart_hash::canonical_chart_hash(&chart).unwrap();
    let app = state(root.clone(), &canonical.hash).await;

    for (variant, max_hits, expected_fragment) in
        [("Expert", 1, "variant"), ("Hard", 2, "note count")]
    {
        let mut events = app.events.subscribe();
        game_commands::handle(
            &app,
            &chart_command(
                "lobby.chart_verify_request",
                chart.to_str().unwrap(),
                variant,
                max_hits,
            ),
        )
        .await
        .unwrap();
        let response = loop {
            let event = events.recv().await.unwrap();
            if event.kind == "chart.verification" {
                break event;
            }
        };
        assert_eq!(response.payload["verified"], false);
        assert!(response.payload["reason"]
            .as_str()
            .unwrap()
            .contains(expected_fragment));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn changing_the_host_custom_chart_verifies_against_the_new_lock() {
    let root = temporary("game-command-change-lock");
    let chart = root.join("chart");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("manifest.json"), b"new competition chart").unwrap();
    let app = state(root.clone(), &"f".repeat(64)).await;
    let mut events = app.events.subscribe();

    game_commands::handle(
        &app,
        &chart_command(
            "room.chart_select_request",
            chart.to_str().unwrap(),
            "Hard",
            1,
        ),
    )
    .await
    .unwrap();
    let response = loop {
        let event = events.recv().await.unwrap();
        if event.kind == "chart.verification" {
            break event;
        }
    };
    assert_eq!(response.payload["verified"], true);
    let locked = app.room.read().await.snapshot.chart.clone().unwrap();
    assert_ne!(locked.hash, "f".repeat(64));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn protocol_v1_is_rejected_instead_of_silently_downgraded() {
    let root = temporary("protocol-v1-rejected");
    let app = AppState::new(
        root.clone(),
        "test-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    let mut legacy = Envelope::new("diagnostics.get", 1, json!({"requestId":"legacy"}));
    legacy.version = 1;
    let error = app.ingest(legacy).await.unwrap_err().to_string();
    assert!(error.contains("unsupported protocol version 1"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn control_requests_are_correlated_and_snapshots_do_not_leak_tokens() {
    let root = temporary("control-ack");
    let app = AppState::new(
        root.clone(),
        "top-secret-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    let mut events = app.events.subscribe();
    let command = Envelope::new("diagnostics.get", 2, json!({"requestId":"req-42"}));
    game_commands::handle(&app, &command).await.unwrap();
    let mut ack = None;
    let mut snapshot = None;
    for _ in 0..4 {
        let event = events.recv().await.unwrap();
        if event.kind == "control.ack" {
            ack = Some(event.clone());
        }
        if event.kind == "runtime.snapshot" {
            snapshot = Some(event);
        }
        if ack.is_some() && snapshot.is_some() {
            break;
        }
    }
    assert_eq!(ack.unwrap().payload["requestId"], "req-42");
    let encoded = serde_json::to_string(&snapshot.unwrap()).unwrap();
    assert!(!encoded.contains("top-secret-token"));
    assert!(!encoded.contains("password"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn renderer_rejects_spectators_as_video_targets() {
    let root = temporary("renderer-spectator");
    let app = state(root.clone(), &"a".repeat(64)).await;
    let spectator = app
        .room
        .write()
        .await
        .request_join("Caster", ParticipantRole::Spectator)
        .unwrap();

    let error = app
        .configure_renderer(
            "A",
            RendererRequest {
                participant_id: Some(spectator),
                participant_name: Some("Caster".into()),
                mode: None,
                width: None,
                height: None,
                fps: None,
                delay_ms: None,
                featured: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("active players"));
    let _ = std::fs::remove_dir_all(root);
}
