#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use beatblock_together_companion::{
    app_state::AppState, http, ipc, model::CompanionConfig, room::unix_ms,
};
use clap::Parser;
use directories::ProjectDirs;
use rand::RngCore;
use serde_json::Value;
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

#[derive(Parser, Debug)]
#[command(version, about = "Hidden Beatblock Together online runtime")]
struct Args {
    #[arg(long, default_value_t = 8974)]
    port: u16,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    parent_pid: Option<u32>,
    #[arg(long)]
    session_id: Option<String>,
}

fn data_directory(explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| {
        ProjectDirs::from("org", "BeatblockTogether", "BeatblockTogether")
            .map(|dirs| dirs.data_local_dir().to_owned())
            .unwrap_or_else(|| PathBuf::from("runtime-data"))
    })
}

fn load_config(data_dir: &std::path::Path) -> CompanionConfig {
    let config_path = data_dir.join("config.json");
    let mut config: CompanionConfig = std::fs::read(&config_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    // Runtime does not link installer UI code. It reads the install manifest only
    // to locate Beatblock when launching isolated renderer processes.
    if config.game_directory.is_none() {
        config.game_directory = std::fs::read(data_dir.join("install-manifest.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("gameDirectory")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
    }
    config
}

fn local_token(data_dir: &std::path::Path) -> Result<String> {
    let path = data_dir.join("local-token.txt");
    if path.is_file() {
        return Ok(std::fs::read_to_string(path)?.trim().to_owned());
    }
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    std::fs::write(path, &token)?;
    Ok(token)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let _instance = SingleInstance::acquire()?;
    let data_dir = data_directory(args.data_dir);
    std::fs::create_dir_all(data_dir.join("logs"))?;
    let file_appender = tracing_appender::rolling::daily(data_dir.join("logs"), "runtime.log");
    let (writer, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("beatblock_together_companion=info".parse()?),
        )
        .init();
    tracing::info!(parent_pid = args.parent_pid, session = ?args.session_id, "runtime started");

    let (state, network_events) = AppState::new(
        data_dir.clone(),
        local_token(&data_dir)?,
        load_config(&data_dir),
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .thread_name("bbt-runtime")
        .build()?;
    let handle = runtime.handle().clone();

    let network_state = state.clone();
    handle.spawn(async move { network_state.run_network_events(network_events).await });
    let render_state = state.clone();
    handle.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(8));
        loop {
            tick.tick().await;
            render_state
                .renderer
                .write_aligned_inputs(unix_ms() * 1_000);
        }
    });
    let export_state = state.clone();
    handle.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        loop {
            tick.tick().await;
            if let Some(featured) = export_state.renderer.aligned_featured_state(unix_ms()) {
                let _ = beatblock_together_companion::exports::write_featured_exports(
                    &export_state.data_dir.join("exports"),
                    &featured,
                );
            }
        }
    });
    handle.spawn(ipc::run_tcp(state.clone()));
    #[cfg(windows)]
    handle.spawn(ipc::run_named_pipe(state.clone()));
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", args.port)))
        .context("bind token-protected localhost API")?;
    let http_state = state.clone();
    handle.spawn(async move {
        if let Err(error) = axum::serve(listener, http::router(http_state)).await {
            tracing::error!(%error, "localhost API stopped");
        }
    });

    runtime.block_on(async {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tick.tick().await;
            if state.shutdown_requested.load(Ordering::Acquire) {
                break;
            }
            if let Some(pid) = args.parent_pid {
                if !parent_alive(pid) {
                    tracing::info!(parent_pid = pid, "Beatblock parent exited");
                    break;
                }
            }
        }
        state.renderer.stop_all();
        state.network.shutdown().await;
    });
    tracing::info!("runtime stopped");
    Ok(())
}

#[cfg(windows)]
fn parent_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };
    // SYNCHRONIZE is sufficient to poll the parent without granting VM access.
    let process = unsafe { OpenProcess(0x0010_0000, 0, pid) };
    if process.is_null() {
        return false;
    }
    let status = unsafe { WaitForSingleObject(process, 0) };
    unsafe { CloseHandle(process) };
    status == WAIT_TIMEOUT
}

#[cfg(not(windows))]
fn parent_alive(_pid: u32) -> bool {
    true
}

struct SingleInstance {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl SingleInstance {
    fn acquire() -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::{
                Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
                System::Threading::CreateMutexW,
            };
            let name = std::ffi::OsStr::new("Local\\BeatblockTogetherRuntime-v2")
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
            if handle.is_null() {
                anyhow::bail!("could not create the runtime instance mutex");
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
                anyhow::bail!("Beatblock Together runtime is already active");
            }
            return Ok(Self { handle });
        }
        #[cfg(not(windows))]
        Ok(Self {})
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
