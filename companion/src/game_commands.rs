use crate::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{ChartLock, ChartTransferMode, Envelope, ParticipantRole, RendererRequest},
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub async fn handle(state: &AppState, message: &Envelope) -> Result<bool> {
    if !is_control_command(&message.kind) {
        return Ok(false);
    }
    let result = execute(state, message).await;
    match result {
        Ok(()) => {
            publish(
                state,
                "control.ack",
                json!({"requestId":message.request_id,"command":message.kind}),
            )
            .await
        }
        Err(error) => {
            let (code, stage, retryable) = classify_error(&error.to_string());
            publish(
                state,
                "control.error",
                json!({
                    "requestId":message.request_id,
                    "command":message.kind,
                    "code":code,
                    "stage":stage,
                    "retryable":retryable,
                    "message":error.to_string()
                }),
            )
            .await
        }
    }
    Ok(true)
}

async fn execute(state: &AppState, message: &Envelope) -> Result<()> {
    let result = match message.kind.as_str() {
        "room.host_request" | "lobby.create_request" => {
            progress(state, message, "validating", "Checking room settings").await;
            let name = message
                .payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Beatblock room")
                .to_owned();
            let password = required(&message.payload, "password")?.to_owned();
            let port = message
                .payload
                .get("port")
                .and_then(Value::as_u64)
                .unwrap_or(crate::model::DEFAULT_HOST_PORT as u64) as u16;
            let admission = message
                .payload
                .get("hostApproval")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let host_participating = message
                .payload
                .get("hostParticipating")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let validity_checks_enabled = message
                .payload
                .get("validityChecksEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if let Some(display_name) = message.payload.get("displayName").and_then(Value::as_str) {
                state
                    .save_host_profile(display_name.to_owned(), port)
                    .await?;
            }
            progress(
                state,
                message,
                "starting",
                "Binding UDP and preparing the room",
            )
            .await;
            let hosted = tokio::time::timeout(
                Duration::from_secs(12),
                state.host_room(
                    name,
                    password,
                    port,
                    if admission {
                        crate::model::AdmissionMode::HostApproval
                    } else {
                        crate::model::AdmissionMode::PasswordOnly
                    },
                    host_participating,
                    validity_checks_enabled,
                ),
            )
            .await
            .context("room setup timed out while binding or mapping the host port")?
            .map(|_| ());
            if hosted.is_ok() {
                state.room.write().await.snapshot.allow_chart_transfers = message
                    .payload
                    .get("allowChartTransfers")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                state.publish_room().await?;
            }
            hosted
        }
        "room.join_request" | "lobby.join_request" => {
            let address = required(&message.payload, "address")?;
            progress(state, message, "resolving", "Resolving the host address").await;
            let address = tokio::time::timeout(Duration::from_secs(5), resolve_address(address))
                .await
                .context("host address resolution timed out")??;
            let password = required(&message.payload, "password")?;
            let display_name = message
                .payload
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or(&state.config.read().await.display_name)
                .to_owned();
            let role = if message
                .payload
                .get("spectator")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                ParticipantRole::Spectator
            } else {
                ParticipantRole::Player
            };
            state
                .save_profile(display_name.clone(), Some(address), role)
                .await?;
            progress(
                state,
                message,
                "connecting",
                "Connecting and authenticating",
            )
            .await;
            tokio::time::timeout(
                Duration::from_secs(12),
                state.join_room(address, password, &display_name, role),
            )
            .await
            .context("room connection timed out")?
            .map(|_| ())
        }
        "room.ready_request" | "lobby.ready_request" => {
            let ready = message
                .payload
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.set_local_ready(ready).await
        }
        "room.start_request" | "lobby.start_request" => {
            let force = message
                .payload
                .get("force")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.start_room(force).await.map(|_| ())
        }
        "room.close_request" | "lobby.close_request" => state.close_room().await,
        "room.leave_request" | "lobby.leave_request" => state.leave_room().await,
        "room.chart_select_request"
        | "room.chart_verify_request"
        | "lobby.chart_select_request"
        | "lobby.chart_verify_request" => chart(state, message).await,
        "room.official_chart_select" | "room.official_chart_verify" => {
            official_chart(state, message).await
        }
        "room.admission_set" => {
            let id = required(&message.payload, "sessionId")?;
            let admit = message
                .payload
                .get("admit")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let role = parse_role(message.payload.get("role"))?;
            state.admit(id, admit, role).await
        }
        "room.role_set" => {
            let id = required(&message.payload, "sessionId")?;
            let role = parse_role(message.payload.get("role"))?;
            state.set_participant_role(id, role).await
        }
        "room.host_play_set" => {
            let participating = message
                .payload
                .get("participating")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            state.set_host_participating(participating).await
        }
        "room.validity_checks_set" => {
            let enabled = message
                .payload
                .get("enabled")
                .and_then(Value::as_bool)
                .context("room.validity_checks_set requires a boolean enabled field")?;
            state.set_validity_checks(enabled).await
        }
        "room.commentator_set" => {
            let id = required(&message.payload, "sessionId")?;
            let enabled = message
                .payload
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.set_commentator_access(id, enabled).await
        }
        "room.kick" => {
            let id = required(&message.payload, "sessionId")?;
            state.kick(id).await
        }
        "setlist.remove" => {
            state
                .remove_setlist(required_index(&message.payload, "index")?)
                .await
        }
        "setlist.move" => {
            state.require_host_control()?;
            let from = required_index(&message.payload, "from")?;
            let to = required_index(&message.payload, "to")?;
            state.room.write().await.move_setlist(from, to)?;
            state.publish_room().await
        }
        "setlist.advance" => state.advance_setlist().await,
        "renderer.configure" => {
            let slot = required(&message.payload, "slot")?;
            let request: RendererRequest = serde_json::from_value(message.payload.clone())?;
            state.configure_renderer(slot, request).await?;
            publish_snapshots(state).await
        }
        "renderer.stop" => {
            let slot = required(&message.payload, "slot")?;
            state.stop_renderer_slot(slot).await?;
            publish_snapshots(state).await
        }
        "broadcast.mirror_set" => {
            let enabled = message
                .payload
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.set_local_broadcast_mirror(enabled).await?;
            publish_snapshots(state).await
        }
        "history.list" => publish_snapshots(state).await,
        "history.delete" => {
            state
                .storage
                .delete_history(required(&message.payload, "roomId")?)?;
            publish_snapshots(state).await
        }
        "history.prune" => {
            let days = message
                .payload
                .get("days")
                .and_then(Value::as_u64)
                .unwrap_or(30)
                .clamp(1, 365);
            state
                .storage
                .prune_raw_events(crate::room::unix_ms().saturating_sub(days * 86_400_000))?;
            state.journals.prune_days(days);
            publish_snapshots(state).await
        }
        "settings.update" => update_settings(state, &message.payload).await,
        "chart.cache_clear" => {
            let active_hash = state
                .room
                .read()
                .await
                .snapshot
                .chart
                .as_ref()
                .map(|chart| chart.hash.clone());
            crate::transfer::clear_cache(
                &state.data_dir.join("chart-cache"),
                active_hash.as_deref(),
            )?;
            publish_snapshots(state).await
        }
        "chart.transfer_request" => {
            if state.is_host.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("the host already owns the locked chart package");
            }
            let room = state.room.read().await.snapshot.clone();
            let chart = room.chart.context("the host has not locked a chart")?;
            if chart.official || chart.transfer_mode != ChartTransferMode::HostTransfer {
                anyhow::bail!("this chart is local-only or the host disabled transfers");
            }
            state
                .network
                .broadcast(Envelope::new(
                    "chart.transfer_request",
                    0,
                    json!({"chartHash":chart.hash}),
                ))
                .await;
            Ok(())
        }
        "chart.transfer_decision" => {
            state
                .decide_chart_transfer(
                    required(&message.payload, "requestId")?,
                    message
                        .payload
                        .get("accept")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    message
                        .payload
                        .get("trustRoom")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    message
                        .payload
                        .get("executableContentConfirmed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
                .await
        }
        "runtime.snapshot_request" | "diagnostics.get" => publish_snapshots(state).await,
        "api.token_rotate" => {
            let token = crate::credentials::rotate_local_token(&state.data_dir)?;
            *state.local_token.write().expect("token lock poisoned") = token;
            publish_snapshots(state).await
        }
        "paths.open_exports" => open::that(state.data_dir.join("exports")).map_err(Into::into),
        "paths.open_logs" => open::that(state.data_dir.join("logs")).map_err(Into::into),
        "runtime.session_end" => {
            if state.is_host.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = state.close_room().await;
            } else {
                state.network.shutdown().await;
            }
            state
                .shutdown_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        "runtime.restart_request" => {
            state
                .shutdown_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        // Keep the dispatcher fail-closed if the command inventory and
        // executor ever drift. Panicking here would strand the IPC control
        // guard in its busy state and make every later room action fail.
        _ => anyhow::bail!("unsupported control command {}", message.kind),
    };
    result
}

pub(crate) fn is_control_command(kind: &str) -> bool {
    matches!(
        kind,
        "room.host_request"
            | "lobby.create_request"
            | "room.join_request"
            | "lobby.join_request"
            | "room.ready_request"
            | "lobby.ready_request"
            | "room.start_request"
            | "lobby.start_request"
            | "room.close_request"
            | "lobby.close_request"
            | "room.leave_request"
            | "lobby.leave_request"
            | "room.chart_select_request"
            | "room.chart_verify_request"
            | "lobby.chart_select_request"
            | "lobby.chart_verify_request"
            | "room.official_chart_select"
            | "room.official_chart_verify"
            | "room.admission_set"
            | "room.role_set"
            | "room.host_play_set"
            | "room.validity_checks_set"
            | "room.commentator_set"
            | "room.kick"
            | "setlist.remove"
            | "setlist.move"
            | "setlist.advance"
            | "renderer.configure"
            | "renderer.stop"
            | "broadcast.mirror_set"
            | "history.list"
            | "history.delete"
            | "history.prune"
            | "settings.update"
            | "chart.cache_clear"
            | "chart.transfer_request"
            | "chart.transfer_decision"
            | "runtime.snapshot_request"
            | "diagnostics.get"
            | "api.token_rotate"
            | "paths.open_exports"
            | "paths.open_logs"
            | "runtime.session_end"
            | "runtime.restart_request"
    )
}

async fn progress(state: &AppState, message: &Envelope, stage: &str, detail: &str) {
    publish(
        state,
        "control.progress",
        json!({
            "requestId":message.request_id,
            "command":message.kind,
            "stage":stage,
            "message":detail,
        }),
    )
    .await;
}

fn classify_error(message: &str) -> (&'static str, &'static str, bool) {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("resolve") {
        ("network.resolve_failed", "resolving", true)
    } else if normalized.contains("timed out") {
        ("network.timeout", "connecting", true)
    } else if normalized.contains("password") || normalized.contains("authentication") {
        ("auth.rejected", "authenticating", true)
    } else if normalized.contains("address") || normalized.contains("bind") {
        ("network.bind_failed", "binding", true)
    } else if normalized.contains("protocol") {
        ("protocol.incompatible", "negotiating", false)
    } else {
        ("runtime.command_failed", "runtime", true)
    }
}

fn parse_role(value: Option<&Value>) -> Result<ParticipantRole> {
    match value.and_then(Value::as_str).unwrap_or("player") {
        "player" => Ok(ParticipantRole::Player),
        "spectator" => Ok(ParticipantRole::Spectator),
        "host" => Ok(ParticipantRole::Host),
        other => anyhow::bail!("unsupported participant role {other}"),
    }
}

fn required_index(payload: &Value, field: &str) -> Result<usize> {
    payload
        .get(field)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .with_context(|| format!("{field} is required"))
}

async fn update_settings(state: &AppState, payload: &Value) -> Result<()> {
    let mut config = state.config.write().await;
    let mut next = config.clone();
    if let Some(name) = payload.get("displayName").and_then(Value::as_str) {
        next.display_name = name.trim().to_owned();
    }
    if let Some(port) = payload.get("hostPort").and_then(Value::as_u64) {
        next.host_port = u16::try_from(port)?;
    }
    if let Some(delay) = payload.get("spectatorDelayMs").and_then(Value::as_u64) {
        next.spectator_delay_ms = u32::try_from(delay)?.clamp(250, 1500);
    }
    if let Some(enabled) = payload.get("hudEnabled").and_then(Value::as_bool) {
        next.hud_enabled = enabled;
    }
    crate::app_state::write_config_atomically(&state.data_dir, &next)?;
    *config = next;
    drop(config);
    publish_snapshots(state).await
}

async fn publish_snapshots(state: &AppState) -> Result<()> {
    state.publish_runtime_snapshot().await
}

async fn official_chart(state: &AppState, message: &Envelope) -> Result<()> {
    let chart_id = required(&message.payload, "chartId")?;
    let variant = message
        .payload
        .get("variant")
        .and_then(Value::as_str)
        .unwrap_or("Default");
    let client = state.client.read().await;
    let game_build = client
        .get("gameBuildHash")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let hash = hex::encode(Sha256::digest(format!(
        "atom-map-v1\0{game_build}\0{chart_id}\0{variant}"
    )));
    drop(client);
    let selecting = message.kind.ends_with("select");
    let expected_max_hits = message
        .payload
        .get("expectedMaxHits")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    let append = message
        .payload
        .get("appendToSetlist")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let had_active_chart = state.room.read().await.snapshot.chart.is_some();
    if selecting {
        *state.selected_chart_path.write().await = Some(chart_id.to_owned());
        state
            .lock_chart(
                ChartLock {
                    hash: hash.clone(),
                    package_name: chart_id.to_owned(),
                    song_name: message
                        .payload
                        .get("songName")
                        .and_then(Value::as_str)
                        .unwrap_or(chart_id)
                        .to_owned(),
                    variant: variant.to_owned(),
                    expected_max_hits,
                    official: true,
                    transfer_mode: ChartTransferMode::VerifyOnly,
                },
                append,
            )
            .await?;
    }
    // Appending a later setlist entry must not replace or invalidate the
    // host's currently selected chart. It becomes verifiable only after the
    // host advances the setlist and that entry becomes active.
    if selecting && append && had_active_chart {
        return Ok(());
    }
    let expected = state.room.read().await.snapshot.chart.clone();
    let reason = chart_mismatch_reason(
        expected.as_ref(),
        &hash,
        variant,
        expected_max_hits,
        "Freeplay",
    );
    let verified = reason.is_none();
    state.set_local_verified(verified, reason.clone()).await?;
    publish(state, "chart.verification", json!({"verified":verified,"hash":hash,"official":true,"chartId":chart_id,"variant":variant,"reason":reason})).await;
    Ok(())
}

async fn chart(state: &AppState, message: &Envelope) -> Result<()> {
    let path = required(&message.payload, "path")?;
    let result = canonical_chart_hash_cached(path, state.data_dir.join("chart-cache"))?;
    let selecting = message.kind.ends_with("chart_select_request");
    let variant = message
        .payload
        .get("variant")
        .and_then(Value::as_str)
        .unwrap_or("Default")
        .to_owned();
    let expected_max_hits = message
        .payload
        .get("expectedMaxHits")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    let append = message
        .payload
        .get("appendToSetlist")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let had_active_chart = state.room.read().await.snapshot.chart.is_some();
    if selecting {
        *state.selected_chart_path.write().await = Some(
            message
                .payload
                .get("levelPath")
                .and_then(Value::as_str)
                .unwrap_or(path)
                .to_owned(),
        );
        let official = message
            .payload
            .get("official")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let allow_transfer = state.room.read().await.snapshot.allow_chart_transfers;
        let chart = ChartLock {
            hash: result.hash.clone(),
            package_name: result.package_name.clone(),
            song_name: message
                .payload
                .get("songName")
                .and_then(Value::as_str)
                .unwrap_or(&result.package_name)
                .to_owned(),
            variant: variant.clone(),
            expected_max_hits,
            official,
            transfer_mode: if !official && allow_transfer {
                ChartTransferMode::HostTransfer
            } else {
                ChartTransferMode::VerifyOnly
            },
        };
        state.lock_chart(chart, append).await?;
    }
    if selecting && append && had_active_chart {
        return Ok(());
    }
    // Read the lock after a host selection. The previous implementation kept
    // the pre-selection hash, which made changing an existing custom chart
    // report a mismatch against the chart the host had just selected.
    let expected = state.room.read().await.snapshot.chart.clone();
    let reason = chart_mismatch_reason(
        expected.as_ref(),
        &result.hash,
        &variant,
        expected_max_hits,
        "custom chart",
    );
    let verified = reason.is_none();
    if verified {
        let local_path = message
            .payload
            .get("levelPath")
            .and_then(Value::as_str)
            .unwrap_or(path)
            .to_owned();
        *state.selected_chart_path.write().await = Some(local_path.clone());
        state
            .chart_paths
            .write()
            .await
            .insert(result.hash.clone(), local_path);
    }
    state.set_local_verified(verified, reason.clone()).await?;
    publish(
        state,
        "chart.verification",
        json!({
            "verified": verified,
            "hash": result.hash,
            "levelPath": message.payload.get("levelPath"),
            "variant": variant,
            "reason": reason,
        }),
    )
    .await;
    Ok(())
}

fn chart_mismatch_reason(
    expected: Option<&ChartLock>,
    actual_hash: &str,
    actual_variant: &str,
    actual_max_hits: u64,
    label: &str,
) -> Option<String> {
    let Some(expected) = expected else {
        return Some("The host has not locked a chart".into());
    };
    if expected.hash != actual_hash {
        return Some(format!("Selected {label} package does not match the host"));
    }
    if expected.variant != actual_variant {
        return Some(format!(
            "Selected {label} variant '{}' does not match host variant '{}'",
            actual_variant, expected.variant
        ));
    }
    if expected.expected_max_hits != actual_max_hits {
        return Some(format!(
            "Selected {label} note count {actual_max_hits} does not match host count {}",
            expected.expected_max_hits
        ));
    }
    None
}

async fn publish(state: &AppState, kind: &str, payload: Value) {
    let _ = state.events.send(Envelope::new(kind, 0, payload));
}

async fn resolve_address(value: &str) -> Result<std::net::SocketAddr> {
    let stripped = value
        .trim()
        .strip_prefix("bbt://")
        .unwrap_or(value.trim())
        .split('?')
        .next()
        .unwrap_or(value.trim());
    let with_port = if stripped.rsplit_once(':').is_some() {
        stripped.to_owned()
    } else {
        format!("{}:{}", stripped, crate::model::DEFAULT_HOST_PORT)
    };
    tokio::net::lookup_host(with_port)
        .await?
        .next()
        .context("host address did not resolve")
}

fn required<'a>(payload: &'a Value, field: &str) -> Result<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{field} is required"))
}
