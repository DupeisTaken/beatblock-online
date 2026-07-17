use beatblock_together_companion::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{CompanionConfig, Envelope},
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

fn percentile(values: &mut [f64], percentile: f64) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values[((values.len() - 1) as f64 * percentile).round() as usize]
}

fn app(root: PathBuf) -> AppState {
    AppState::new(root, "benchmark-token".into(), CompanionConfig::default())
        .unwrap()
        .0
}

fn envelope(kind: &str, sequence: u64, payload: serde_json::Value) -> Envelope {
    let mut message = Envelope::new(kind, sequence, payload);
    message.run_time_us = sequence;
    message
}

fn report_path() -> PathBuf {
    std::env::var_os("BBT_BENCH_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../reports/trial-runs/companion-benchmark-latest.json")
        })
}

fn write_report(path: &Path, report: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut bytes = serde_json::to_vec_pretty(report).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let root = std::env::temp_dir().join(format!("bbt-benchmark-{}", rand::random::<u64>()));
    let chart = root.join("chart");
    let cache = root.join("cache");
    std::fs::create_dir_all(&chart).unwrap();
    let block = vec![0x5au8; 32 * 1024];
    for index in 0..128 {
        std::fs::write(chart.join(format!("asset-{index:03}.bin")), &block).unwrap();
    }

    let started = Instant::now();
    let cold = canonical_chart_hash_cached(&chart, &cache).unwrap();
    let cold_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let started = Instant::now();
    let cached = canonical_chart_hash_cached(&chart, &cache).unwrap();
    let cached_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(cold.hash, cached.hash);

    let state = app(root.clone());
    let mut export_ms = Vec::with_capacity(250);
    let mut export_enqueue_ms = Vec::with_capacity(250);
    for sequence in 0..250 {
        let started = Instant::now();
        state
            .ingest(envelope(
                "gameplay.snapshot",
                sequence,
                json!({
                    "state":"playing","playerName":"Benchmark Player",
                    "songName":"Stress Signal","lobbyName":"Maximum Grid",
                    "accuracy":99.42,"combo":sequence,"misses":1,"rank":1,
                    "progress":sequence as f64 / 250.0,"connected":true,
                    "updatedAtMs":sequence
                }),
            ))
            .await
            .unwrap();
        export_enqueue_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
        // Measure completed publication in realistic coalesced batches instead
        // of reporting only the near-zero queueing time as filesystem latency.
        if sequence % 10 == 9 {
            let started = Instant::now();
            state.exports.flush();
            export_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
        }
    }
    let exported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("exports/gameplay.json")).unwrap())
            .unwrap();
    assert_eq!(exported["combo"], 249);

    let journal_started = Instant::now();
    for sequence in 0..5_000 {
        state
            .ingest(envelope(
                "run.score_delta",
                sequence,
                json!({"lobbyId":"lobby-1","runId":"run-1","runSequence":sequence}),
            ))
            .await
            .unwrap();
    }
    state.journals.flush();
    let journal_seconds = journal_started.elapsed().as_secs_f64();
    let recovered = state.journal_events();
    assert_eq!(recovered.len(), 5_000);

    let mut p95_values = export_ms.clone();
    let export_p95_ms = percentile(&mut p95_values, 0.95);
    let mut enqueue_p95_values = export_enqueue_ms.clone();
    let export_enqueue_p95_ms = percentile(&mut enqueue_p95_values, 0.95);
    let journal_events_per_second = 5_000.0 / journal_seconds;
    let sqlite_started = Instant::now();
    for sequence in 0..5_000 {
        let mut event = envelope(
            "run.score_delta",
            sequence,
            json!({"lobbyId":"lobby-1","runId":"run-1","runSequence":sequence}),
        );
        event.run_id = Some("run-1".into());
        state.storage.queue_event("lobby-1", &event).unwrap();
    }
    state.storage.flush_pending_events().unwrap();
    let sqlite_seconds = sqlite_started.elapsed().as_secs_f64();
    let sqlite_recovered = state.storage.events("lobby-1", "run-1", 0).unwrap();
    let sqlite_journal_events_per_second = 5_000.0 / sqlite_seconds;
    assert_eq!(sqlite_recovered.len(), 5_000);
    let passed = cached_ms < cold_ms
        && export_p95_ms < 100.0
        && journal_events_per_second > 100.0
        && sqlite_journal_events_per_second > 1_000.0;
    let report = json!({
        "schemaVersion": 2,
        "generatedAtMs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
        "passed": passed,
        "workload": {"chartBytes": 128 * 32 * 1024, "snapshots": 250, "journalEvents": 5_000},
        "metrics": {
            "chartColdMs": cold_ms,
            "chartCachedMs": cached_ms,
            "chartCacheSpeedup": cold_ms / cached_ms.max(0.001),
            "exportMeanMs": export_ms.iter().sum::<f64>() / export_ms.len() as f64,
            "exportP95Ms": export_p95_ms,
            "exportEnqueueP95Ms": export_enqueue_p95_ms,
            "exportFilesWritten": state.exports.completed_writes(),
            "exportBytesWritten": state.exports.completed_bytes(),
            "journalEventsPerSecond": journal_events_per_second,
            "journalRecoveredEvents": recovered.len(),
            "sqliteJournalEventsPerSecond": sqlite_journal_events_per_second,
            "sqliteJournalRecoveredEvents": sqlite_recovered.len()
        },
        "thresholds": {
            "cacheHitFasterThanCold": true,
            "exportP95Ms": 100,
            "exportFilesWritten": 60,
            "journalEventsPerSecond": 100,
            "sqliteJournalEventsPerSecond": 1_000
        }
    });
    let output = report_path();
    write_report(&output, &report);
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    println!("Report: {}", output.display());
    drop(state);
    let _ = std::fs::remove_dir_all(root);
    assert!(passed, "companion benchmark thresholds were not met");
}
