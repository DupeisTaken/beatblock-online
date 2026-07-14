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
            line = lines.next_line() => match line? { Some(line) => { let message: Envelope = serde_json::from_str(&line)?; if let Err(error) = state.ingest(message).await { tracing::warn!(%error, "IPC message rejected"); } }, None => break },
            event = events.recv() => if let Ok(event) = event { writer.write_all(serde_json::to_string(&event)?.as_bytes()).await?; writer.write_all(b"\n").await?; }
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
            .create(r"\\.\pipe\beatblock-together-v1")?;
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
                    line = lines.next_line() => match line { Ok(Some(line)) => match serde_json::from_str::<Envelope>(&line) { Ok(message) => { let _ = state.ingest(message).await; }, Err(error) => tracing::warn!(%error, "bad pipe message") }, _ => break },
                    event = events.recv() => if let Ok(event) = event { if writer.write_all(serde_json::to_string(&event).unwrap_or_default().as_bytes()).await.is_err() { break; } let _ = writer.write_all(b"\n").await; }
                }
            }
        });
    }
}

async fn write_ready<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    state: &AppState,
) -> Result<()> {
    let config = state.config.read().await.clone();
    let ready = Envelope {
        version: 1,
        kind: "companion.ready".into(),
        sequence: 0,
        timestamp_ms: unix_ms(),
        payload: serde_json::json!({
            "configured": config.instance_url.is_some(),
            "userId": config.user_id,
            "displayName": config.display_name,
            "role": config.role
        }),
    };
    writer
        .write_all(serde_json::to_string(&ready)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
