use std::{
    env,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use interprocess::{
    ConnectWaitMode,
    local_socket::{
        ConnectOptions, GenericFilePath, ToFsName,
        traits::{Stream as _, StreamCommon as _},
    },
};
use serde::{Deserialize, Serialize};

const PRESENCE_VERSION: u32 = 2;
const STALE_GRACE: Duration = Duration::from_secs(6);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
const READ_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RECONNECT_MIN: Duration = Duration::from_millis(20);
const RECONNECT_MAX: Duration = Duration::from_secs(1);
const LEGACY_POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const PARTIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct NormPresence {
    socket_path: PathBuf,
    snapshot: Option<PresenceSnapshot>,
    connection_failed_at: Option<Instant>,
    request_generation: u64,
    workspace_changes: Vec<NormWorkspaceChange>,
    completion: Arc<Mutex<Option<PresenceCompletion>>>,
    open_tab_completion: Arc<Mutex<Option<Result<(), String>>>>,
    open_tab_running: bool,
    shutdown_tx: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
    #[cfg(test)]
    disabled: bool,
}

impl NormPresence {
    pub(crate) fn new() -> Self {
        Self::with_socket_path(daemon_socket_path())
    }

    fn with_socket_path(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            snapshot: None,
            connection_failed_at: None,
            request_generation: 0,
            workspace_changes: Vec::new(),
            completion: Arc::new(Mutex::new(None)),
            open_tab_completion: Arc::new(Mutex::new(None)),
            open_tab_running: false,
            shutdown_tx: None,
            worker: None,
            #[cfg(test)]
            disabled: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn disabled_for_test(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub(crate) fn poll(&mut self) -> bool {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> bool {
        #[cfg(test)]
        if self.disabled {
            return false;
        }

        self.start_worker();
        let completion = self
            .completion
            .lock()
            .expect("Norm presence completion lock poisoned")
            .take();
        let mut changed =
            completion.is_some_and(|completion| self.accept_completion(completion, now));
        changed |= self.expire_stale_snapshot(now);
        changed
    }

    fn start_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }
        self.request_generation = self.request_generation.wrapping_add(1);
        let generation = self.request_generation;
        let socket_path = self.socket_path.clone();
        let completion = self.completion.clone();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        self.shutdown_tx = Some(shutdown_tx);
        self.worker = Some(thread::spawn(move || {
            stream_presence(socket_path, generation, completion, shutdown_rx);
        }));
    }

    fn accept_completion(&mut self, completion: PresenceCompletion, now: Instant) -> bool {
        if completion.generation != self.request_generation {
            return false;
        }
        match completion.outcome {
            FetchOutcome::Snapshot(snapshot) => {
                let changed = self.snapshot.as_ref() != Some(&snapshot);
                self.workspace_changes
                    .extend(active_workspace_changes(self.snapshot.as_ref(), &snapshot));
                self.snapshot = Some(snapshot);
                self.connection_failed_at = None;
                changed
            }
            FetchOutcome::Absent => {
                self.connection_failed_at = None;
                self.snapshot.take().is_some()
            }
            FetchOutcome::Transient => {
                self.connection_failed_at
                    .get_or_insert(completion.observed_at);
                self.expire_stale_snapshot(now)
            }
        }
    }

    fn expire_stale_snapshot(&mut self, now: Instant) -> bool {
        let expired = self
            .connection_failed_at
            .is_some_and(|failure| now.saturating_duration_since(failure) >= STALE_GRACE);
        if !expired {
            return false;
        }
        self.connection_failed_at = None;
        self.snapshot.take().is_some()
    }

    pub(crate) fn take_workspace_changes(&mut self) -> Vec<NormWorkspaceChange> {
        std::mem::take(&mut self.workspace_changes)
    }

    pub(crate) fn open_tab(&mut self, workspace: PathBuf) -> Result<(), String> {
        let Some(snapshot) = self
            .snapshot
            .as_ref()
            .filter(|snapshot| !snapshot.instances.is_empty())
        else {
            return Err("No running Norm TUI is available".into());
        };
        if self.open_tab_running {
            return Err("a Norm tab is already being created".into());
        }
        let instance_id = snapshot
            .instances
            .iter()
            .find(|instance| instance.tabs.iter().any(|tab| tab.workspace == workspace))
            .or_else(|| snapshot.instances.first())
            .map(|instance| instance.instance_id.clone());
        let completion = self.open_tab_completion.clone();
        self.open_tab_running = true;
        thread::spawn(move || {
            let mut command = Command::new("norm");
            command.arg("open");
            if let Some(instance_id) = instance_id {
                command.arg("--instance").arg(instance_id);
            }
            let result = command
                .arg(&workspace)
                .output()
                .map_err(|error| format!("could not run Norm: {error}"))
                .and_then(|output| {
                    if output.status.success() {
                        Ok(())
                    } else {
                        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                        Err(if detail.is_empty() {
                            format!("Norm could not open a tab for {}", workspace.display())
                        } else {
                            detail
                        })
                    }
                });
            *completion
                .lock()
                .expect("Norm tab completion lock poisoned") = Some(result);
        });
        Ok(())
    }

    pub(crate) fn take_open_tab_completion(&mut self) -> Option<Result<(), String>> {
        let completion = self
            .open_tab_completion
            .lock()
            .expect("Norm tab completion lock poisoned")
            .take();
        if completion.is_some() {
            self.open_tab_running = false;
        }
        completion
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown_tx.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_snapshot_for_test(&mut self, response: &str) {
        let now = Instant::now();
        self.accept_completion(
            PresenceCompletion {
                generation: self.request_generation,
                observed_at: now,
                outcome: FetchOutcome::Snapshot(
                    parse_response(response.as_bytes()).expect("valid Norm test presence"),
                ),
            },
            now,
        );
    }
}

impl Drop for NormPresence {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PresenceSnapshot {
    daemon_epoch: String,
    revision: u64,
    watch: bool,
    instances: Vec<NormInstance>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NormWorkspaceChange {
    pub(crate) instance_id: String,
    pub(crate) workspace: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NormInstance {
    instance_id: String,
    active_tab_id: Option<u64>,
    tabs: Vec<NormTab>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NormTab {
    tab_id: u64,
    workspace: PathBuf,
}

struct PresenceCompletion {
    generation: u64,
    observed_at: Instant,
    outcome: FetchOutcome,
}

enum FetchOutcome {
    Snapshot(PresenceSnapshot),
    Absent,
    Transient,
}

#[derive(Serialize)]
enum PresenceRequest {
    ListPresence { version: u32, watch: bool },
}

#[derive(Deserialize)]
enum PresenceResponse {
    Presence(PresenceDto),
}

#[derive(Deserialize)]
struct PresenceDto {
    version: u32,
    #[serde(default)]
    watch: bool,
    daemon_epoch: String,
    revision: u64,
    instances: Vec<InstanceDto>,
}

#[derive(Deserialize)]
struct InstanceDto {
    instance_id: String,
    active_tab_id: Option<u64>,
    tabs: Vec<TabDto>,
}

#[derive(Deserialize)]
struct TabDto {
    tab_id: u64,
    workspace: PathBuf,
}

fn stream_presence(
    path: PathBuf,
    generation: u64,
    completion: Arc<Mutex<Option<PresenceCompletion>>>,
    shutdown: mpsc::Receiver<()>,
) {
    let mut reconnect_delay = RECONNECT_MIN;
    let mut failure_started = None;
    loop {
        if shutdown.try_recv().is_ok() {
            return;
        }
        let mut received_snapshot = false;
        let mut stream_confirmed = false;
        let connection_started = Instant::now();
        let failure = match watch_presence_connection(
            &path,
            generation,
            &completion,
            &shutdown,
            &mut received_snapshot,
            &mut stream_confirmed,
        ) {
            Ok(()) => return,
            Err(failure) => failure,
        };
        if received_snapshot {
            failure_started = None;
        }
        let failure_at = *failure_started.get_or_insert_with(Instant::now);
        if !received_snapshot || stream_confirmed {
            publish_completion(
                &completion,
                PresenceCompletion {
                    generation,
                    observed_at: failure_at,
                    outcome: match failure {
                        FetchError::Absent => FetchOutcome::Absent,
                        FetchError::Transient => FetchOutcome::Transient,
                    },
                },
            );
        }
        reconnect_delay = if received_snapshot && !stream_confirmed {
            LEGACY_POLL_INTERVAL
        } else if received_snapshot && connection_started.elapsed() >= LEGACY_POLL_INTERVAL {
            RECONNECT_MIN
        } else {
            reconnect_delay.saturating_mul(2).min(RECONNECT_MAX)
        };
        match shutdown.recv_timeout(reconnect_delay) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn watch_presence_connection(
    path: &Path,
    generation: u64,
    completion: &Arc<Mutex<Option<PresenceCompletion>>>,
    shutdown: &mpsc::Receiver<()>,
    received_snapshot: &mut bool,
    stream_confirmed: &mut bool,
) -> Result<(), FetchError> {
    let name = path
        .to_fs_name::<GenericFilePath>()
        .map_err(|_| FetchError::Transient)?;
    let mut stream = ConnectOptions::new()
        .name(name)
        .wait_mode(ConnectWaitMode::Timeout(CONNECT_TIMEOUT))
        .connect_sync()
        .map_err(classify_connect_error)?;
    if stream
        .peer_creds()
        .map_err(|_| FetchError::Transient)?
        .euid()
        != Some(current_euid())
    {
        return Err(FetchError::Transient);
    }
    stream
        .set_send_timeout(Some(WRITE_TIMEOUT))
        .map_err(|_| FetchError::Transient)?;
    stream
        .set_recv_timeout(Some(READ_POLL_INTERVAL))
        .map_err(|_| FetchError::Transient)?;

    serde_json::to_writer(
        &mut stream,
        &PresenceRequest::ListPresence {
            version: PRESENCE_VERSION,
            watch: true,
        },
    )
    .map_err(|_| FetchError::Transient)?;
    stream
        .write_all(b"\n")
        .and_then(|()| stream.flush())
        .map_err(|_| FetchError::Transient)?;

    let mut pending = Vec::new();
    let mut scanned = 0;
    let mut partial_started = None;
    let mut buffer = [0; 8192];
    loop {
        if shutdown.try_recv().is_ok() {
            return Ok(());
        }
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) =>
            {
                if partial_started
                    .is_some_and(|started: Instant| started.elapsed() >= PARTIAL_FRAME_TIMEOUT)
                {
                    return Err(FetchError::Transient);
                }
                continue;
            }
            Err(_) => return Err(FetchError::Transient),
        };
        if read == 0 {
            return Err(FetchError::Transient);
        }
        pending.extend_from_slice(&buffer[..read]);
        partial_started.get_or_insert_with(Instant::now);
        if partial_started.is_some_and(|started| started.elapsed() >= PARTIAL_FRAME_TIMEOUT) {
            return Err(FetchError::Transient);
        }
        while let Some(relative_end) = pending[scanned..].iter().position(|byte| *byte == b'\n') {
            let line_end = scanned + relative_end;
            if line_end > MAX_RESPONSE_BYTES {
                return Err(FetchError::Transient);
            }
            let remainder = pending.split_off(line_end + 1);
            pending.truncate(line_end);
            let snapshot = parse_response(&pending).map_err(|_| FetchError::Transient)?;
            *stream_confirmed |= snapshot.watch;
            publish_completion(
                completion,
                PresenceCompletion {
                    generation,
                    observed_at: Instant::now(),
                    outcome: FetchOutcome::Snapshot(snapshot),
                },
            );
            *received_snapshot = true;
            pending = remainder;
            scanned = 0;
            partial_started = (!pending.is_empty()).then(Instant::now);
        }
        scanned = pending.len();
        if pending.len() > MAX_RESPONSE_BYTES {
            return Err(FetchError::Transient);
        }
    }
}

fn publish_completion(
    completion: &Arc<Mutex<Option<PresenceCompletion>>>,
    next: PresenceCompletion,
) {
    *completion
        .lock()
        .expect("Norm presence completion lock poisoned") = Some(next);
}

fn parse_response(response: &[u8]) -> Result<PresenceSnapshot, String> {
    let PresenceResponse::Presence(presence) =
        serde_json::from_slice(response).map_err(|error| error.to_string())?;
    if presence.version != PRESENCE_VERSION {
        return Err(format!(
            "unsupported Norm presence version {}",
            presence.version
        ));
    }
    Ok(PresenceSnapshot {
        daemon_epoch: presence.daemon_epoch,
        revision: presence.revision,
        watch: presence.watch,
        instances: presence
            .instances
            .into_iter()
            .map(|instance| NormInstance {
                instance_id: instance.instance_id,
                active_tab_id: instance.active_tab_id,
                tabs: instance
                    .tabs
                    .into_iter()
                    .map(|tab| NormTab {
                        tab_id: tab.tab_id,
                        workspace: tab.workspace,
                    })
                    .collect(),
            })
            .collect(),
    })
}

fn active_workspace_changes(
    previous: Option<&PresenceSnapshot>,
    next: &PresenceSnapshot,
) -> Vec<NormWorkspaceChange> {
    let Some(previous) = previous.filter(|previous| previous.daemon_epoch == next.daemon_epoch)
    else {
        return Vec::new();
    };
    next.instances
        .iter()
        .filter_map(|instance| {
            let previous = previous
                .instances
                .iter()
                .find(|previous| previous.instance_id == instance.instance_id)?;
            let (Some(previous_tab), Some(active_tab)) =
                (previous.active_tab_id, instance.active_tab_id)
            else {
                return None;
            };
            if previous_tab == active_tab {
                return None;
            }
            let tab = instance.tabs.iter().find(|tab| tab.tab_id == active_tab)?;
            Some(NormWorkspaceChange {
                instance_id: instance.instance_id.clone(),
                workspace: tab.workspace.clone(),
            })
        })
        .collect()
}

fn classify_connect_error(error: io::Error) -> FetchError {
    if matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    ) {
        FetchError::Absent
    } else {
        FetchError::Transient
    }
}

#[derive(Clone, Copy)]
enum FetchError {
    Absent,
    Transient,
}

fn daemon_socket_path() -> PathBuf {
    if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(runtime).join("norm/daemon.sock");
    }
    PathBuf::from(format!("/tmp/norm-{}/norm/daemon.sock", current_euid()))
}

fn current_euid() -> libc::uid_t {
    // SAFETY: geteuid has no arguments or safety preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(epoch: &str, active: u64) -> PresenceSnapshot {
        PresenceSnapshot {
            daemon_epoch: epoch.to_owned(),
            revision: active,
            watch: true,
            instances: vec![NormInstance {
                instance_id: "terminal-a".to_owned(),
                active_tab_id: Some(active),
                tabs: vec![
                    NormTab {
                        tab_id: 1,
                        workspace: PathBuf::from("/work/one"),
                    },
                    NormTab {
                        tab_id: 2,
                        workspace: PathBuf::from("/work/two"),
                    },
                ],
            }],
        }
    }

    #[test]
    fn parses_instance_and_workspace_state() {
        let response = br#"{"Presence":{"version":2,"watch":true,"daemon_epoch":"epoch-a","revision":7,"instances":[{"instance_id":"terminal-a","active_tab_id":2,"tabs":[{"tab_id":2,"workspace":"/work/two"}]}]}}"#;
        let snapshot = parse_response(response).unwrap();
        assert_eq!(
            snapshot.instances[0].tabs[0].workspace,
            Path::new("/work/two")
        );
    }

    #[test]
    fn emits_workspace_changes_only_for_tab_switches_in_the_same_daemon() {
        let changes =
            active_workspace_changes(Some(&presence("epoch-a", 1)), &presence("epoch-a", 2));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].workspace, Path::new("/work/two"));
        assert!(active_workspace_changes(None, &presence("epoch-a", 2)).is_empty());
        assert!(
            active_workspace_changes(Some(&presence("epoch-a", 1)), &presence("epoch-b", 2))
                .is_empty()
        );
    }
}
