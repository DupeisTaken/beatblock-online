use beatblock_online_companion::{
    app_state::AppState,
    game_commands,
    model::{
        AdmissionMode, ChartLock, ChartTransferMode, CompanionConfig, Envelope, ParticipantRole,
        RendererRequest, RoomLifecycle,
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
    let canonical = beatblock_online_companion::chart_hash::canonical_chart_hash(&chart).unwrap();
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
    let canonical = beatblock_online_companion::chart_hash::canonical_chart_hash(&chart).unwrap();
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
    assert_eq!(locked.transfer_mode, ChartTransferMode::HostTransfer);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn host_can_disable_custom_chart_transfer_for_the_room() {
    let root = temporary("game-command-transfer-disabled");
    let chart = root.join("chart");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("manifest.json"), b"local-only chart").unwrap();
    let app = state(root.clone(), &"f".repeat(64)).await;
    app.room.write().await.snapshot.allow_chart_transfers = false;
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
    assert_eq!(
        app.room
            .read()
            .await
            .snapshot
            .chart
            .as_ref()
            .unwrap()
            .transfer_mode,
        ChartTransferMode::VerifyOnly
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn appending_a_setlist_chart_preserves_the_active_chart_and_host_verification() {
    let root = temporary("game-command-append");
    let first = root.join("first");
    let second = root.join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("manifest.json"), b"first competition chart").unwrap();
    std::fs::write(second.join("manifest.json"), b"second competition chart").unwrap();
    let first_hash = beatblock_online_companion::chart_hash::canonical_chart_hash(&first).unwrap();

    let app = AppState::new(
        root.clone(),
        "test-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    let room = RoomEngine::host("Test".into(), "Host".into(), AdmissionMode::PasswordOnly);
    let host = room.snapshot.host_session_id.clone();
    *app.room.write().await = room;
    *app.local_session_id.write().await = Some(host.clone());
    app.is_host.store(true, Ordering::Relaxed);
    *app.selected_chart_path.write().await = Some("Custom Levels/First/".into());
    app.lock_chart(
        ChartLock {
            hash: first_hash.hash.clone(),
            package_name: "first".into(),
            song_name: "First".into(),
            variant: "Hard".into(),
            expected_max_hits: 1,
            official: false,
            transfer_mode: ChartTransferMode::VerifyOnly,
        },
        true,
    )
    .await
    .unwrap();
    app.set_local_verified(true, None).await.unwrap();

    let mut append = chart_command(
        "room.chart_select_request",
        second.to_str().unwrap(),
        "Hard",
        1,
    );
    append.payload["appendToSetlist"] = json!(true);
    append.payload["levelPath"] = json!("Custom Levels/Second/");
    let mut events = app.events.subscribe();
    game_commands::handle(&app, &append).await.unwrap();

    let mut saw_verification = false;
    loop {
        let event = events.recv().await.unwrap();
        if event.kind == "chart.verification" {
            saw_verification = true;
        }
        if event.kind == "control.ack" {
            break;
        }
    }
    let snapshot = app.room.read().await.snapshot.clone();
    assert_eq!(snapshot.setlist.len(), 2);
    assert_eq!(snapshot.chart.as_ref().unwrap().hash, first_hash.hash);
    assert!(
        snapshot
            .participants
            .iter()
            .find(|participant| participant.session_id == host)
            .unwrap()
            .verified
    );
    assert_eq!(
        app.selected_chart_path.read().await.as_deref(),
        Some("Custom Levels/First/")
    );
    assert!(!saw_verification);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn removing_the_active_setlist_chart_updates_the_active_game_path() {
    let root = temporary("game-command-remove-active");
    let app = AppState::new(
        root.clone(),
        "test-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    let room = RoomEngine::host("Test".into(), "Host".into(), AdmissionMode::PasswordOnly);
    let host = room.snapshot.host_session_id.clone();
    *app.room.write().await = room;
    *app.local_session_id.write().await = Some(host);
    app.is_host.store(true, Ordering::Relaxed);

    for (name, digit) in [("First", 'a'), ("Second", 'b')] {
        *app.selected_chart_path.write().await = Some(format!("Custom Levels/{name}/"));
        app.lock_chart(
            ChartLock {
                hash: digit.to_string().repeat(64),
                package_name: name.into(),
                song_name: name.into(),
                variant: "Hard".into(),
                expected_max_hits: 1,
                official: false,
                transfer_mode: ChartTransferMode::VerifyOnly,
            },
            true,
        )
        .await
        .unwrap();
    }
    app.remove_setlist(0).await.unwrap();
    let snapshot = app.room.read().await.snapshot.clone();
    assert_eq!(snapshot.chart.as_ref().unwrap().song_name, "Second");
    assert_eq!(
        app.selected_chart_path.read().await.as_deref(),
        Some("Custom Levels/Second/")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn protocol_v2_is_rejected_instead_of_silently_downgraded() {
    let root = temporary("protocol-v2-rejected");
    let app = AppState::new(
        root.clone(),
        "test-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    let mut legacy = Envelope::new("diagnostics.get", 1, json!({"requestId":"legacy"}));
    legacy.version = 2;
    let error = app.ingest(legacy).await.unwrap_err().to_string();
    assert!(error.contains("unsupported protocol version 2"));
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
async fn runtime_snapshot_carries_the_authoritative_local_session_id() {
    let root = temporary("runtime-session-id");
    let app = AppState::new(
        root.clone(),
        "test-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    *app.local_session_id.write().await = Some("session-authoritative".into());
    let mut events = app.events.subscribe();
    app.publish_runtime_snapshot().await.unwrap();
    let snapshot = events.recv().await.unwrap();
    assert_eq!(snapshot.kind, "runtime.snapshot");
    assert_eq!(snapshot.payload["sessionId"], "session-authoritative");
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn invalid_control_request_returns_a_correlated_structured_error() {
    let root = temporary("control-error");
    let app = AppState::new(
        root.clone(),
        "test-token".into(),
        CompanionConfig::default(),
    )
    .unwrap()
    .0;
    let mut events = app.events.subscribe();
    let command = Envelope::new(
        "room.host_request",
        3,
        json!({"requestId":"req-invalid","name":"Missing Password","port":32145}),
    );
    assert!(game_commands::handle(&app, &command).await.unwrap());
    let error = loop {
        let event = events.recv().await.unwrap();
        if event.kind == "control.error" {
            break event;
        }
    };
    assert_eq!(error.payload["requestId"], "req-invalid");
    assert_eq!(error.payload["code"], "auth.rejected");
    assert_eq!(error.payload["retryable"], true);
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

#[tokio::test]
async fn renderer_reconfiguration_is_rejected_after_the_countdown_begins() {
    let root = temporary("renderer-active-race");
    let app = state(root.clone(), &"a".repeat(64)).await;
    let host = app.room.read().await.snapshot.host_session_id.clone();
    app.room.write().await.snapshot.lifecycle = RoomLifecycle::Playing;

    let error = app
        .configure_renderer(
            "A",
            RendererRequest {
                participant_id: Some(host),
                participant_name: Some("Host".into()),
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

    assert!(error.contains("before the synchronized start"));
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn host_play_command_preserves_host_identity_across_spectating() {
    let root = temporary("host-play-command");
    let app = state(root.clone(), &"a".repeat(64)).await;
    let host = app.room.read().await.snapshot.host_session_id.clone();
    app.renderer
        .configure(
            "A",
            RendererRequest {
                participant_id: Some(host.clone()),
                participant_name: Some("Host".into()),
                mode: None,
                width: None,
                height: None,
                fps: None,
                delay_ms: None,
                featured: None,
            },
        )
        .unwrap();

    let spectate = Envelope::new("room.host_play_set", 1, json!({"participating": false}));
    assert!(game_commands::handle(&app, &spectate).await.unwrap());
    assert_eq!(
        app.room.read().await.player(&host).unwrap().role,
        ParticipantRole::Spectator
    );
    assert!(!app.renderer.slot("A").unwrap().active);
    assert!(
        !app.broadcast_plan
            .read()
            .await
            .slots
            .iter()
            .find(|slot| slot.id == "A")
            .unwrap()
            .active
    );

    let play = Envelope::new("room.host_play_set", 2, json!({"participating": true}));
    assert!(game_commands::handle(&app, &play).await.unwrap());
    assert_eq!(
        app.room.read().await.player(&host).unwrap().role,
        ParticipantRole::Host
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn host_can_toggle_run_validity_checks_before_a_race() {
    let root = temporary("validity-check-command");
    let app = state(root.clone(), &"a".repeat(64)).await;

    let disable = Envelope::new("room.validity_checks_set", 1, json!({"enabled": false}));
    assert!(game_commands::handle(&app, &disable).await.unwrap());
    assert!(!app.room.read().await.snapshot.validity_checks_enabled);

    app.room.write().await.snapshot.lifecycle = RoomLifecycle::Playing;
    let mut events = app.events.subscribe();
    let locked = Envelope::new(
        "room.validity_checks_set",
        2,
        json!({"requestId":"checks-locked","enabled": true}),
    );
    assert!(game_commands::handle(&app, &locked).await.unwrap());
    let error = loop {
        let event = events.recv().await.unwrap();
        if event.kind == "control.error" {
            break event;
        }
    };
    assert_eq!(error.payload["requestId"], "checks-locked");
    assert_eq!(error.payload["command"], "room.validity_checks_set");
    assert!(!app.room.read().await.snapshot.validity_checks_enabled);

    app.room.write().await.snapshot.lifecycle = RoomLifecycle::Ready;
    let malformed = Envelope::new(
        "room.validity_checks_set",
        3,
        json!({"requestId":"checks-malformed","enabled":"yes"}),
    );
    assert!(game_commands::handle(&app, &malformed).await.unwrap());
    let error = loop {
        let event = events.recv().await.unwrap();
        if event.kind == "control.error" && event.payload["requestId"] == "checks-malformed" {
            break event;
        }
    };
    assert!(error.payload["message"]
        .as_str()
        .unwrap()
        .contains("boolean enabled"));
    assert!(!app.room.read().await.snapshot.validity_checks_enabled);
    let _ = std::fs::remove_dir_all(root);
}
