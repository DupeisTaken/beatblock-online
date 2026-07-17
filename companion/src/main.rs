#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use beatblock_online_companion::{
    app_state::AppState, http, ipc, model::CompanionConfig, room::unix_ms,
};
use clap::Parser;
use directories::ProjectDirs;
use rand::RngCore;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, SystemTime},
};

#[derive(Parser, Debug)]
#[command(version, about = "Hidden Beatblock Online online runtime")]
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
        ProjectDirs::from("org", "BeatblockOnline", "BeatblockOnline")
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
    if let Some(manifest) = std::fs::read(data_dir.join("install-manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    {
        if config.game_directory.is_none() {
            config.game_directory = manifest
                .get("gameDirectory")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        config.firewall_installed = manifest
            .get("firewallInstalled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        config.firewall_public = manifest
            .get("firewallPublic")
            .and_then(Value::as_bool)
            .unwrap_or(false);
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
    prune_managed_files(
        &data_dir.join("logs"),
        Duration::from_secs(14 * 86_400),
        64 * 1024 * 1024,
    );
    prune_managed_files(
        &data_dir.join("chart-cache"),
        Duration::from_secs(30 * 86_400),
        128 * 1024 * 1024,
    );
    let file_appender = tracing_appender::rolling::daily(data_dir.join("logs"), "runtime.log");
    let (writer, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("beatblock_online_companion=info".parse()?),
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
        loop {
            let delay = render_state
                .renderer
                .active_input_fps()
                .map(|fps| Duration::from_micros(1_000_000 / fps.max(1) as u64))
                .unwrap_or(Duration::from_millis(250));
            tokio::time::sleep(delay).await;
            if render_state.renderer.active_input_fps().is_none() {
                continue;
            }
            render_state
                .renderer
                .write_aligned_inputs(unix_ms() * 1_000);
        }
    });
    let health_state = state.clone();
    handle.spawn(async move {
        let mut snapshot_elapsed = Duration::ZERO;
        loop {
            let delay = if health_state.renderer.active_input_fps().is_some() {
                Duration::from_millis(100)
            } else {
                Duration::from_millis(500)
            };
            tokio::time::sleep(delay).await;
            health_state.renderer.refresh_health(unix_ms());
            snapshot_elapsed += delay;
            if snapshot_elapsed >= Duration::from_millis(500) {
                snapshot_elapsed = Duration::ZERO;
                health_state.publish_renderer_snapshot();
            }
        }
    });
    let export_state = state.clone();
    handle.spawn(async move {
        let mut had_featured_state = false;
        loop {
            let delay = if export_state.renderer.has_active_featured_slot() {
                Duration::from_millis(33)
            } else {
                Duration::from_millis(250)
            };
            tokio::time::sleep(delay).await;
            if let Some(featured) = export_state.renderer.aligned_featured_state(unix_ms()) {
                export_state.exports.publish_featured(Some(featured));
                had_featured_state = true;
            } else if had_featured_state {
                export_state.exports.publish_featured(None);
                had_featured_state = false;
            }
        }
    });
    let storage_state = state.clone();
    handle.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if storage_state.storage.has_pending_events() {
                let storage = storage_state.storage.clone();
                let storage_result =
                    tokio::task::spawn_blocking(move || storage.flush_pending_events()).await;
                if let Err(error) = storage_result
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result)
                {
                    tracing::warn!(%error, "journal storage batch failed");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    });
    let persistence_state = state.clone();
    handle.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut room_tick = false;
        let mut last_disconnect_scan = tokio::time::Instant::now();
        let mut room_retry_at = tokio::time::Instant::now();
        let mut last_room_warning = tokio::time::Instant::now() - Duration::from_secs(5);
        loop {
            tick.tick().await;
            room_tick = !room_tick;
            if room_tick && tokio::time::Instant::now() >= room_retry_at {
                if let Err(error) = persistence_state.flush_room_updates().await {
                    room_retry_at = tokio::time::Instant::now() + Duration::from_millis(250);
                    if last_room_warning.elapsed() >= Duration::from_secs(5) {
                        tracing::warn!(%error, "coalesced room publication failed");
                        last_room_warning = tokio::time::Instant::now();
                    }
                }
            }
            if last_disconnect_scan.elapsed() >= Duration::from_secs(1) {
                last_disconnect_scan = tokio::time::Instant::now();
                if let Err(error) = persistence_state.expire_due_disconnects(unix_ms()).await {
                    tracing::warn!(%error, "disconnect expiry publication failed");
                }
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
        if let Err(error) = state.flush_room_updates().await {
            tracing::warn!(%error, "final room publication failed");
        }
        let storage = state.storage.clone();
        if let Err(error) = tokio::task::spawn_blocking(move || storage.flush_pending_events())
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result)
        {
            tracing::warn!(%error, "final journal storage batch failed");
        }
        state.journals.flush();
        state.cancel_background_tasks();
        state.release_nat_mapping().await;
        state.renderer.stop_all();
        state.network.shutdown().await;
        state.exports.flush();
    });
    // A wedged filesystem call inside spawn_blocking must not keep the hidden
    // runtime alive forever after Beatblock has exited.
    runtime.shutdown_timeout(Duration::from_secs(2));
    tracing::info!("runtime stopped");
    Ok(())
}

/// Applies both an age and size ceiling to runtime-owned diagnostic/cache
/// directories. Imported charts, match summaries, and user files are outside
/// these managed paths and are never considered for deletion.
fn prune_managed_files(directory: &Path, max_age: Duration, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    entry.path(),
                    metadata.len(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect::<Vec<_>>();
    files.retain(|(path, _, modified)| {
        if *modified < cutoff {
            let _ = std::fs::remove_file(path);
            false
        } else {
            true
        }
    });
    files.sort_by_key(|(_, _, modified)| std::cmp::Reverse(*modified));
    let mut retained = 0u64;
    for (path, size, _) in files {
        if retained.saturating_add(size) > max_bytes {
            let _ = std::fs::remove_file(path);
        } else {
            retained = retained.saturating_add(size);
        }
    }
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
    handles: Vec<windows_sys::Win32::Foundation::HANDLE>,
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
            let legacy_runtime_stem = concat!("Beatblock", "TogetherRuntime");
            let names = [
                "Local\\BeatblockOnlineRuntime-v3".to_string(),
                format!("Local\\{legacy_runtime_stem}-v3"),
            ];
            let mut handles = Vec::with_capacity(names.len());
            for name in names {
                let name = std::ffi::OsStr::new(&name)
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
                if handle.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    if !handle.is_null() {
                        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
                    }
                    for acquired in handles {
                        unsafe { windows_sys::Win32::Foundation::CloseHandle(acquired) };
                    }
                    if handle.is_null() {
                        anyhow::bail!("could not create the runtime instance mutex");
                    }
                    anyhow::bail!("Beatblock Online runtime is already active");
                }
                handles.push(handle);
            }
            Ok(Self { handles })
        }
        #[cfg(not(windows))]
        Ok(Self {})
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        #[cfg(windows)]
        for handle in &self.handles {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(*handle) };
        }
    }
}
