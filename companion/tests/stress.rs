use beatblock_together_companion::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{CompanionConfig, Envelope, GameplayState},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, RwLock};

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bbt-{name}-{}", rand::random::<u64>()))
}

fn state(root: PathBuf, queue: usize) -> AppState {
    let (events, _) = broadcast::channel(4096);
    let (remote_tx, _remote_rx) = mpsc::channel(queue);
    AppState {
        local_token: Arc::new("test-token".into()),
        gameplay: Arc::new(RwLock::new(GameplayState::default())),
        lobby: Arc::new(RwLock::new(json!({"id":"stress"}))),
        config: Arc::new(RwLock::new(CompanionConfig::default())),
        client: Arc::new(RwLock::new(json!({}))),
        events,
        remote_tx,
        data_dir: Arc::new(root),
    }
}

fn message(kind: &str, sequence: u64, payload: serde_json::Value) -> Envelope {
    Envelope {
        version: 1,
        kind: kind.into(),
        sequence,
        timestamp_ms: sequence,
        payload,
    }
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
    for receiver in &mut receivers {
        for expected in 0..100 {
            assert_eq!(receiver.recv().await.unwrap().sequence, expected);
        }
    }
    let exported: GameplayState =
        serde_json::from_slice(&std::fs::read(root.join("exports/state.json")).unwrap()).unwrap();
    assert_eq!(exported.player_name, "Player 99");
    assert!(!root.join("exports/state.tmp").exists());
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
