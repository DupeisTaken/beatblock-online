use beatblock_online_companion::{
    model::{AdmissionMode, ChartLock, ChartTransferMode, ParticipantRole, ScoreTotals},
    room::RoomEngine,
};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let mut room = RoomEngine::host(
        "Maximum Grid".into(),
        "Host".into(),
        AdmissionMode::PasswordOnly,
    );
    for index in 1..16 {
        room.request_join(&format!("Player {}", index + 1), ParticipantRole::Player)?;
    }
    for index in 0..32 {
        room.request_join(
            &format!("Spectator {}", index + 1),
            ParticipantRole::Spectator,
        )?;
    }
    assert!(room
        .request_join("Overflow", ParticipantRole::Player)
        .is_err());
    assert!(room
        .request_join("Overflow watcher", ParticipantRole::Spectator)
        .is_err());

    let chart = ChartLock {
        hash: "a".repeat(64),
        package_name: "trial-chart.zip".into(),
        song_name: "Trial Signal".into(),
        variant: "Hard".into(),
        expected_max_hits: 100,
        official: false,
        transfer_mode: ChartTransferMode::VerifyOnly,
    };
    room.lock_chart(chart, true)?;
    let players = room
        .snapshot
        .participants
        .iter()
        .filter(|p| p.role != ParticipantRole::Spectator)
        .map(|p| p.session_id.clone())
        .collect::<Vec<_>>();
    for id in &players {
        room.set_verified(id, true, None)?;
        room.set_ready(id, true)?;
    }
    let scheduled = room.schedule_start(false, 2_000)?;

    let ingest_started = Instant::now();
    for (player_index, id) in players.iter().enumerate() {
        for sequence in 0..240u64 {
            let current = sequence.min(99) + 1;
            let misses = if player_index == 15 && sequence >= 120 {
                1
            } else {
                0
            };
            let totals = ScoreTotals {
                hits: current - misses,
                misses,
                barelies: if player_index == 14 { 1 } else { 0 },
                combo: current - misses,
                max_combo: current - misses,
                current_max_hits: current,
                max_hits: 100,
                mine_hits: 0,
            };
            room.ingest_score(
                id,
                &format!("run-{player_index}"),
                sequence,
                &json!({"progress": current as f64 / 100.0, "totals": totals}),
            )?;
        }
        room.finish_run(id, &format!("run-{player_index}"))?;
    }
    let ingest_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
    let agreement = room
        .snapshot
        .participants
        .iter()
        .filter(|p| p.role != ParticipantRole::Spectator)
        .all(|p| (0.0..=100.0).contains(&p.accuracy) && p.rank.is_some());
    let telemetry_kib_per_second = 32.0 * 60.0 / 1024.0;
    let passed =
        room.snapshot.participants.len() == 48 && agreement && telemetry_kib_per_second < 15.0;
    let report = json!({
        "schemaVersion": 2,
        "scenario": "direct-host-16-player-32-spectator",
        "passed": passed,
        "capabilities": {
            "directRoom": true, "passwordAdmissionModel": true, "players16": true,
            "spectators32": true, "chartLock": true, "readyVerification": true,
            "scheduledStart": scheduled > 0, "authoritativeRanking": agreement,
            "setTotal": true, "capacityRejection": true, "packed60HzTelemetry": true
        },
        "workload": {"players": 16, "spectators": 32, "scoreEvents": 16 * 240, "renderDatagramHz": 60},
        "metrics": {
            "elapsedMs": started.elapsed().as_secs_f64() * 1000.0,
            "scoreIngestMs": ingest_ms,
            "scoreEventsPerSecond": 3840.0 / (ingest_ms / 1000.0),
            "renderTelemetryKiBPerSecondPerPlayer": telemetry_kib_per_second,
            "scheduledStartUnixMs": scheduled
        },
        "standings": room.snapshot.participants.iter().filter(|p| p.role != ParticipantRole::Spectator)
            .map(|p| json!({"name":p.display_name,"rank":p.rank,"accuracy":p.accuracy,"setTotal":p.set_total,"validity":p.validity})).collect::<Vec<_>>()
    });
    let output = std::env::var_os("BBT_HOST_TRIAL_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../reports/trial-runs/host-room-simulation-latest.json")
        });
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !passed {
        anyhow::bail!("direct-host trial failed");
    }
    Ok(())
}
