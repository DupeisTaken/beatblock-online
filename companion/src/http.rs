use crate::{
    app_state::AppState,
    chart_hash::canonical_chart_hash_cached,
    model::{
        AdmissionRequest, ChartHashRequest, ChartLock, CompanionConfig, HostRoomRequest,
        JoinRoomRequest, ReadyRequest, RendererRequest, StartRequest, DEFAULT_HOST_PORT,
    },
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockChartRequest {
    chart: ChartLock,
    #[serde(default)]
    append_to_setlist: bool,
}

fn authorized(state: &AppState, token: &Option<String>) -> Result<(), StatusCode> {
    if token.as_deref() == Some(state.local_token.read().expect("token lock poisoned").as_str()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/state", get(get_state))
        .route("/v1/room", get(get_room))
        .route("/v1/lobby", get(get_room))
        .route("/v1/players", get(get_players))
        .route("/v1/run", get(get_state))
        .route("/v1/streams", get(get_streams))
        .route("/v1/history", get(get_history))
        .route("/v1/diagnostics", get(get_diagnostics))
        .route("/v1/events", get(events))
        .route("/v1/config", get(get_config).put(put_config))
        .route("/v1/chart-hash", post(chart_hash))
        .route("/v1/host", post(host_room))
        .route("/v1/join", post(join_room))
        .route("/v1/room/admission", put(admit))
        .route("/v1/room/chart", put(lock_chart))
        .route("/v1/room/ready", put(set_ready))
        .route("/v1/room/start", post(start_room))
        .route("/v1/room/close", post(close_room))
        .route("/v1/streams/{slot}", put(configure_stream))
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
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(json!({
        "gameplay": state.gameplay.read().await.clone(),
        "connection": state.connection_status.read().await.clone(),
        "hosting": state.is_host.load(std::sync::atomic::Ordering::Relaxed),
        "sessionId": state.local_session_id.read().await.clone(),
    })))
}

async fn get_room(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(
        serde_json::to_value(&state.room.read().await.snapshot).unwrap_or_default(),
    ))
}

async fn get_players(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(json!(state.room.read().await.snapshot.participants)))
}

async fn get_streams(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(Json(json!({
        "streams": state.renderer.slots(),
        "budgetWarning": state.renderer.budget_warning(),
    })))
}

async fn get_history(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    state
        .storage
        .history()
        .map(|history| Json(json!(history)))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_diagnostics(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, &query.token)?;
    let local_addresses = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, address)| address.is_ipv4())
        .map(|(name, address)| json!({"adapter":name,"address":address.to_string()}))
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "protocolVersion": crate::model::PROTOCOL_VERSION,
        "runtimeVersion": env!("CARGO_PKG_VERSION"),
        "connection": state.connection_status.read().await.clone(),
        "hosting": state.is_host.load(std::sync::atomic::Ordering::Relaxed),
        "peerCount": state.network.peer_count().await,
        "localAddresses": local_addresses,
        "rendererBudgetWarning": state.renderer.budget_warning(),
        "dataDirectory": state.data_dir.to_string_lossy(),
        "relayAvailable": false,
    })))
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
    let bytes = serde_json::to_vec_pretty(&config).map_err(|_| StatusCode::BAD_REQUEST)?;
    std::fs::write(state.data_dir.join("config.json"), bytes)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    *state.config.write().await = config.clone();
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

async fn host_room(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<HostRoomRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state
        .host_room(
            request.name,
            request.password,
            request.port.unwrap_or(DEFAULT_HOST_PORT),
            request
                .admission_mode
                .unwrap_or(crate::model::AdmissionMode::HostApproval),
        )
        .await
    {
        Ok(address) => Json(json!({
            "address":address.to_string(),
            "joinUri":format!("bbt://{}?v=2", public_display_address(address)),
            "room":state.room.read().await.snapshot.clone(),
        }))
        .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn join_room(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<JoinRoomRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let address = match resolve_address(&request.address).await {
        Ok(address) => address,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":error.to_string()})),
            )
                .into_response()
        }
    };
    match state
        .join_room(
            address,
            &request.password,
            &request.display_name,
            request.role,
        )
        .await
    {
        Ok(session_id) => Json(json!({"sessionId":session_id,"address":address})).into_response(),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn admit(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<AdmissionRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    result_response(
        state
            .admit(&request.session_id, request.admit, request.role)
            .await,
    )
}

async fn lock_chart(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<LockChartRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    result_response(
        state
            .lock_chart(request.chart, request.append_to_setlist)
            .await,
    )
}

async fn set_ready(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<ReadyRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    result_response(state.set_local_ready(request.ready).await)
}

async fn start_room(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<StartRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.start_room(request.force).await {
        Ok(start) => {
            Json(json!({"scheduledStartTimeMs":start,"force":request.force})).into_response()
        }
        Err(error) => (
            StatusCode::CONFLICT,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn close_room(State(state): State<AppState>, Query(query): Query<TokenQuery>) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    result_response(state.close_room().await)
}

async fn configure_stream(
    State(state): State<AppState>,
    Path(slot): Path<String>,
    Query(query): Query<TokenQuery>,
    Json(request): Json<RendererRequest>,
) -> Response {
    if authorized(&state, &query.token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.configure_renderer(&slot, request).await {
        Ok(stream) => Json(json!(stream)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn events(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, StatusCode> {
    authorized(&state, &query.token)?;
    Ok(websocket.on_upgrade(move |socket| stream_events(socket, state)))
}

async fn stream_events(mut socket: WebSocket, state: AppState) {
    let mut events = state.events.subscribe();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if socket.send(Message::Text(serde_json::to_string(&event).unwrap_or_default().into())).await.is_err() { break; }
                }
                Err(_) => break,
            },
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}

fn result_response(result: anyhow::Result<()>) -> Response {
    match result {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(error) => (
            StatusCode::CONFLICT,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn resolve_address(value: &str) -> anyhow::Result<SocketAddr> {
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
        format!("{stripped}:{DEFAULT_HOST_PORT}")
    };
    tokio::net::lookup_host(with_port)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("host address did not resolve"))
}

fn public_display_address(bound: SocketAddr) -> String {
    if bound.ip().is_unspecified() {
        format!("127.0.0.1:{}", bound.port())
    } else {
        bound.to_string()
    }
}
