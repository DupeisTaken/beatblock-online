use crate::{app_state::AppState, model::Envelope};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub async fn run(state: AppState, mut outbound: mpsc::Receiver<Envelope>) {
    loop {
        let config = state.config.read().await.clone();
        let Some(instance_url) = config.instance_url else {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        };
        let Ok(entry) = keyring::Entry::new("BeatblockTogether", "access-token") else {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        };
        let access_token = match entry.get_password() {
            Ok(token) => token,
            Err(_) => match refresh_access(&instance_url).await {
                Some(token) => token,
                None => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            },
        };
        let gateway = instance_url
            .replace("https://", "wss://")
            .replace("http://", "ws://")
            + "/api/v1/gateway?access_token="
            + &url::form_urlencoded::byte_serialize(access_token.as_bytes()).collect::<String>();
        match connect_async(gateway).await {
            Ok((socket, _)) => {
                let (mut writer, mut reader) = socket.split();
                tracing::info!("connected to competition instance");
                for event in state.journal_events() {
                    if writer
                        .send(Message::Text(
                            serde_json::to_string(&event).unwrap_or_default().into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                let mut clock = tokio::time::interval(std::time::Duration::from_secs(5));
                let mut clock_sequence = 0u64;
                loop {
                    tokio::select! {
                        outgoing = outbound.recv() => match outgoing { Some(event) => { if writer.send(Message::Text(serde_json::to_string(&event).unwrap_or_default().into())).await.is_err() { break; } }, None => return },
                        _ = clock.tick() => {
                            let event = Envelope { version: 1, kind: "clock.ping".into(), sequence: clock_sequence, timestamp_ms: unix_ms(), payload: serde_json::json!({ "clientSendTimeMs": unix_ms() }) };
                            clock_sequence += 1;
                            if writer.send(Message::Text(serde_json::to_string(&event).unwrap_or_default().into())).await.is_err() { break; }
                        },
                        incoming = reader.next() => match incoming { Some(Ok(Message::Text(text))) => if let Ok(mut event) = serde_json::from_str::<Envelope>(&text) {
                            if event.kind == "clock.pong" {
                                if let Some(payload) = event.payload.as_object_mut() {
                                    payload.insert("companionReceiveTimeMs".into(), serde_json::json!(unix_ms()));
                                }
                            }
                            let _ = state.ingest_remote(event).await;
                        }, Some(Ok(_)) => {}, _ => break }
                    }
                }
                disconnected(&state, "Competition gateway disconnected").await;
            }
            Err(error) => {
                tracing::warn!(%error, "remote gateway connection failed");
                disconnected(&state, &format!("Competition gateway unavailable: {error}")).await;
                let _ = refresh_access(&instance_url).await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn disconnected(state: &AppState, reason: &str) {
    let _ = state
        .ingest_remote(Envelope {
            version: 1,
            kind: "gateway.disconnected".into(),
            sequence: 0,
            timestamp_ms: unix_ms(),
            payload: serde_json::json!({"reason": reason}),
        })
        .await;
}

async fn refresh_access(instance_url: &str) -> Option<String> {
    let refresh_entry = keyring::Entry::new("BeatblockTogether", "refresh-token").ok()?;
    let refresh_token = refresh_entry.get_password().ok()?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/auth/refresh",
            instance_url.trim_end_matches('/')
        ))
        .json(&serde_json::json!({ "refreshToken": refresh_token }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: serde_json::Value = response.json().await.ok()?;
    let access_token = value.get("accessToken")?.as_str()?.to_owned();
    let next_refresh = value.get("refreshToken")?.as_str()?.to_owned();
    keyring::Entry::new("BeatblockTogether", "access-token")
        .ok()?
        .set_password(&access_token)
        .ok()?;
    refresh_entry.set_password(&next_refresh).ok()?;
    Some(access_token)
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
