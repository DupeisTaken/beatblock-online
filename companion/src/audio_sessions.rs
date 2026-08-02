use crate::model::AudioIsolationState;
use anyhow::Result;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_RETRY: Duration = Duration::from_millis(50);
const DISCOVERY_STEADY_RETRY: Duration = Duration::from_secs(1);
const RESTORE_RETRIES: usize = 20;

trait SessionControl: Clone {
    fn process_id(&self) -> u32;
    fn session_id(&self) -> &str;
    fn muted(&self) -> Result<bool>;
    fn set_muted(&self, muted: bool) -> Result<()>;
}

#[derive(Clone)]
struct HeldSession<S> {
    session: S,
    original_muted: bool,
}

struct HeldIsolation<S> {
    process_id: u32,
    sessions: HashMap<String, HeldSession<S>>,
}

struct IsolationEngine<S: SessionControl> {
    held: HashMap<String, HeldIsolation<S>>,
}

impl<S: SessionControl> Default for IsolationEngine<S> {
    fn default() -> Self {
        Self {
            held: HashMap::new(),
        }
    }
}

impl<S: SessionControl> IsolationEngine<S> {
    fn isolate_exact(&mut self, key: &str, process_id: u32, sessions: Vec<S>) -> Result<bool> {
        // A slot name is stable across renderer generations, but its PID is
        // not. Never let a lease retained after a failed restoration make the
        // next process appear isolated: the old generation must restore before
        // this key can acquire sessions owned by a different PID.
        if self
            .held
            .get(key)
            .is_some_and(|held| held.process_id != process_id)
        {
            self.restore(key)?;
        }

        let mut seen = self
            .held
            .get(key)
            .map(|held| {
                held.sessions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let mut matches = Vec::new();
        for session in sessions {
            if session.process_id() != process_id || !seen.insert(session.session_id().to_owned()) {
                continue;
            }
            let original_muted = session.muted()?;
            matches.push((
                session.session_id().to_owned(),
                HeldSession {
                    session,
                    original_muted,
                },
            ));
        }
        if matches.is_empty() {
            return Ok(self
                .held
                .get(key)
                .is_some_and(|held| held.process_id == process_id && !held.sessions.is_empty()));
        }
        for (changed, (_, held)) in matches.iter().enumerate() {
            if let Err(error) = held.session.set_muted(true) {
                for (_, rollback) in matches[..changed].iter().rev() {
                    let _ = rollback.session.set_muted(rollback.original_muted);
                }
                return Err(error);
            }
        }
        let held = self
            .held
            .entry(key.to_owned())
            .or_insert_with(|| HeldIsolation {
                process_id,
                sessions: HashMap::new(),
            });
        debug_assert_eq!(held.process_id, process_id);
        held.sessions.extend(matches);
        Ok(true)
    }

    fn restore(&mut self, key: &str) -> Result<bool> {
        let Some(held) = self.held.get(key) else {
            return Ok(false);
        };
        let mut first_error = None;
        for session in held.sessions.values() {
            if let Err(error) = session.session.set_muted(session.original_muted) {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        self.held.remove(key);
        Ok(true)
    }

    fn keys(&self) -> Vec<String> {
        self.held.keys().cloned().collect()
    }

    fn holds_exact(&self, key: &str, process_id: u32) -> bool {
        self.held
            .get(key)
            .is_some_and(|held| held.process_id == process_id && !held.sessions.is_empty())
    }
}

fn restore_with_retries<S: SessionControl>(
    engine: &mut IsolationEngine<S>,
    key: &str,
) -> Result<bool> {
    let mut last_error = None;
    for attempt in 0..RESTORE_RETRIES {
        match engine.restore(key) {
            Ok(restored) => return Ok(restored),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < RESTORE_RETRIES {
            std::thread::sleep(DISCOVERY_RETRY);
        }
    }
    Err(last_error.expect("restore retry loop always records an error"))
}

enum WorkerCommand {
    Isolate {
        key: String,
        process_id: u32,
    },
    Restore {
        key: String,
        complete: mpsc::SyncSender<std::result::Result<(), String>>,
    },
    SetEnabled(bool),
    Shutdown,
}

#[derive(Clone)]
struct PendingIsolation {
    // The request remains in this map after its first successful acquisition.
    // Before acquisition it is a fast discovery request; afterward it is an
    // active lease refreshed at the steady cadence until Restore removes it.
    process_id: u32,
    started: Instant,
    next_attempt: Instant,
}

fn poll_pending_isolations<S, F>(
    engine: &mut IsolationEngine<S>,
    pending: &mut HashMap<String, PendingIsolation>,
    now: Instant,
    mut enumerate: F,
) -> Vec<(String, AudioIsolationState)>
where
    S: SessionControl,
    F: FnMut() -> Result<Vec<S>>,
{
    let mut updates = Vec::new();
    for (key, request) in pending.clone() {
        if now < request.next_attempt {
            continue;
        }
        let result = enumerate()
            .and_then(|sessions| engine.isolate_exact(&key, request.process_id, sessions));
        match result {
            Ok(true) => {
                // Successful discovery becomes an active lease. Keep polling
                // it at the steady cadence so sessions created later on a new
                // endpoint are muted and remembered for restoration too.
                if let Some(request) = pending.get_mut(&key) {
                    request.next_attempt = now + DISCOVERY_STEADY_RETRY;
                }
                updates.push((
                    key,
                    AudioIsolationState {
                        status: "muted".into(),
                        muted: true,
                        error: None,
                    },
                ));
            }
            Ok(false) => {
                let initial_window_elapsed =
                    now.saturating_duration_since(request.started) >= DISCOVERY_TIMEOUT;
                if let Some(request) = pending.get_mut(&key) {
                    request.next_attempt = now
                        + if initial_window_elapsed {
                            DISCOVERY_STEADY_RETRY
                        } else {
                            DISCOVERY_RETRY
                        };
                }
                if initial_window_elapsed {
                    updates.push((
                        key,
                        AudioIsolationState {
                            status: "warning".into(),
                            muted: false,
                            error: Some(
                                "renderer audio session was not found by exact process id yet; desktop playback is unchanged and discovery will continue while the renderer is running"
                                    .into(),
                            ),
                        },
                    ));
                }
            }
            Err(error) => {
                let held_current_sessions = engine.holds_exact(&key, request.process_id);
                let initial_window_elapsed =
                    now.saturating_duration_since(request.started) >= DISCOVERY_TIMEOUT;
                let warning = initial_window_elapsed || held_current_sessions;
                if let Some(request) = pending.get_mut(&key) {
                    request.next_attempt = now
                        + if warning {
                            DISCOVERY_STEADY_RETRY
                        } else {
                            DISCOVERY_RETRY
                        };
                }
                updates.push((
                    key,
                    AudioIsolationState {
                        status: if warning {
                            "warning".into()
                        } else {
                            "pending".into()
                        },
                        muted: held_current_sessions,
                        error: Some(if held_current_sessions {
                            format!(
                                "renderer audio isolation refresh failed; existing exact-process sessions remain muted, but newly created sessions may be audible until discovery recovers: {error}"
                            )
                        } else if initial_window_elapsed {
                            format!(
                                "renderer audio isolation failed; desktop playback is unchanged and exact-process discovery will continue: {error}"
                            )
                        } else {
                            error.to_string()
                        }),
                    },
                ));
            }
        }
    }
    updates
}

pub struct AudioIsolationWorker {
    tx: mpsc::Sender<WorkerCommand>,
    states: Arc<Mutex<HashMap<String, AudioIsolationState>>>,
    enabled: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// `false` when the background thread could not be started at all (e.g.
    /// the OS refused the spawn). In that state every operation degrades
    /// gracefully: `isolate` records a warning, sends are silently dropped,
    /// and the runtime continues with desktop audio unchanged.
    available: bool,
}

impl AudioIsolationWorker {
    pub fn new(enabled: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let states = Arc::new(Mutex::new(HashMap::new()));
        let enabled_state = Arc::new(AtomicBool::new(enabled));
        match spawn_worker(rx, states.clone(), enabled_state.clone()) {
            Ok(worker) => Self {
                tx,
                states,
                enabled: enabled_state,
                worker: Some(worker),
                available: true,
            },
            Err(error) => {
                // `rx` was dropped when spawn failed, so any future `tx.send`
                // returns Err immediately and is silently discarded by the
                // callers below. Log once so the condition is visible in
                // runtime logs without crashing.
                tracing::warn!(
                    %error,
                    "renderer audio isolation worker could not be started; desktop audio stays unchanged for every renderer slot"
                );
                Self {
                    tx,
                    states,
                    enabled: enabled_state,
                    worker: None,
                    available: false,
                }
            }
        }
    }

    /// Returns `false` when the background thread failed to start. In that
    /// case audio isolation is silently unavailable for the lifetime of this
    /// worker.
    pub fn available(&self) -> bool {
        self.available
    }

    /// Construct a worker that is permanently unavailable, as if the OS had
    /// refused the thread spawn. Used only by unit tests to exercise the
    /// degraded path without actually exhausting thread resources.
    #[cfg(test)]
    fn new_unavailable() -> Self {
        let (tx, rx) = mpsc::channel::<WorkerCommand>();
        // Drop `rx` immediately so every `tx.send` returns `Err` at once,
        // exactly matching the real spawn-failure path.
        drop(rx);
        Self {
            tx,
            states: Arc::new(Mutex::new(HashMap::new())),
            enabled: Arc::new(AtomicBool::new(true)),
            worker: None,
            available: false,
        }
    }

    pub fn isolate(&self, key: &str, process_id: u32) {
        let key = key.to_ascii_uppercase();
        if !self.available {
            self.states.lock().expect("audio states poisoned").insert(
                key,
                AudioIsolationState {
                    status: "warning".into(),
                    muted: false,
                    error: Some(
                        "renderer desktop audio isolation is unavailable: \
                         the worker thread could not be started"
                            .into(),
                    ),
                },
            );
            return;
        }
        if !self.enabled.load(Ordering::Acquire) {
            self.states.lock().expect("audio states poisoned").insert(
                key,
                AudioIsolationState {
                    status: "disabled".into(),
                    muted: false,
                    error: None,
                },
            );
            return;
        }
        self.states.lock().expect("audio states poisoned").insert(
            key.clone(),
            AudioIsolationState {
                status: "pending".into(),
                muted: false,
                error: None,
            },
        );
        let _ = self.tx.send(WorkerCommand::Isolate { key, process_id });
    }

    pub fn restore(&self, key: &str) {
        let key = key.to_ascii_uppercase();
        let (complete, wait) = mpsc::sync_channel(0);
        let outcome = if self
            .tx
            .send(WorkerCommand::Restore {
                key: key.clone(),
                complete,
            })
            .is_ok()
        {
            wait.recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|error| {
                    Err(format!("renderer mute restoration timed out: {error}"))
                })
        } else {
            Err("renderer mute restoration worker is unavailable".into())
        };
        let mut states = self.states.lock().expect("audio states poisoned");
        match outcome {
            Ok(()) => {
                states.remove(&key);
            }
            Err(error) => {
                states.insert(
                    key,
                    AudioIsolationState {
                        status: "warning".into(),
                        muted: false,
                        error: Some(error),
                    },
                );
            }
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        let _ = self.tx.send(WorkerCommand::SetEnabled(enabled));
    }

    pub fn state(&self, key: &str) -> Option<AudioIsolationState> {
        self.states
            .lock()
            .expect("audio states poisoned")
            .get(&key.to_ascii_uppercase())
            .cloned()
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

impl Drop for AudioIsolationWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
#[derive(Clone)]
struct WindowsSession {
    process_id: u32,
    session_id: String,
    volume: windows::Win32::Media::Audio::ISimpleAudioVolume,
}

#[cfg(windows)]
impl SessionControl for WindowsSession {
    fn process_id(&self) -> u32 {
        self.process_id
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn muted(&self) -> Result<bool> {
        Ok(unsafe { self.volume.GetMute() }?.as_bool())
    }
    fn set_muted(&self, muted: bool) -> Result<()> {
        unsafe { self.volume.SetMute(muted, std::ptr::null()) }?;
        Ok(())
    }
}

#[cfg(windows)]
fn enumerate_windows_sessions() -> Result<Vec<WindowsSession>> {
    use windows::{
        core::Interface,
        Win32::{
            Media::Audio::{
                eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
                ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
            },
            System::Com::{CoCreateInstance, CLSCTX_ALL},
        },
    };
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
    let endpoints = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
    let endpoint_count = unsafe { endpoints.GetCount() }?;
    let mut result = Vec::new();
    for endpoint_index in 0..endpoint_count {
        let endpoint = unsafe { endpoints.Item(endpoint_index) }?;
        let manager: IAudioSessionManager2 = unsafe { endpoint.Activate(CLSCTX_ALL, None) }?;
        let sessions = unsafe { manager.GetSessionEnumerator() }?;
        let count = unsafe { sessions.GetCount() }?;
        for session_index in 0..count {
            let control = unsafe { sessions.GetSession(session_index) }?;
            let control2: IAudioSessionControl2 = control.cast()?;
            let process_id = unsafe { control2.GetProcessId() }?;
            let identifier = unsafe { control2.GetSessionInstanceIdentifier() }?;
            let session_id = unsafe { identifier.to_string() };
            unsafe {
                windows::Win32::System::Com::CoTaskMemFree(Some(identifier.0.cast()));
            }
            let session_id = session_id?;
            let volume: ISimpleAudioVolume = control.cast()?;
            result.push(WindowsSession {
                process_id,
                session_id,
                volume,
            });
        }
    }
    Ok(result)
}

#[cfg(windows)]
fn spawn_worker(
    rx: mpsc::Receiver<WorkerCommand>,
    states: Arc<Mutex<HashMap<String, AudioIsolationState>>>,
    enabled: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>, std::io::Error> {
    std::thread::Builder::new()
        .name("bbt-renderer-audio".into())
        .spawn(move || {
            use windows::Win32::System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED,
            };
            let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let mut engine = IsolationEngine::<WindowsSession>::default();
            let mut pending = HashMap::<String, PendingIsolation>::new();
            loop {
                match rx.recv_timeout(DISCOVERY_RETRY) {
                    Ok(WorkerCommand::Isolate { key, process_id }) => {
                        if enabled.load(Ordering::Acquire) && initialized {
                            let now = Instant::now();
                            pending.insert(
                                key,
                                PendingIsolation {
                                    process_id,
                                    started: now,
                                    next_attempt: now,
                                },
                            );
                        } else if enabled.load(Ordering::Acquire) {
                            states.lock().expect("audio states poisoned").insert(
                                key,
                                AudioIsolationState {
                                    status: "warning".into(),
                                    muted: false,
                                    error: Some(
                                        "Windows Core Audio initialization failed; desktop playback was left unchanged"
                                            .into(),
                                    ),
                                },
                            );
                        }
                    }
                    Ok(WorkerCommand::Restore { key, complete }) => {
                        pending.remove(&key);
                        let result = restore_with_retries(&mut engine, &key)
                            .map(|_| ())
                            .map_err(|error| {
                                format!("renderer mute restoration failed: {error}")
                            });
                        let _ = complete.send(result);
                    }
                    Ok(WorkerCommand::SetEnabled(next)) => {
                        if !next {
                            pending.clear();
                            for key in engine.keys() {
                                let state = match restore_with_retries(&mut engine, &key) {
                                    Ok(_) => AudioIsolationState {
                                        status: "disabled".into(),
                                        muted: false,
                                        error: None,
                                    },
                                    Err(error) => AudioIsolationState {
                                        status: "warning".into(),
                                        muted: false,
                                        error: Some(format!(
                                            "renderer mute restoration failed while disabling automatic isolation: {error}"
                                        )),
                                    },
                                };
                                states
                                    .lock()
                                    .expect("audio states poisoned")
                                    .insert(key, state);
                            }
                            for state in states
                                .lock()
                                .expect("audio states poisoned")
                                .values_mut()
                            {
                                if state.status != "warning" {
                                    state.status = "disabled".into();
                                    state.muted = false;
                                    state.error = None;
                                }
                            }
                        }
                    }
                    Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        for key in engine.keys() {
                            let _ = restore_with_retries(&mut engine, &key);
                        }
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if !enabled.load(Ordering::Acquire) || !initialized {
                    continue;
                }
                let updates = poll_pending_isolations(
                    &mut engine,
                    &mut pending,
                    Instant::now(),
                    enumerate_windows_sessions,
                );
                if !updates.is_empty() {
                    states
                        .lock()
                        .expect("audio states poisoned")
                        .extend(updates);
                }
            }
            if initialized {
                unsafe { CoUninitialize() };
            }
        })
}

#[cfg(not(windows))]
fn spawn_worker(
    rx: mpsc::Receiver<WorkerCommand>,
    states: Arc<Mutex<HashMap<String, AudioIsolationState>>>,
    _enabled: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>, std::io::Error> {
    Ok(std::thread::spawn(move || {
        while let Ok(command) = rx.recv() {
            match command {
                WorkerCommand::Isolate { key, .. } => {
                    states.lock().expect("audio states poisoned").insert(
                        key,
                        AudioIsolationState {
                            status: "warning".into(),
                            muted: false,
                            error: Some(
                                "renderer desktop muting requires Windows Core Audio".into(),
                            ),
                        },
                    );
                }
                WorkerCommand::Restore { complete, .. } => {
                    let _ = complete.send(Ok(()));
                }
                WorkerCommand::Shutdown => break,
                WorkerCommand::SetEnabled(_) => {}
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static NEXT_FAKE_SESSION_ID: AtomicUsize = AtomicUsize::new(1);

    #[derive(Clone)]
    struct FakeSession {
        session_id: String,
        process_id: u32,
        muted: Arc<Mutex<bool>>,
        fail_writes: Arc<Mutex<usize>>,
    }

    impl FakeSession {
        fn new(process_id: u32, muted: bool) -> Self {
            let ordinal = NEXT_FAKE_SESSION_ID.fetch_add(1, Ordering::Relaxed);
            Self::identified(format!("pid-{process_id}-{ordinal}"), process_id, muted)
        }

        fn identified(session_id: impl Into<String>, process_id: u32, muted: bool) -> Self {
            Self {
                session_id: session_id.into(),
                process_id,
                muted: Arc::new(Mutex::new(muted)),
                fail_writes: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl SessionControl for FakeSession {
        fn process_id(&self) -> u32 {
            self.process_id
        }
        fn session_id(&self) -> &str {
            &self.session_id
        }
        fn muted(&self) -> Result<bool> {
            Ok(*self.muted.lock().unwrap())
        }
        fn set_muted(&self, muted: bool) -> Result<()> {
            let mut failures = self.fail_writes.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                anyhow::bail!("injected audio-session failure");
            }
            *self.muted.lock().unwrap() = muted;
            Ok(())
        }
    }

    #[test]
    fn exact_pid_isolation_never_changes_the_host_and_restores_original_states() {
        let host = FakeSession::new(100, false);
        let child = FakeSession::new(200, false);
        let already_muted = FakeSession::new(200, true);
        let mut engine = IsolationEngine::default();
        assert!(engine
            .isolate_exact(
                "A",
                200,
                vec![host.clone(), child.clone(), already_muted.clone()]
            )
            .unwrap());
        assert!(!host.muted().unwrap());
        assert!(child.muted().unwrap());
        assert!(already_muted.muted().unwrap());
        assert!(engine.restore("A").unwrap());
        assert!(!child.muted().unwrap());
        assert!(already_muted.muted().unwrap());
    }

    #[test]
    fn missing_pid_leaves_playback_unchanged_and_restore_retries_are_safe() {
        let host = FakeSession::new(100, false);
        let mut engine = IsolationEngine::default();
        assert!(!engine.isolate_exact("A", 200, vec![host.clone()]).unwrap());
        assert!(!host.muted().unwrap());

        let child = FakeSession::new(200, false);
        engine
            .isolate_exact("AUTOPLAY", 200, vec![child.clone()])
            .unwrap();
        *child.fail_writes.lock().unwrap() = 1;
        assert!(restore_with_retries(&mut engine, "AUTOPLAY").unwrap());
        assert!(!child.muted().unwrap());
    }

    #[test]
    fn partial_isolation_failure_rolls_back_every_session_changed_by_the_worker() {
        let first = FakeSession::new(200, false);
        let second = FakeSession::new(200, false);
        *second.fail_writes.lock().unwrap() = 1;
        let mut engine = IsolationEngine::default();
        assert!(engine
            .isolate_exact("A", 200, vec![first.clone(), second.clone()])
            .is_err());
        assert!(!first.muted().unwrap());
        assert!(!second.muted().unwrap());
        assert!(engine.keys().is_empty());
    }

    #[test]
    fn delayed_audio_session_remains_discoverable_after_the_initial_window() {
        let host = FakeSession::new(100, false);
        let child = FakeSession::new(200, false);
        let started = Instant::now();
        let mut pending = HashMap::from([(
            "A".into(),
            PendingIsolation {
                process_id: 200,
                started,
                next_attempt: started,
            },
        )]);
        let mut engine = IsolationEngine::default();

        let updates = poll_pending_isolations(&mut engine, &mut pending, started, || {
            Ok(vec![host.clone()])
        });
        assert!(updates.is_empty());
        assert!(pending.contains_key("A"));

        let after_initial_window = started + DISCOVERY_TIMEOUT + Duration::from_millis(1);
        let updates =
            poll_pending_isolations(&mut engine, &mut pending, after_initial_window, || {
                Ok(vec![host.clone()])
            });
        assert_eq!(updates[0].1.status, "warning");
        assert!(pending.contains_key("A"));
        assert!(!host.muted().unwrap());

        let after_steady_retry = after_initial_window + DISCOVERY_STEADY_RETRY;
        let updates =
            poll_pending_isolations(&mut engine, &mut pending, after_steady_retry, || {
                Ok(vec![host.clone(), child.clone()])
            });
        assert_eq!(updates[0].1.status, "muted");
        assert!(pending.contains_key("A"));
        assert!(!host.muted().unwrap());
        assert!(child.muted().unwrap());
    }

    #[test]
    fn active_isolation_discovers_late_sessions_and_restores_every_original_state() {
        let host = FakeSession::identified("host", 100, false);
        let first = FakeSession::identified("first", 200, false);
        let late = FakeSession::identified("late", 200, false);
        let started = Instant::now();
        let mut pending = HashMap::from([(
            "A".into(),
            PendingIsolation {
                process_id: 200,
                started,
                next_attempt: started,
            },
        )]);
        let mut engine = IsolationEngine::default();

        let updates = poll_pending_isolations(&mut engine, &mut pending, started, || {
            Ok(vec![host.clone(), first.clone()])
        });
        assert_eq!(updates[0].1.status, "muted");
        assert!(pending.contains_key("A"), "active leases must keep polling");
        assert!(!host.muted().unwrap());
        assert!(first.muted().unwrap());

        let refresh_error_at = started + DISCOVERY_STEADY_RETRY;
        let updates = poll_pending_isolations(&mut engine, &mut pending, refresh_error_at, || {
            anyhow::bail!("injected enumeration failure")
        });
        assert_eq!(updates[0].1.status, "warning");
        assert!(updates[0].1.muted, "the existing lease remains effective");
        assert!(first.muted().unwrap());

        let updates = poll_pending_isolations(
            &mut engine,
            &mut pending,
            refresh_error_at + DISCOVERY_STEADY_RETRY,
            || Ok(vec![host.clone(), first.clone(), late.clone()]),
        );
        assert_eq!(updates[0].1.status, "muted");
        assert!(!host.muted().unwrap());
        assert!(late.muted().unwrap());

        assert!(engine.restore("A").unwrap());
        assert!(!host.muted().unwrap());
        assert!(
            !first.muted().unwrap(),
            "re-enumeration must not replace the first original state"
        );
        assert!(
            !late.muted().unwrap(),
            "late session must regain its original state"
        );
    }

    #[test]
    fn failed_old_pid_restoration_never_marks_or_mutes_a_reassigned_pid() {
        let old = FakeSession::identified("old", 200, false);
        let replacement = FakeSession::identified("replacement", 300, false);
        let mut engine = IsolationEngine::default();
        assert!(engine.isolate_exact("A", 200, vec![old.clone()]).unwrap());
        *old.fail_writes.lock().unwrap() = RESTORE_RETRIES + 5;

        assert!(restore_with_retries(&mut engine, "A").is_err());
        assert!(old.muted().unwrap());
        assert!(engine
            .isolate_exact("A", 300, vec![replacement.clone()])
            .is_err());
        assert!(old.muted().unwrap());
        assert!(!replacement.muted().unwrap());

        let started = Instant::now() - DISCOVERY_TIMEOUT;
        let mut pending = HashMap::from([(
            "A".into(),
            PendingIsolation {
                process_id: 300,
                started,
                next_attempt: started,
            },
        )]);
        let updates = poll_pending_isolations(&mut engine, &mut pending, Instant::now(), || {
            Ok(vec![replacement.clone()])
        });
        assert_eq!(updates[0].1.status, "warning");
        assert!(!updates[0].1.muted, "old PID ownership must not transfer");
        assert!(pending.contains_key("A"));
        assert!(!replacement.muted().unwrap());
    }

    /// When the worker thread could not be started, every operation must
    /// degrade gracefully: `available()` returns false, `isolate` records a
    /// warning state, and neither `restore` nor `set_enabled` panic.
    #[test]
    fn unavailable_worker_degrades_gracefully_without_panicking() {
        let worker = AudioIsolationWorker::new_unavailable();

        assert!(!worker.available());

        // isolate must record a warning — not "pending" — so callers can see
        // that the isolation request was not submitted to any background thread.
        worker.isolate("slot-a", 1234);
        let state = worker.state("slot-a").expect("state must be recorded");
        assert_eq!(state.status, "warning", "expected warning, got {state:?}");
        assert!(!state.muted);
        assert!(state.error.is_some());

        // restore on an unavailable worker must not panic even though there is
        // nothing to restore and the channel send will fail immediately.
        worker.restore("slot-a");

        // set_enabled likewise must not panic.
        worker.set_enabled(false);
        worker.set_enabled(true);
    }
}
