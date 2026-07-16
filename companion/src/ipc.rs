use crate::{app_state::AppState, model::Envelope};
use anyhow::Result;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

pub async fn run_tcp(state: AppState) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8975").await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_tcp(stream, state).await {
                tracing::warn!(%error, "IPC client disconnected");
            }
        });
    }
}

async fn handle_tcp(stream: TcpStream, state: AppState) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut events = state.events.subscribe();
    write_ready(&mut writer, &state).await?;
    loop {
        tokio::select! {
            line = lines.next_line() => match line? { Some(line) => match serde_json::from_str::<Envelope>(&line) { Ok(message) => if let Err(error) = state.ingest(message).await { publish_ipc_error(&state, error.to_string()); }, Err(error) => publish_ipc_error(&state, format!("malformed protocol message: {error}")) }, None => break },
            event = events.recv() => if let Ok(event) = event { if let Some(event) = for_game(event) { writer.write_all(serde_json::to_string(&event)?.as_bytes()).await?; writer.write_all(b"\n").await?; } }
        }
    }
    Ok(())
}

#[cfg(windows)]
pub async fn run_named_pipe(state: AppState) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(r"\\.\pipe\beatblock-together-v2")?;
        server.connect().await?;
        let state = state.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut lines = BufReader::new(reader).lines();
            let mut events = state.events.subscribe();
            if write_ready(&mut writer, &state).await.is_err() {
                return;
            }
            loop {
                tokio::select! {
                    line = lines.next_line() => match line { Ok(Some(line)) => match serde_json::from_str::<Envelope>(&line) { Ok(message) => if let Err(error) = state.ingest(message).await { publish_ipc_error(&state, error.to_string()); }, Err(error) => publish_ipc_error(&state, format!("malformed protocol message: {error}")) }, _ => break },
                    event = events.recv() => if let Ok(event) = event { if let Some(event) = for_game(event) { if writer.write_all(serde_json::to_string(&event).unwrap_or_default().as_bytes()).await.is_err() { break; } let _ = writer.write_all(b"\n").await; } }
                }
            }
        });
    }
}

fn publish_ipc_error(state: &AppState, message: String) {
    tracing::warn!(%message, "IPC message rejected");
    let code = if message.contains("unsupported protocol version") {
        "protocol.incompatible"
    } else {
        "protocol.malformed"
    };
    let _ = state.events.send(Envelope::new(
        "runtime.error",
        0,
        serde_json::json!({"code":code,"message":message}),
    ));
}

async fn write_ready<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    state: &AppState,
) -> Result<()> {
    let config = state.config.read().await.clone();
    let ready = Envelope::new(
        "runtime.ready",
        0,
        serde_json::json!({
            "configured": true,
            "displayName": config.display_name,
            "role": config.requested_role,
            "hostAddress": config.host_address,
            "hostPort": config.host_port,
            "hosting": state.is_host.load(std::sync::atomic::Ordering::Relaxed),
            "connection": state.connection_status.read().await.clone(),
            "sessionId": state.local_session_id.read().await.clone(),
            "runtimeTimeMs": crate::room::unix_ms(),
        }),
    );
    writer
        .write_all(serde_json::to_string(&ready)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

fn for_game(mut event: Envelope) -> Option<Envelope> {
    // Client telemetry and control requests already originated in Beatblock.
    // Echoing them back fills the pipe at 60 Hz and can starve shutdown/control
    // reads after the game leaves Online. Only runtime-owned public state crosses
    // the return path.
    let allowed = matches!(
        event.kind.as_str(),
        "runtime.error"
            | "control.ack"
            | "control.error"
            | "runtime.snapshot"
            | "room.snapshot"
            | "room.context"
            | "room.start_scheduled"
            | "lobby.snapshot"
            | "lobby.context"
            | "lobby.start_scheduled"
            | "chart.verification"
            | "clock.pong"
            | "leaderboard.snapshot"
            | "match.results"
            | "renderer.snapshot"
            | "history.snapshot"
            | "diagnostics.snapshot"
    );
    if !allowed {
        return None;
    }
    if let Some(payload) = event.payload.as_object_mut() {
        payload.insert(
            "runtimeTimeMs".into(),
            serde_json::json!(crate::room::unix_ms()),
        );
    }
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_ipc_does_not_echo_high_rate_client_telemetry() {
        assert!(for_game(Envelope::new("render.sample", 1, serde_json::json!({}))).is_none());
        assert!(for_game(Envelope::new("gameplay.snapshot", 2, serde_json::json!({}))).is_none());
        assert!(for_game(Envelope::new("runtime.snapshot", 3, serde_json::json!({}))).is_some());
        assert!(for_game(Envelope::new("control.ack", 4, serde_json::json!({}))).is_some());
    }
}
