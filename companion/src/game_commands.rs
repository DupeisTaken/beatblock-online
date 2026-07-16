use crate::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{ChartLock, ChartTransferMode, Envelope, ParticipantRole, RendererRequest},
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub async fn handle(state: &AppState, message: &Envelope) -> Result<bool> {
    let result = match message.kind.as_str() {
        "room.host_request" | "lobby.create_request" => {
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
            if let Some(display_name) = message.payload.get("displayName").and_then(Value::as_str) {
                state
                    .save_profile(display_name.to_owned(), None, ParticipantRole::Host)
                    .await?;
            }
            state
                .host_room(
                    name,
                    password,
                    port,
                    if admission {
                        crate::model::AdmissionMode::HostApproval
                    } else {
                        crate::model::AdmissionMode::PasswordOnly
                    },
                )
                .await
                .map(|_| ())
        }
        "room.join_request" | "lobby.join_request" => {
            let address = required(&message.payload, "address")?;
            let address = resolve_address(address).await?;
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
            state
                .join_room(address, password, &display_name, role)
                .await
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
            state.require_host_control()?;
            let id = required(&message.payload, "sessionId")?;
            let role = parse_role(message.payload.get("role"))?;
            state.room.write().await.set_role(id, role)?;
            if role == ParticipantRole::Spectator {
                state.renderer.stop_participant(id);
            }
            state.publish_room().await
        }
        "room.kick" => {
            let id = required(&message.payload, "sessionId")?;
            state.kick(id).await
        }
        "setlist.remove" => {
            state.require_host_control()?;
            state
                .room
                .write()
                .await
                .remove_setlist(required_index(&message.payload, "index")?)?;
            state.publish_room().await
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
            state.require_host_control()?;
            let slot = required(&message.payload, "slot")?;
            if state.renderer.slot(slot).is_none() {
                anyhow::bail!("unknown renderer slot");
            }
            state.renderer.stop_slot(slot);
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
            publish_snapshots(state).await
        }
        "settings.update" => update_settings(state, &message.payload).await,
        "runtime.snapshot_request" | "diagnostics.get" => publish_snapshots(state).await,
        "api.token_rotate" => {
            let token = hex::encode(rand::random::<[u8; 24]>());
            std::fs::write(state.data_dir.join("local-token.txt"), &token)?;
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
        _ => return Ok(false),
    };
    match result {
        Ok(()) => publish(state, "control.ack", json!({"requestId":message.request_id,"command":message.kind})).await,
        Err(error) => publish(state, "control.error", json!({"requestId":message.request_id,"command":message.kind,"message":error.to_string()})).await,
    }
    Ok(true)
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
    if let Some(name) = payload.get("displayName").and_then(Value::as_str) {
        config.display_name = name.trim().to_owned();
    }
    if let Some(port) = payload.get("hostPort").and_then(Value::as_u64) {
        config.host_port = u16::try_from(port)?;
    }
    if let Some(delay) = payload.get("spectatorDelayMs").and_then(Value::as_u64) {
        config.spectator_delay_ms = (delay as u32).clamp(250, 1500);
    }
    if let Some(enabled) = payload.get("hudEnabled").and_then(Value::as_bool) {
        config.hud_enabled = enabled;
    }
    std::fs::write(
        state.data_dir.join("config.json"),
        serde_json::to_vec_pretty(&*config)?,
    )?;
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
                message
                    .payload
                    .get("appendToSetlist")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
            .await?;
    }
    let expected = state.room.read().await.snapshot.chart.clone();
    let reason = chart_mismatch_reason(
        expected.as_ref(),
        &hash,
        variant,
        expected_max_hits,
        "Atom Map",
    );
    let verified = reason.is_none();
    state.set_local_verified(verified, reason.clone()).await?;
    publish(state, "chart.verification", json!({"verified":verified,"hash":hash,"official":true,"chartId":chart_id,"variant":variant,"reason":reason})).await;
    Ok(())
}

async fn chart(state: &AppState, message: &Envelope) -> Result<()> {
    if message
        .payload
        .get("allowTransfer")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        anyhow::bail!("host chart transfer is not available; every player must install and verify the chart locally");
    }
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
    if selecting {
        *state.selected_chart_path.write().await = Some(
            message
                .payload
                .get("levelPath")
                .and_then(Value::as_str)
                .unwrap_or(path)
                .to_owned(),
        );
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
            official: message
                .payload
                .get("official")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            transfer_mode: ChartTransferMode::VerifyOnly,
        };
        let append = message
            .payload
            .get("appendToSetlist")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        state.lock_chart(chart, append).await?;
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
