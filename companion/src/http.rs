use crate::{
    app_state::{AppState, HostRoomOptions},
    chart_hash::canonical_chart_hash_cached,
    model::{
        AdmissionRequest, ChartHashRequest, ChartLock, CompanionConfig, HostRoomRequest,
        JoinRoomRequest, ReadyRequest, RendererRequest, StartRequest, DEFAULT_HOST_PORT,
    },
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        DefaultBodyLimit, Path, State, WebSocketUpgrade,
    },
    http::{header, HeaderMap, Method, Request, StatusCode},
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
#[serde(rename_all = "camelCase")]
struct LockChartRequest {
    chart: ChartLock,
    #[serde(default)]
    append_to_setlist: bool,
}

fn authorized(state: &AppState, candidate: Option<&str>) -> Result<(), StatusCode> {
    let token = state.local_token.read().expect("token lock poisoned");
    if candidate.is_some_and(|candidate| constant_time_eq(candidate.as_bytes(), token.as_bytes())) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn websocket_token(headers: &HeaderMap) -> Option<(&str, Option<String>)> {
    if let Some(token) = bearer_token(headers) {
        return Some((token, None));
    }
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)?
        .to_str()
        .ok()?
        .split(',')
        .map(str::trim)
        .find_map(|protocol| {
            protocol
                .strip_prefix("bbt-token.")
                .map(|token| (token, Some(protocol.to_owned())))
        })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn is_allowed_local_origin(value: &[u8]) -> bool {
    let Ok(value) = std::str::from_utf8(value) else {
        return false;
    };
    let Ok(origin) = url::Url::parse(value) else {
        return false;
    };
    origin.scheme() == "http"
        && matches!(origin.host_str(), Some("127.0.0.1" | "localhost"))
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.path() == "/"
        && origin.query().is_none()
        && origin.fragment().is_none()
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
                    is_allowed_local_origin(origin.as_bytes())
                }))
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
        )
        // Every JSON command is tiny; reject oversized localhost requests
        // before buffering them even when the bearer token is valid.
        .layer(DefaultBodyLimit::max(64 * 1024))
        // Query strings may contain third-party data. Log only the path so
        // credentials can never reappear when verbose HTTP tracing is enabled.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::debug_span!(
                    "http.request",
                    method = %request.method(),
                    path = request.uri().path()
                )
            }),
        )
        .with_state(state)
}

async fn get_state(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    Ok(Json(json!({
        "gameplay": state.gameplay.read().await.clone(),
        "connection": state.connection_status.read().await.clone(),
        "hosting": state.is_host.load(std::sync::atomic::Ordering::Relaxed),
        "sessionId": state.local_session_id.read().await.clone(),
    })))
}

async fn get_room(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    Ok(Json(
        serde_json::to_value(&state.room.read().await.snapshot).unwrap_or_default(),
    ))
}

async fn get_players(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    Ok(Json(json!(state.room.read().await.snapshot.participants)))
}

async fn get_streams(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    Ok(Json(json!({
        "streams": state.renderer.slots(),
        "budgetWarning": state.renderer.budget_warning(),
    })))
}

async fn get_history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    state
        .storage
        .history()
        .map(|history| Json(json!(history)))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    let local_addresses = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, address)| address.is_ipv4())
        .map(|(name, address)| json!({"adapter":name,"address":address.to_string()}))
        .collect::<Vec<_>>();
    let client = state.client.read().await.clone();
    Ok(Json(json!({
        "protocolVersion": crate::model::PROTOCOL_VERSION,
        "runtimeVersion": env!("CARGO_PKG_VERSION"),
        "testedBeatblockVersion": crate::compatibility::TESTED_BEATBLOCK_VERSION,
        "testedBeatblockBuildId": crate::compatibility::TESTED_BEATBLOCK_BUILD_ID,
        "detectedBeatblockVersion": client.get("gameVersion").cloned().unwrap_or(Value::Null),
        "detectedBeatblockBuildId": client.get("gameBuildId").cloned().unwrap_or(Value::Null),
        "detectedBeatblockBuildSource": client.get("gameBuildSource").cloned().unwrap_or(Value::Null),
        "connection": state.connection_status.read().await.clone(),
        "hosting": state.is_host.load(std::sync::atomic::Ordering::Relaxed),
        "peerCount": state.network.peer_count().await,
        "localAddresses": local_addresses,
        "rendererBudgetWarning": state.renderer.budget_warning(),
        "dataDirectory": state.data_dir.to_string_lossy(),
        "ipcClientId": state.ipc_client_id().await,
        "relayAvailable": false,
    })))
}

async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CompanionConfig>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    Ok(Json(state.config.read().await.clone()))
}

async fn put_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(config): Json<CompanionConfig>,
) -> Result<Json<CompanionConfig>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    state
        .replace_config(config)
        .await
        .map(Json)
        .map_err(|error| {
            tracing::warn!(%error, "invalid runtime configuration");
            StatusCode::BAD_REQUEST
        })
}

async fn chart_hash(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ChartHashRequest>,
) -> Result<Json<crate::chart_hash::ChartHash>, StatusCode> {
    authorized(&state, bearer_token(&headers))?;
    canonical_chart_hash_cached(request.path, state.data_dir.join("chart-cache"))
        .map(Json)
        .map_err(|error| {
            tracing::warn!(%error, "chart hashing failed");
            StatusCode::BAD_REQUEST
        })
}

async fn host_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<HostRoomRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state
        .host_room(HostRoomOptions {
            room_name: request.name,
            password: request.password,
            port: request.port.unwrap_or(DEFAULT_HOST_PORT),
            admission_mode: request
                .admission_mode
                .unwrap_or(crate::model::AdmissionMode::HostApproval),
            host_participating: request.host_participating.unwrap_or(true),
            validity_checks_enabled: request.validity_checks_enabled.unwrap_or(true),
            require_same_game_build: request.require_same_game_build.unwrap_or(true),
            modifiers: request.modifiers.unwrap_or_default(),
        })
        .await
    {
        Ok(address) => Json(json!({
            "address":address.to_string(),
            "joinUri":join_uri(address),
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
    headers: HeaderMap,
    Json(request): Json<JoinRoomRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
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
    headers: HeaderMap,
    Json(request): Json<AdmissionRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
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
    headers: HeaderMap,
    Json(request): Json<LockChartRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
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
    headers: HeaderMap,
    Json(request): Json<ReadyRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    result_response(state.set_local_ready(request.ready).await)
}

async fn start_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StartRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
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

async fn close_room(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    result_response(state.close_room().await)
}

async fn configure_stream(
    State(state): State<AppState>,
    Path(slot): Path<String>,
    headers: HeaderMap,
    Json(request): Json<RendererRequest>,
) -> Response {
    if authorized(&state, bearer_token(&headers)).is_err() {
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
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !is_allowed_local_origin(origin.as_bytes()) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    let (token, protocol) = websocket_token(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    authorized(&state, Some(token))?;
    let websocket = if let Some(protocol) = protocol {
        websocket.protocols([protocol])
    } else {
        websocket
    };
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

fn join_uri(address: SocketAddr) -> String {
    format!(
        "bbt://{}?v={}",
        public_display_address(address),
        crate::model::PROTOCOL_VERSION
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_tokens_use_headers_and_constant_time_comparison() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer abc123".parse().unwrap());
        assert_eq!(bearer_token(&headers), Some("abc123"));
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"different"));
        assert!(!constant_time_eq(b"same", b"samf"));
    }

    #[test]
    fn websocket_subprotocol_carries_the_token_without_a_query_string() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "other, bbt-token.abc123".parse().unwrap(),
        );
        assert_eq!(
            websocket_token(&headers),
            Some(("abc123", Some("bbt-token.abc123".into())))
        );
    }

    #[test]
    fn cors_and_websocket_origins_are_exactly_loopback_http() {
        for allowed in [
            b"http://localhost:3000".as_slice(),
            b"http://127.0.0.1:8080".as_slice(),
        ] {
            assert!(is_allowed_local_origin(allowed));
        }
        for rejected in [
            b"https://localhost:3000".as_slice(),
            b"http://localhost.evil.example:3000".as_slice(),
            b"http://127.0.0.1.example:3000".as_slice(),
            b"http://localhost:3000/path".as_slice(),
        ] {
            assert!(!is_allowed_local_origin(rejected));
        }
    }

    #[test]
    fn generated_join_links_follow_the_protocol_constant() {
        let address: SocketAddr = "0.0.0.0:32145".parse().unwrap();
        assert_eq!(
            join_uri(address),
            format!("bbt://127.0.0.1:32145?v={}", crate::model::PROTOCOL_VERSION)
        );
    }
}
