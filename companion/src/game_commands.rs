use crate::{app_state::AppState, chart_hash::canonical_chart_hash_cached, model::Envelope};
use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::{json, Value};

pub async fn handle(state: &AppState, message: &Envelope) -> Result<bool> {
    let result = match message.kind.as_str() {
        "lobby.create_request" => {
            let name = message
                .payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Beatblock lobby");
            remote(
                state,
                Method::POST,
                "/api/v1/lobbies",
                json!({"name": name}),
            )
            .await
        }
        "lobby.join_request" => {
            let code = required(&message.payload, "code")?;
            let spectator = message
                .payload
                .get("spectator")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            remote(
                state,
                Method::POST,
                &format!("/api/v1/lobbies/{}/join", encode(code)),
                json!({"spectator": spectator}),
            )
            .await
        }
        "lobby.ready_request" => {
            let lobby_id = current_lobby_id(state).await?;
            let ready = message
                .payload
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let fingerprint = state
                .lobby
                .read()
                .await
                .pointer("/chart/hash")
                .cloned()
                .unwrap_or(Value::Null);
            remote(
                state,
                Method::PUT,
                &format!("/api/v1/lobbies/{lobby_id}/ready"),
                json!({"ready": ready, "fingerprint": fingerprint, "client": state.client.read().await.clone()}),
            )
            .await
        }
        "lobby.start_request" => {
            let lobby_id = current_lobby_id(state).await?;
            remote(
                state,
                Method::POST,
                &format!("/api/v1/lobbies/{lobby_id}/start"),
                json!({}),
            )
            .await
        }
        "lobby.leave_request" => {
            let lobby_id = current_lobby_id(state).await?;
            let outcome = remote(
                state,
                Method::POST,
                &format!("/api/v1/lobbies/{lobby_id}/leave"),
                json!({}),
            )
            .await;
            if outcome.is_ok() {
                let _ = state.remote_tx.try_send(Envelope {
                    version: 1,
                    kind: "lobby.unsubscribe".into(),
                    sequence: 0,
                    timestamp_ms: unix_ms(),
                    payload: json!({"lobbyId": lobby_id}),
                });
                publish(
                    state,
                    "lobby.snapshot",
                    json!({"id":"offline","code":"LOCAL","name":"Offline practice","lifecycle":"forming","players":[]}),
                )
                .await;
                publish(
                    state,
                    "lobby.context",
                    json!({"lobbyId":"offline","lobbyName":"Offline practice"}),
                )
                .await;
            }
            outcome
        }
        "lobby.close_request" => {
            let lobby_id = current_lobby_id(state).await?;
            remote(
                state,
                Method::POST,
                &format!("/api/v1/lobbies/{lobby_id}/close"),
                json!({}),
            )
            .await
        }
        "lobby.chart_select_request" | "lobby.chart_verify_request" => chart(state, message).await,
        _ => return Ok(false),
    };
    if let Err(error) = result {
        publish(
            state,
            "companion.error",
            json!({"command": message.kind, "message": error.to_string()}),
        )
        .await;
    }
    Ok(true)
}

async fn chart(state: &AppState, message: &Envelope) -> Result<()> {
    let path = required(&message.payload, "path")?;
    let result = canonical_chart_hash_cached(path, state.data_dir.join("chart-cache"))?;
    let expected = state
        .lobby
        .read()
        .await
        .pointer("/chart/hash")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let selecting = message.kind == "lobby.chart_select_request";
    if selecting {
        let lobby_id = current_lobby_id(state).await?;
        let chart = json!({
            "algorithm": result.algorithm.clone(),
            "hash": result.hash.clone(),
            "packageName": result.package_name.clone(),
            "songName": required(&message.payload, "songName")?,
            "variant": message.payload.get("variant").and_then(Value::as_str).unwrap_or("Default"),
            "expectedMaxHits": message.payload.get("expectedMaxHits").and_then(Value::as_u64).unwrap_or(1).max(1)
        });
        remote(
            state,
            Method::PUT,
            &format!("/api/v1/lobbies/{lobby_id}/chart"),
            chart,
        )
        .await?;
    }
    let verified = expected
        .as_deref()
        .map_or(selecting, |hash| hash == result.hash);
    publish(
        state,
        "chart.verification",
        json!({
            "verified": verified,
            "hash": result.hash,
            "levelPath": message.payload.get("levelPath"),
            "variant": message.payload.get("variant"),
            "reason": if verified { Value::Null } else { Value::String("Selected chart package does not match the lobby".into()) }
        }),
    )
    .await;
    Ok(())
}

async fn remote(state: &AppState, method: Method, path: &str, body: Value) -> Result<()> {
    let config = state.config.read().await.clone();
    let instance = config
        .instance_url
        .context("No competition instance is configured")?;
    let access_token = keyring::Entry::new("BeatblockTogether", "access-token")?
        .get_password()
        .context("Remote session is unavailable; reconnect in the companion console")?;
    let response = reqwest::Client::new()
        .request(
            method,
            format!("{}{}", instance.trim_end_matches('/'), path),
        )
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .unwrap_or_else(|_| json!({"error":"Invalid competition instance response"}));
    if !status.is_success() {
        anyhow::bail!(
            "{}",
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Competition instance rejected the command")
        );
    }
    publish(state, "lobby.snapshot", value.clone()).await;
    publish(
        state,
        "lobby.context",
        json!({
            "lobbyId": value.get("id"),
            "lobbyName": value.get("name"),
            "playerName": config.display_name,
            "userId": config.user_id
        }),
    )
    .await;
    Ok(())
}

async fn publish(state: &AppState, kind: &str, payload: Value) {
    if kind == "lobby.snapshot" {
        *state.lobby.write().await = payload.clone();
    }
    let _ = state.events.send(Envelope {
        version: 1,
        kind: kind.into(),
        sequence: 0,
        timestamp_ms: unix_ms(),
        payload,
    });
}

async fn current_lobby_id(state: &AppState) -> Result<String> {
    state
        .lobby
        .read()
        .await
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| *id != "offline")
        .map(str::to_owned)
        .context("Join a competition lobby first")
}

fn required<'a>(payload: &'a Value, field: &str) -> Result<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{field} is required"))
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
