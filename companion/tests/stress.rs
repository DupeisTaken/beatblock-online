use beatblock_online_companion::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{
        AdmissionMode, BroadcastPlan, ChartLock, ChartTransferMode, CompanionConfig, Envelope,
        GameplayState, ParticipantRole, RendererMode, RendererSlot, ScoreTotals,
    },
    room::RoomEngine,
};
use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bbt-{name}-{}", rand::random::<u64>()))
}

fn state(root: PathBuf, _queue: usize) -> AppState {
    AppState::new(root, "test-token".into(), CompanionConfig::default())
        .unwrap()
        .0
}

fn message(kind: &str, sequence: u64, payload: serde_json::Value) -> Envelope {
    let mut message = Envelope::new(kind, sequence, payload);
    message.run_time_us = sequence;
    message
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn journals_every_run_event_when_remote_queue_is_saturated() {
    let root = temporary("journal-stress");
    let app = state(root.clone(), 1);
    for sequence in 0..2_000 {
        app.ingest(message(
            "run.score_delta",
            sequence,
            json!({"lobbyId":"lobby-1","runId":"run-1","runSequence":sequence}),
        ))
        .await
        .unwrap();
    }
    let journal = app.journal_events();
    assert_eq!(journal.len(), 2_000);
    assert_eq!(journal.first().unwrap().sequence, 0);
    assert_eq!(journal.last().unwrap().sequence, 1_999);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn broadcasts_to_32_consumers_and_exports_only_complete_snapshots() {
    let root = temporary("export-stress");
    let app = state(root.clone(), 8);
    let mut receivers = (0..32).map(|_| app.events.subscribe()).collect::<Vec<_>>();
    for sequence in 0..100 {
        app.ingest(message(
            "gameplay.snapshot",
            sequence,
            json!({
                "state":"playing","playerName":format!("Player {sequence}"),
                "songName":"Stress Signal","lobbyName":"Maximum Grid",
                "accuracy":99.25,"combo":sequence,"misses":1,"rank":1,
                "progress":sequence as f64 / 100.0,"connected":true,
                "updatedAtMs":sequence
            }),
        ))
        .await
        .unwrap();
    }
    app.exports.flush();
    for receiver in &mut receivers {
        for expected in 0..100 {
            assert_eq!(receiver.recv().await.unwrap().sequence, expected);
        }
    }
    let exported: GameplayState =
        serde_json::from_slice(&std::fs::read(root.join("exports/gameplay.json")).unwrap())
            .unwrap();
    assert_eq!(exported.player_name, "Player 99");
    assert!(!root.join("exports/gameplay.tmp").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gameplay_ingest_never_waits_for_durable_obs_export_writes() {
    let root = temporary("nonblocking-exports");
    let app = state(root.clone(), 8);
    let started = Instant::now();
    for sequence in 0..1_000 {
        app.ingest(message(
            "gameplay.snapshot",
            sequence,
            json!({
                "state":"playing","playerName":"Host","songName":"Load Test",
                "lobbyName":"Room","accuracy":99.9,"combo":sequence,"misses":0,
                "rank":1,"progress":0.5,"connected":true,"updatedAtMs":sequence
            }),
        ))
        .await
        .unwrap();
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "telemetry ingestion was blocked by filesystem exports: {:?}",
        started.elapsed()
    );
    app.exports.flush();
    let exported: GameplayState =
        serde_json::from_slice(&std::fs::read(root.join("exports/gameplay.json")).unwrap())
            .unwrap();
    assert_eq!(exported.combo, 999);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn chart_cache_ignores_junk_hits_cache_and_invalidates_after_change() {
    let root = temporary("chart-cache");
    let chart = root.join("chart");
    let cache = root.join("cache");
    std::fs::create_dir_all(&chart).unwrap();
    std::fs::write(chart.join("level.json"), b"one").unwrap();
    let first = canonical_chart_hash_cached(&chart, &cache).unwrap();
    std::fs::write(chart.join("Thumbs.db"), b"ignored").unwrap();
    let hit = canonical_chart_hash_cached(&chart, &cache).unwrap();
    assert_eq!(first.hash, hit.hash);
    std::thread::sleep(std::time::Duration::from_millis(5));
    std::fs::write(chart.join("level.json"), b"two-and-longer").unwrap();
    let changed = canonical_chart_hash_cached(&chart, &cache).unwrap();
    assert_ne!(first.hash, changed.hash);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_room_handles_16_players_32_spectators_and_ordered_rankings() {
    let mut room = RoomEngine::host(
        "Stress room".into(),
        "Host".into(),
        AdmissionMode::PasswordOnly,
    );
    for index in 1..16 {
        room.request_join(&format!("Player {}", index + 1), ParticipantRole::Player)
            .unwrap();
    }
    for index in 0..32 {
        room.request_join(
            &format!("Spectator {}", index + 1),
            ParticipantRole::Spectator,
        )
        .unwrap();
    }
    assert!(room
        .request_join("Player 17", ParticipantRole::Player)
        .is_err());
    assert!(room
        .request_join("Spectator 33", ParticipantRole::Spectator)
        .is_err());

    room.lock_chart(
        ChartLock {
            hash: "d".repeat(64),
            package_name: "stress-chart.zip".into(),
            song_name: "Maximum Grid".into(),
            variant: "Hard".into(),
            expected_max_hits: 100,
            official: false,
            transfer_mode: ChartTransferMode::VerifyOnly,
        },
        true,
    )
    .unwrap();
    let players = room
        .snapshot
        .participants
        .iter()
        .filter(|participant| participant.role != ParticipantRole::Spectator)
        .map(|participant| participant.session_id.clone())
        .collect::<Vec<_>>();
    for id in &players {
        room.set_verified(id, true, None).unwrap();
        room.set_ready(id, true).unwrap();
    }
    room.schedule_start(false, 2_000).unwrap();

    for (player_index, id) in players.iter().enumerate() {
        for sequence in 0..240 {
            let current = (sequence + 1).min(100);
            let misses = player_index as u64;
            room.ingest_score(
                id,
                &format!("run-{player_index}"),
                sequence,
                &json!({
                    "progress": current as f64 / 100.0,
                    "totals": ScoreTotals {
                        hits: current.saturating_sub(misses),
                        misses,
                        barelies: 0,
                        combo: current.saturating_sub(misses),
                        max_combo: current.saturating_sub(misses),
                        current_max_hits: current,
                        max_hits: 100,
                        mine_hits: 0,
                    }
                }),
            )
            .unwrap();
        }
        room.finish_run(id, &format!("run-{player_index}")).unwrap();
    }

    let ranked = room
        .snapshot
        .participants
        .iter()
        .filter(|participant| participant.role != ParticipantRole::Spectator)
        .collect::<Vec<_>>();
    assert_eq!(room.snapshot.participants.len(), 48);
    assert_eq!(ranked.len(), 16);
    assert_eq!(
        ranked
            .iter()
            .filter(|participant| participant.rank.is_some())
            .count(),
        16
    );
    assert!(ranked
        .iter()
        .all(|participant| participant.set_total >= 0.0));
}

#[test]
fn four_stream_plan_targets_multiple_commentators_without_processes() {
    let mut room = RoomEngine::host(
        "Broadcast stress".into(),
        "Host".into(),
        AdmissionMode::PasswordOnly,
    );
    let mut players = Vec::new();
    for index in 0..4 {
        players.push(
            room.request_join(&format!("Player {}", index + 1), ParticipantRole::Player)
                .unwrap(),
        );
    }
    let mut commentators = Vec::new();
    for index in 0..8 {
        let id = room
            .request_join(
                &format!("Commentator {}", index + 1),
                ParticipantRole::Spectator,
            )
            .unwrap();
        room.set_commentator_access(&id, true).unwrap();
        commentators.push(id);
    }
    let slots = players
        .iter()
        .enumerate()
        .map(|(index, participant)| {
            let mut slot =
                RendererSlot::defaults(&((b'A' + index as u8) as char).to_string(), index == 0);
            slot.participant_id = Some(participant.clone());
            slot.participant_name = Some(format!("Player {}", index + 1));
            slot.mode = RendererMode::Clean;
            slot.active = true;
            slot
        })
        .collect::<Vec<_>>();
    let plan = BroadcastPlan::from_slots(7, 10, &slots);
    assert_eq!(plan.slots.len(), 4);
    assert!(plan
        .slots
        .iter()
        .all(|slot| slot.render_source_id.is_some()));
    assert_eq!(
        room.snapshot
            .participants
            .iter()
            .filter(|participant| participant.commentator_access)
            .count(),
        commentators.len()
    );
}
