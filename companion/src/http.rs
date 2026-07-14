use crate::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{ChartHashRequest, CompanionConfig, RedeemRequest, SpectateRequest},
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

fn authorized(state: &AppState, token: &Option<String>) -> Result<(), StatusCode> {
    if token.as_deref() == Some(state.local_token.as_str()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub fn router(state: AppState, web_dir: std::path::PathBuf) -> Router {
    let index = web_dir.join("index.html");
    Router::new()
        .route("/v1/state", get(get_state))
        .route("/v1/lobby", get(get_lobby))
        .route("/v1/players", get(get_players))
        .route("/v1/run", get(get_state))
        .route("/v1/events", get(events))
        .route("/v1/config", get(get_config).put(put_config))
        .route("/v1/redeem", post(redeem))
        .route("/v1/chart-hash", post(chart_hash))
        .route("/v1/spectate", post(spectate))
        .route("/v1/lobbies", post(create_lobby))
        .route("/v1/lobbies/join", post(join_lobby))
        .route("/v1/lobby/chart", put(lock_chart))
        .route("/v1/lobby/ready", put(set_ready))
        .route("/v1/lobby/start", post(start_lobby))
        .route("/v1/lobby/close", post(close_lobby))
        .nest_service("/assets", ServeDir::new(web_dir.join("assets")))
        .fallback_service(ServeFile::new(index))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| {
                    let value = origin.as_bytes();
                    value.starts_with(b"http://127.0.0.1:")
                        || value.starts_with(b"http://localhost:")
                }))
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers([header::CONTENT_TYPE]),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn get_state(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<crate::model::GameplayState>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(state.gameplay.read().await.clone()))
}
async fn get_lobby(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(state.lobby.read().await.clone()))
}
async fn get_players(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(
        state
            .lobby
            .read()
            .await
            .get("players")
            .cloned()
            .unwrap_or_else(|| json!([])),
    ))
}
async fn get_config(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<CompanionConfig>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(state.config.read().await.clone()))
}
async fn put_config(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(config): Json<CompanionConfig>,
) -> Result<Json<CompanionConfig>, StatusCode> {
    authorized(&state, &query.token)?;
    persist_config(&state, &config)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(config))
}
async fn chart_hash(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<ChartHashRequest>,
) -> Result<Json<crate::chart_hash::ChartHash>, StatusCode> {
    authorized(&state, &query.token)?;
    canonical_chart_hash_cached(request.path, state.data_dir.join("chart-cache"))
        .map(Json)
        .map_err(|error| {
            tracing::warn!(%error, "chart hashing failed");
            StatusCode::BAD_REQUEST
        })
}

async fn redeem(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<RedeemRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let endpoint = format!(
        "{}/api/v1/auth/redeem",
        request.instance_url.trim_end_matches('/')
    );
    let response = match reqwest::Client::new().post(endpoint).json(&json!({ "inviteCode": request.invite_code, "displayName": request.display_name, "deviceName": request.device_name.unwrap_or_else(|| "Windows PC".into()) })).send().await {
        Ok(response) => response, Err(error) => return (StatusCode::BAD_GATEWAY, Json(json!({"error": error.to_string()}))).into_response(),
    };
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .unwrap_or_else(|_| json!({"error":"Invalid instance response"}));
    if !status.is_success() {
        return (
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(value),
        )
            .into_response();
    }
    if let Some(token) = value.get("accessToken").and_then(Value::as_str) {
        if let Ok(entry) = keyring::Entry::new("BeatblockTogether", "access-token") {
            let _ = entry.set_password(token);
        }
    }
    if let Some(token) = value.get("refreshToken").and_then(Value::as_str) {
        if let Ok(entry) = keyring::Entry::new("BeatblockTogether", "refresh-token") {
            let _ = entry.set_password(token);
        }
    }
    let config = CompanionConfig {
        instance_url: Some(request.instance_url),
        user_id: value
            .pointer("/user/id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        display_name: value
            .pointer("/user/displayName")
            .and_then(Value::as_str)
            .map(str::to_owned),
        role: value
            .pointer("/user/role")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    if let Err(error) = persist_config(&state, &config).await {
        tracing::warn!(%error, "config persistence failed");
    }
    let _ = state.events.send(crate::model::Envelope {
        version: 1,
        kind: "companion.ready".into(),
        sequence: 0,
        timestamp_ms: unix_ms(),
        payload: json!({
            "configured": true,
            "userId": config.user_id,
            "displayName": config.display_name,
            "role": config.role
        }),
    });
    Json(value).into_response()
}

async fn spectate(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<SpectateRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let config = state.config.read().await.clone();
    let Some(instance_url) = config.instance_url else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"No competition instance is configured"})),
        )
            .into_response();
    };
    let access_token = match keyring::Entry::new("BeatblockTogether", "access-token")
        .and_then(|entry| entry.get_password())
    {
        Ok(token) => token,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"Remote session is unavailable"})),
            )
                .into_response()
        }
    };
    let response = match reqwest::Client::new()
        .post(format!(
            "{}/api/v1/auth/browser-ticket",
            instance_url.trim_end_matches('/')
        ))
        .bearer_auth(access_token)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error":error.to_string()})),
            )
                .into_response()
        }
    };
    if !response.status().is_success() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unable to create browser handoff"})),
        )
            .into_response();
    }
    let value: Value = response.json().await.unwrap_or_default();
    let Some(ticket) = value.get("ticket").and_then(Value::as_str) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let url = format!(
        "{}/?api={}&lobby={}&ticket={}",
        instance_url.trim_end_matches('/'),
        url::form_urlencoded::byte_serialize(instance_url.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(request.lobby_id.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect::<String>()
    );
    Json(json!({ "url": url, "expiresInSeconds": 60 })).into_response()
}

async fn create_lobby(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(body): Json<Value>,
) -> Response {
    proxy_lobby(
        state,
        query,
        reqwest::Method::POST,
        "/api/v1/lobbies".into(),
        body,
    )
    .await
}

async fn join_lobby(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(body): Json<Value>,
) -> Response {
    let Some(code) = body.get("code").and_then(Value::as_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"code is required"})),
        )
            .into_response();
    };
    proxy_lobby(
        state,
        query,
        reqwest::Method::POST,
        format!(
            "/api/v1/lobbies/{}/join",
            url::form_urlencoded::byte_serialize(code.as_bytes()).collect::<String>()
        ),
        json!({ "spectator": body.get("spectator").and_then(Value::as_bool).unwrap_or(false) }),
    )
    .await
}

async fn lock_chart(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(body): Json<Value>,
) -> Response {
    let Some(lobby_id) = body.get("lobbyId").and_then(Value::as_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"lobbyId is required"})),
        )
            .into_response();
    };
    let chart = body.get("chart").cloned().unwrap_or(Value::Null);
    proxy_lobby(
        state,
        query,
        reqwest::Method::PUT,
        format!("/api/v1/lobbies/{lobby_id}/chart"),
        chart,
    )
    .await
}

async fn set_ready(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(mut body): Json<Value>,
) -> Response {
    let Some(lobby_id) = body
        .get("lobbyId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"lobbyId is required"})),
        )
            .into_response();
    };
    let client = state.client.read().await.clone();
    if let Some(object) = body.as_object_mut() {
        object.remove("lobbyId");
        object.insert("client".into(), client);
    }
    proxy_lobby(
        state,
        query,
        reqwest::Method::PUT,
        format!("/api/v1/lobbies/{lobby_id}/ready"),
        body,
    )
    .await
}

async fn start_lobby(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(body): Json<Value>,
) -> Response {
    lobby_command(state, query, body, "start").await
}

async fn close_lobby(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(body): Json<Value>,
) -> Response {
    lobby_command(state, query, body, "close").await
}

async fn lobby_command(state: AppState, query: TokenQuery, body: Value, command: &str) -> Response {
    let Some(lobby_id) = body.get("lobbyId").and_then(Value::as_str) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"lobbyId is required"})),
        )
            .into_response();
    };
    proxy_lobby(
        state,
        query,
        reqwest::Method::POST,
        format!("/api/v1/lobbies/{lobby_id}/{command}"),
        json!({}),
    )
    .await
}

async fn proxy_lobby(
    state: AppState,
    query: TokenQuery,
    method: reqwest::Method,
    path: String,
    body: Value,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let config = state.config.read().await.clone();
    let Some(instance_url) = config.instance_url else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"No competition instance is configured"})),
        )
            .into_response();
    };
    let access_token = match keyring::Entry::new("BeatblockTogether", "access-token")
        .and_then(|entry| entry.get_password())
    {
        Ok(token) => token,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"Remote session is unavailable"})),
            )
                .into_response()
        }
    };
    let response = match reqwest::Client::new()
        .request(
            method,
            format!("{}{}", instance_url.trim_end_matches('/'), path),
        )
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error":error.to_string()})),
            )
                .into_response()
        }
    };
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .unwrap_or_else(|_| json!({"error":"Invalid instance response"}));
    if status.is_success() {
        let event = crate::model::Envelope {
            version: 1,
            kind: "lobby.snapshot".into(),
            sequence: 0,
            timestamp_ms: unix_ms(),
            payload: value.clone(),
        };
        let _ = state.ingest_remote(event).await;
        let context = crate::model::Envelope {
            version: 1,
            kind: "lobby.context".into(),
            sequence: 0,
            timestamp_ms: unix_ms(),
            payload: json!({ "lobbyId": value.get("id"), "lobbyName": value.get("name"), "playerName": config.display_name, "userId": config.user_id }),
        };
        let _ = state.ingest_remote(context).await;
    }
    (
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(value),
    )
        .into_response()
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn events(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(ws.on_upgrade(move |socket| event_socket(socket, state)))
}
async fn event_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.events.subscribe();
    loop {
        tokio::select! {
            event = events.recv() => match event { Ok(event) => if sender.send(Message::Text(serde_json::to_string(&event).unwrap_or_default().into())).await.is_err() { break; }, Err(_) => break },
            incoming = receiver.next() => match incoming { Some(Ok(Message::Close(_))) | None => break, _ => {} }
        }
    }
}
async fn persist_config(state: &AppState, config: &CompanionConfig) -> anyhow::Result<()> {
    let path = state.data_dir.join("config.json");
    let temporary = state.data_dir.join("config.json.tmp");
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(config)?).await?;
    crate::exports::replace_file(&temporary, &path)?;
    *state.config.write().await = config.clone();
    Ok(())
}
