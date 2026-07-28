use crate::{app_state::AppState, model::Envelope};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    time::{sleep, Duration},
};

const MAX_IPC_FRAME: usize = 1_048_576;
const MAX_IPC_CLIENTS: usize = 16;

struct ControlGuard(Option<AppState>);

impl Drop for ControlGuard {
    fn drop(&mut self) {
        if let Some(state) = self.0.take() {
            state.end_control();
        }
    }
}

pub async fn run_tcp(state: AppState) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8975").await?;
    let client_slots = Arc::new(Semaphore::new(MAX_IPC_CLIENTS));
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "TCP IPC listener accept failed; retrying");
                sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        let Ok(client_slot) = client_slots.clone().try_acquire_owned() else {
            // Loopback is still an untrusted boundary: cap abandoned local
            // connections so they cannot consume an unbounded task count.
            drop(stream);
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _client_slot = client_slot;
            if let Err(error) = handle_tcp(stream, state).await {
                tracing::warn!(%error, "IPC client disconnected");
            }
        });
    }
}

async fn handle_tcp(stream: TcpStream, state: AppState) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut events = state.events.subscribe();
    let mut claimed = false;
    loop {
        tokio::select! {
            line = read_ipc_line(&mut reader) => match line? {
                Some(line) => match serde_json::from_str::<Envelope>(&line) {
                    Ok(message) => {
                        let was_claimed = claimed;
                        if let Err(error) = accept_client_message(&state, message, &mut claimed).await {
                            write_direct_error(&mut writer, error.to_string()).await?;
                            break;
                        }
                        if !was_claimed && claimed { write_ready(&mut writer, &state).await?; }
                    },
                    Err(error) => {
                        write_direct_error(&mut writer, format!("malformed protocol message: {error}")).await?;
                        break;
                    }
                },
                None => break
            },
            event = events.recv() => if let Ok(event) = event { if let Some(event) = for_game(event) { writer.write_all(serde_json::to_string(&event)?.as_bytes()).await?; writer.write_all(b"\n").await?; } }
        }
    }
    Ok(())
}

#[cfg(windows)]
pub async fn run_named_pipe(state: AppState) -> Result<()> {
    let client_slots = Arc::new(Semaphore::new(MAX_IPC_CLIENTS));
    loop {
        let server = match create_owner_only_named_pipe(r"\\.\pipe\beatblock-online-v3") {
            Ok(server) => server,
            Err(error) => {
                tracing::warn!(%error, "named-pipe IPC listener creation failed; retrying");
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if let Err(error) = server.connect().await {
            tracing::warn!(%error, "named-pipe IPC listener connect failed; retrying");
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        let Ok(client_slot) = client_slots.clone().try_acquire_owned() else {
            drop(server);
            sleep(Duration::from_millis(50)).await;
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _client_slot = client_slot;
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = BufReader::new(reader);
            let mut events = state.events.subscribe();
            let mut claimed = false;
            loop {
                tokio::select! {
                    line = read_ipc_line(&mut reader) => match line {
                        Ok(Some(line)) => match serde_json::from_str::<Envelope>(&line) {
                            Ok(message) => {
                                let was_claimed = claimed;
                                if let Err(error) = accept_client_message(&state, message, &mut claimed).await {
                                    let _ = write_direct_error(&mut writer, error.to_string()).await;
                                    break;
                                }
                                if !was_claimed && claimed && write_ready(&mut writer, &state).await.is_err() { break; }
                            },
                            Err(error) => {
                                let _ = write_direct_error(&mut writer, format!("malformed protocol message: {error}")).await;
                                break;
                            }
                        },
                        _ => break
                    },
                    event = events.recv() => if let Ok(event) = event { if let Some(event) = for_game(event) { if writer.write_all(serde_json::to_string(&event).unwrap_or_default().as_bytes()).await.is_err() { break; } let _ = writer.write_all(b"\n").await; } }
                }
            }
        });
    }
}

#[cfg(windows)]
fn create_owner_only_named_pipe(
    name: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use std::{ffi::c_void, ptr};
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
    };

    // SYSTEM and the object owner are the only principals granted access.
    // The game and runtime run as the same Windows user, while another local
    // account cannot race client.hello and seize the control channel.
    let sddl = "D:P(A;;GA;;;SY)(A;;GA;;;OW)"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(std::io::Error::last_os_error())
            .context("create owner-only named-pipe security descriptor");
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(false)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
            )
    };
    unsafe {
        LocalFree(descriptor);
    }
    result.context("create owner-only named-pipe IPC listener")
}

/// Reads one newline-delimited IPC message without allowing a malformed local
/// client to grow the process heap indefinitely before sending a delimiter.
async fn read_ipc_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>> {
    let mut bytes = Vec::with_capacity(4 * 1024);
    let read = reader
        .take((MAX_IPC_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_IPC_FRAME {
        anyhow::bail!("IPC protocol message exceeds the 1 MiB safety limit");
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    Ok(Some(
        String::from_utf8(bytes).context("IPC message is not UTF-8")?,
    ))
}

async fn accept_client_message(
    state: &AppState,
    message: Envelope,
    claimed: &mut bool,
) -> Result<()> {
    if !*claimed {
        if message.kind != "client.hello" {
            anyhow::bail!("client.hello must be the first IPC message");
        }
        let client_id = message
            .payload
            .get("instanceId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !state.claim_ipc_client(client_id).await {
            anyhow::bail!("another Beatblock instance owns this Online runtime");
        }
        *claimed = true;
    }
    if crate::game_commands::is_control_command(&message.kind) {
        let bypass_busy = message.kind == "runtime.session_end";
        if !bypass_busy && !state.try_begin_control() {
            let _ = state.events.send(Envelope::new(
                "control.error",
                0,
                serde_json::json!({
                    "code":"control.busy",
                    "stage":"queued",
                    "retryable":true,
                    "requestId":message.request_id,
                    "command":message.kind,
                    "message":"Another Online action is still finishing"
                }),
            ));
            return Ok(());
        }
        let state = state.clone();
        tokio::spawn(async move {
            // Drop-based release also runs if a command handler unwinds, so one
            // bad action cannot leave every later GUI control permanently busy.
            let _control_guard = ControlGuard((!bypass_busy).then(|| state.clone()));
            let request_id = message.request_id.clone();
            if let Err(error) = state.ingest(message).await {
                let _ = state.events.send(Envelope::new(
                    "runtime.error",
                    0,
                    serde_json::json!({
                        "code":"runtime.command_dispatch_failed",
                        "requestId":request_id,
                        "message":error.to_string()
                    }),
                ));
            }
        });
        Ok(())
    } else {
        state.ingest(message).await
    }
}

async fn write_direct_error<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    message: String,
) -> Result<()> {
    let code = if message.contains("owns this Online runtime") {
        "client.runtime_busy"
    } else if message.contains("client.hello") {
        "client.handshake_required"
    } else {
        "protocol.malformed"
    };
    let error = Envelope::new(
        "runtime.error",
        0,
        serde_json::json!({"code":code,"message":message}),
    );
    writer
        .write_all(serde_json::to_string(&error)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    Ok(())
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
            | "control.progress"
            | "runtime.snapshot"
            | "runtime.heartbeat"
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
            | "broadcast.plan"
            | "broadcast.revoked"
            | "chart.transfer_offer"
            | "chart.transfer_progress"
            | "chart.transfer_complete"
            | "chart.transfer_failed"
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
    use crate::model::CompanionConfig;

    #[cfg(windows)]
    #[tokio::test]
    async fn owner_only_named_pipe_accepts_the_creating_windows_user() {
        use tokio::net::windows::named_pipe::ClientOptions;

        let name = format!(r"\\.\pipe\bbt-ipc-acl-{}", uuid::Uuid::new_v4());
        let _server = create_owner_only_named_pipe(&name).unwrap();
        let _client = ClientOptions::new().open(&name).unwrap();
    }

    #[tokio::test]
    async fn ipc_reader_rejects_oversized_lines_without_unbounded_buffering() {
        let bytes = vec![b'x'; MAX_IPC_FRAME + 1];
        let mut reader = BufReader::new(bytes.as_slice());
        let error = read_ipc_line(&mut reader).await.unwrap_err().to_string();
        assert!(error.contains("1 MiB"));
    }

    #[test]
    fn game_ipc_does_not_echo_high_rate_client_telemetry() {
        assert!(for_game(Envelope::new("render.sample", 1, serde_json::json!({}))).is_none());
        assert!(for_game(Envelope::new("gameplay.snapshot", 2, serde_json::json!({}))).is_none());
        assert!(for_game(Envelope::new("runtime.snapshot", 3, serde_json::json!({}))).is_some());
        assert!(for_game(Envelope::new("control.ack", 4, serde_json::json!({}))).is_some());
    }

    #[test]
    fn game_ipc_forwards_transfer_and_broadcast_state() {
        for (sequence, kind) in [
            "broadcast.plan",
            "broadcast.revoked",
            "chart.transfer_offer",
            "chart.transfer_progress",
            "chart.transfer_complete",
            "chart.transfer_failed",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                for_game(Envelope::new(kind, sequence as u64, serde_json::json!({}))).is_some(),
                "{kind} must reach the in-game Online UI"
            );
        }
    }

    #[tokio::test]
    async fn first_game_instance_owns_runtime_and_reconnects_keep_ownership() {
        let root = std::env::temp_dir().join(format!("bbt-ipc-owner-{}", rand::random::<u64>()));
        let state = AppState::new(root.clone(), "token".into(), CompanionConfig::default())
            .unwrap()
            .0;
        let hello = |id: &str| {
            Envelope::new(
                "client.hello",
                1,
                serde_json::json!({
                    "instanceId":id,
                    "clientVersion":"test",
                    "gameVersion":"1.7.1a (Early Access)[d40b7083]"
                }),
            )
        };
        let mut first = false;
        accept_client_message(&state, hello("game-a"), &mut first)
            .await
            .unwrap();
        let mut reconnect = false;
        accept_client_message(&state, hello("game-a"), &mut reconnect)
            .await
            .unwrap();
        let mut second = false;
        let error = accept_client_message(&state, hello("game-b"), &mut second)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("owns this Online runtime"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn control_dispatch_does_not_block_ipc_progress_or_heartbeats() {
        let root = std::env::temp_dir().join(format!("bbt-ipc-control-{}", rand::random::<u64>()));
        let state = AppState::new(root.clone(), "token".into(), CompanionConfig::default())
            .unwrap()
            .0;
        let mut claimed = false;
        accept_client_message(
            &state,
            Envelope::new(
                "client.hello",
                0,
                serde_json::json!({
                    "instanceId":"game-a",
                    "gameVersion":"1.7.1a (Early Access)[d40b7083]"
                }),
            ),
            &mut claimed,
        )
        .await
        .unwrap();
        let mut events = state.events.subscribe();
        let started = std::time::Instant::now();
        accept_client_message(
            &state,
            Envelope::new(
                "room.host_request",
                1,
                serde_json::json!({"requestId":"bad-host","name":"Missing password"}),
            ),
            &mut claimed,
        )
        .await
        .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.kind == "control.error" {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(error.payload["requestId"], "bad-host");
        let _ = std::fs::remove_dir_all(root);
    }
}
