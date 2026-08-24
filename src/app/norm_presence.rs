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

use super::AgentStatus;

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
    scroll: usize,
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
        Self::with_socket_path(daemon_socket_path(), Instant::now())
    }

    fn with_socket_path(socket_path: PathBuf, _now: Instant) -> Self {
        Self {
            socket_path,
            snapshot: None,
            connection_failed_at: None,
            request_generation: 0,
            scroll: 0,
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
                self.scroll = self.scroll.min(snapshot.agents.len().saturating_sub(1));
                self.snapshot = Some(snapshot);
                self.connection_failed_at = None;
                changed
            }
            FetchOutcome::Absent => {
                self.connection_failed_at = None;
                self.scroll = 0;
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
        self.scroll = 0;
        self.snapshot.take().is_some()
    }

    pub(crate) fn is_available(&self) -> bool {
        self.snapshot.is_some()
    }

    pub(crate) fn agents(&self) -> &[NormAgent] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.agents.as_slice())
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    pub(crate) fn scroll_agents(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta);
    }

    pub(crate) fn take_workspace_changes(&mut self) -> Vec<NormWorkspaceChange> {
        std::mem::take(&mut self.workspace_changes)
    }

    pub(crate) fn open_tab(&mut self, workspace: PathBuf) -> Result<(), String> {
        if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.instances.is_empty())
        {
            return Err("No running Norm TUI is available".into());
        }
        if self.open_tab_running {
            return Err("a Norm agent is already being created".into());
        }
        let instance_id = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .instances
                .iter()
                .find(|instance| instance.tabs.iter().any(|tab| tab.workspace == workspace))
                .or_else(|| snapshot.instances.first())
                .map(|instance| instance.instance_id.clone())
        });
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
                            format!("Norm could not create an agent for {}", workspace.display())
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
    _revision: u64,
    _watch: bool,
    agents: Vec<NormAgent>,
    instances: Vec<NormInstance>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NormWorkspaceChange {
    pub(crate) instance_id: String,
    pub(crate) pane_id: String,
    pub(crate) workspace: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NormAgent {
    pub(crate) identity: NormAgentIdentity,
    pub(crate) workspace: PathBuf,
    pub(crate) view: NormAgentView,
    pub(crate) lifecycle: NormLifecycle,
    pub(crate) activity: NormActivity,
    pub(crate) session_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) open_views: u32,
    _sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NormAgentView {
    ActiveHerdrPane(String),
    InactiveHerdrPane,
    MissingHerdrPane,
    NoView,
}

impl NormAgent {
    pub(crate) fn status(&self) -> AgentStatus {
        match (self.lifecycle, self.activity) {
            (NormLifecycle::Terminal, _) => AgentStatus::Done,
            (NormLifecycle::Starting, _) => AgentStatus::Unknown,
            (NormLifecycle::Running, NormActivity::Idle) => AgentStatus::Idle,
            (NormLifecycle::Running, NormActivity::Working) => AgentStatus::Working,
            (NormLifecycle::Running, NormActivity::Blocked) => AgentStatus::Blocked,
            (NormLifecycle::Running, NormActivity::Unknown) => AgentStatus::Unknown,
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match (self.lifecycle, self.activity) {
            (NormLifecycle::Terminal, _) => "terminal",
            (NormLifecycle::Starting, _) => "starting",
            (NormLifecycle::Running, NormActivity::Idle) => "idle",
            (NormLifecycle::Running, NormActivity::Working) => "working",
            (NormLifecycle::Running, NormActivity::Blocked) => "blocked",
            (NormLifecycle::Running, NormActivity::Unknown) => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NormAgentIdentity {
    pub(crate) daemon_epoch: String,
    pub(crate) id: u64,
    pub(crate) generation: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum NormLifecycle {
    Starting,
    Running,
    Terminal,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum NormActivity {
    Idle,
    Working,
    Blocked,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NormInstance {
    instance_id: String,
    _revision: u64,
    active_tab_id: Option<u64>,
    herdr_pane_id: Option<String>,
    tabs: Vec<NormTab>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NormTab {
    tab_id: u64,
    _ordinal: u16,
    agent_id: Option<u64>,
    generation: u64,
    workspace: PathBuf,
    _label: String,
    _connection: NormConnection,
    _activity: NormActivity,
    _writable: bool,
    _session_title: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
enum NormConnection {
    Connecting,
    Ready,
    Failed,
    Disconnected,
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
    agents: Vec<AgentDto>,
    instances: Vec<InstanceDto>,
}

#[derive(Deserialize)]
struct AgentDto {
    id: u64,
    generation: u64,
    sequence: u64,
    workspace: PathBuf,
    lifecycle: NormLifecycle,
    activity: NormActivity,
    session_id: Option<String>,
    title: Option<String>,
    open_views: u32,
}

#[derive(Deserialize)]
struct InstanceDto {
    instance_id: String,
    revision: u64,
    active_tab_id: Option<u64>,
    herdr_pane_id: Option<String>,
    tabs: Vec<TabDto>,
}

#[derive(Deserialize)]
struct TabDto {
    tab_id: u64,
    ordinal: u16,
    agent_id: Option<u64>,
    generation: u64,
    workspace: PathBuf,
    label: String,
    connection: NormConnection,
    activity: NormActivity,
    writable: bool,
    session_title: Option<String>,
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
        let outcome = watch_presence_connection(
            &path,
            generation,
            &completion,
            &shutdown,
            &mut received_snapshot,
            &mut stream_confirmed,
        );
        let failure = match outcome {
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
    let peer_euid = stream
        .peer_creds()
        .map_err(|_| FetchError::Transient)?
        .euid();
    if peer_euid != Some(current_euid()) {
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
            *stream_confirmed |= snapshot._watch;
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
    let mut slot = completion
        .lock()
        .expect("Norm presence completion lock poisoned");
    *slot = Some(next);
}

#[cfg(test)]
fn fetch_presence(path: &Path) -> FetchOutcome {
    let completion = Arc::new(Mutex::new(None));
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let worker_completion = completion.clone();
    let path = path.to_owned();
    let worker = thread::spawn(move || stream_presence(path, 1, worker_completion, shutdown_rx));
    let deadline = Instant::now() + Duration::from_secs(2);
    let outcome = loop {
        if let Some(completion) = completion
            .lock()
            .expect("Norm presence completion lock poisoned")
            .take()
        {
            break completion.outcome;
        }
        if Instant::now() >= deadline {
            break FetchOutcome::Transient;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let _ = shutdown_tx.send(());
    let _ = worker.join();
    outcome
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
    let daemon_epoch = presence.daemon_epoch;
    let instances = presence
        .instances
        .into_iter()
        .map(|instance| NormInstance {
            instance_id: instance.instance_id,
            _revision: instance.revision,
            active_tab_id: instance.active_tab_id,
            herdr_pane_id: instance.herdr_pane_id,
            tabs: instance
                .tabs
                .into_iter()
                .map(|tab| NormTab {
                    tab_id: tab.tab_id,
                    _ordinal: tab.ordinal,
                    agent_id: tab.agent_id,
                    generation: tab.generation,
                    workspace: tab.workspace,
                    _label: tab.label,
                    _connection: tab.connection,
                    _activity: tab.activity,
                    _writable: tab.writable,
                    _session_title: tab.session_title,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    Ok(PresenceSnapshot {
        daemon_epoch: daemon_epoch.clone(),
        _revision: presence.revision,
        _watch: presence.watch,
        agents: presence
            .agents
            .into_iter()
            .filter(|agent| {
                agent.open_views > 0
                    || matches!(
                        agent.activity,
                        NormActivity::Working | NormActivity::Blocked
                    )
            })
            .map(|agent| NormAgent {
                identity: NormAgentIdentity {
                    daemon_epoch: daemon_epoch.clone(),
                    id: agent.id,
                    generation: agent.generation,
                },
                workspace: agent.workspace,
                view: agent_view(&instances, agent.id, agent.generation),
                lifecycle: agent.lifecycle,
                activity: agent.activity,
                session_id: agent.session_id,
                title: agent.title,
                open_views: agent.open_views,
                _sequence: agent.sequence,
            })
            .collect(),
        instances,
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
            let pane_id = instance
                .herdr_pane_id
                .as_ref()
                .filter(|pane| !pane.is_empty())?;
            let tab = instance.tabs.iter().find(|tab| tab.tab_id == active_tab)?;
            Some(NormWorkspaceChange {
                instance_id: instance.instance_id.clone(),
                pane_id: pane_id.clone(),
                workspace: tab.workspace.clone(),
            })
        })
        .collect()
}

fn agent_view(instances: &[NormInstance], agent_id: u64, generation: u64) -> NormAgentView {
    let mut active = Vec::new();
    let mut active_without_pane = false;
    let mut inactive_with_pane = false;
    let mut matched = false;
    for instance in instances {
        for tab in &instance.tabs {
            if tab.agent_id != Some(agent_id) || tab.generation != generation {
                continue;
            }
            matched = true;
            let pane = instance
                .herdr_pane_id
                .as_deref()
                .filter(|pane| !pane.is_empty());
            if instance.active_tab_id == Some(tab.tab_id) {
                if let Some(pane) = pane {
                    active.push((!tab._writable, instance.instance_id.as_str(), pane));
                } else {
                    active_without_pane = true;
                }
            } else if pane.is_some() {
                inactive_with_pane = true;
            }
        }
    }
    if let Some((_, _, pane)) = active.into_iter().min() {
        NormAgentView::ActiveHerdrPane(pane.to_owned())
    } else if active_without_pane || matched && !inactive_with_pane {
        NormAgentView::MissingHerdrPane
    } else if inactive_with_pane {
        NormAgentView::InactiveHerdrPane
    } else {
        NormAgentView::NoView
    }
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
    use std::{
        io::{BufRead, BufReader},
        os::unix::net::UnixListener,
        sync::mpsc,
    };

    use super::*;

    const PRESENCE: &str = r#"{"Presence":{"version":2,"daemon_epoch":"epoch-a","revision":7,"agents":[{"id":42,"generation":3,"sequence":9,"workspace":"/work/repo","harness":"opencode","lifecycle":"Running","activity":"Blocked","session_id":"session-a","title":"Fix parser","open_views":2,"future_agent_field":true}],"instances":[{"instance_id":"terminal-a","revision":4,"active_tab_id":8,"herdr_pane_id":"w9:p4","tabs":[{"tab_id":8,"ordinal":0,"agent_id":42,"generation":3,"workspace":"/work/repo","harness":"opencode","label":"parser","connection":"Ready","activity":"Working","writable":false,"session_title":"Fix parser","future_tab_field":17}],"future_instance_field":{}}],"future_presence_field":"ignored"}}"#;

    fn two_tab_presence(epoch: &str, active_tab_id: u64) -> String {
        serde_json::json!({
            "Presence": {
                "version": 2,
                "watch": true,
                "daemon_epoch": epoch,
                "revision": 8,
                "agents": [],
                "instances": [{
                    "instance_id": "terminal-a",
                    "revision": 5,
                    "active_tab_id": active_tab_id,
                    "herdr_pane_id": "w9:p4",
                    "tabs": [{
                        "tab_id": 8,
                        "ordinal": 0,
                        "agent_id": 42,
                        "generation": 3,
                        "workspace": "/work/one",
                        "harness": "opencode",
                        "label": "one",
                        "connection": "Ready",
                        "activity": "Idle",
                        "writable": true,
                        "session_title": null
                    }, {
                        "tab_id": 9,
                        "ordinal": 1,
                        "agent_id": 43,
                        "generation": 1,
                        "workspace": "/work/two",
                        "harness": "opencode",
                        "label": "two",
                        "connection": "Ready",
                        "activity": "Idle",
                        "writable": true,
                        "session_title": null
                    }]
                }]
            }
        })
        .to_string()
    }

    #[test]
    fn active_tab_changes_emit_once_without_synthesizing_daemon_restarts() {
        let mut presence = NormPresence::with_socket_path(PathBuf::new(), Instant::now());
        presence.set_snapshot_for_test(&two_tab_presence("epoch-a", 8));
        assert!(presence.take_workspace_changes().is_empty());

        presence.set_snapshot_for_test(&two_tab_presence("epoch-a", 9));
        assert_eq!(
            presence.take_workspace_changes(),
            [NormWorkspaceChange {
                instance_id: "terminal-a".to_owned(),
                pane_id: "w9:p4".to_owned(),
                workspace: PathBuf::from("/work/two"),
            }]
        );

        presence.set_snapshot_for_test(&two_tab_presence("epoch-a", 9));
        assert!(presence.take_workspace_changes().is_empty());
        presence.set_snapshot_for_test(&two_tab_presence("epoch-b", 8));
        assert!(presence.take_workspace_changes().is_empty());
    }

    #[test]
    fn parses_identity_status_and_retains_topology() {
        let snapshot = parse_response(PRESENCE.as_bytes()).unwrap();
        assert!(!snapshot._watch);
        assert_eq!(snapshot._revision, 7);
        assert_eq!(snapshot.agents.len(), 1);
        let agent = &snapshot.agents[0];
        assert_eq!(agent.identity.daemon_epoch, "epoch-a");
        assert_eq!(agent.identity.id, 42);
        assert_eq!(agent.identity.generation, 3);
        assert_eq!(agent.status(), AgentStatus::Blocked);
        assert_eq!(agent.status_label(), "blocked");
        assert_eq!(
            agent.view,
            NormAgentView::ActiveHerdrPane("w9:p4".to_owned())
        );
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(snapshot.instances[0].tabs.len(), 1);
        assert_eq!(
            snapshot.instances[0].tabs[0]._connection,
            NormConnection::Ready
        );
    }

    #[test]
    fn distinguishes_inactive_and_legacy_norm_views() {
        let inactive = PRESENCE.replacen("\"active_tab_id\":8", "\"active_tab_id\":9", 1);
        assert_eq!(
            parse_response(inactive.as_bytes()).unwrap().agents[0].view,
            NormAgentView::InactiveHerdrPane
        );

        let legacy = PRESENCE.replacen("\"herdr_pane_id\":\"w9:p4\",", "", 1);
        assert_eq!(
            parse_response(legacy.as_bytes()).unwrap().agents[0].view,
            NormAgentView::MissingHerdrPane
        );
    }

    #[test]
    fn rejects_unsupported_protocol_versions() {
        let response = PRESENCE.replacen("\"version\":2", "\"version\":1", 1);
        assert_eq!(
            parse_response(response.as_bytes()).unwrap_err(),
            "unsupported Norm presence version 1"
        );
    }

    #[test]
    fn transient_failures_expire_but_absence_clears_immediately() {
        let now = Instant::now();
        let mut presence = NormPresence::with_socket_path(PathBuf::new(), now);
        let snapshot = parse_response(PRESENCE.as_bytes()).unwrap();
        assert!(presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                observed_at: now,
                outcome: FetchOutcome::Snapshot(snapshot.clone()),
            },
            now,
        ));
        assert!(presence.is_available());
        assert!(!presence.expire_stale_snapshot(now + STALE_GRACE * 2));

        assert!(!presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                observed_at: now,
                outcome: FetchOutcome::Transient,
            },
            now + STALE_GRACE - Duration::from_millis(1),
        ));
        assert!(presence.is_available());
        assert!(presence.expire_stale_snapshot(now + STALE_GRACE));
        assert!(!presence.is_available());

        presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                observed_at: now,
                outcome: FetchOutcome::Snapshot(snapshot),
            },
            now,
        );
        assert!(presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                observed_at: now,
                outcome: FetchOutcome::Absent,
            },
            now,
        ));
        assert!(!presence.is_available());
    }

    #[test]
    fn newer_transport_failure_replaces_an_unread_snapshot() {
        let now = Instant::now();
        let completion = Arc::new(Mutex::new(None));
        publish_completion(
            &completion,
            PresenceCompletion {
                generation: 1,
                observed_at: now,
                outcome: FetchOutcome::Snapshot(parse_response(PRESENCE.as_bytes()).unwrap()),
            },
        );
        publish_completion(
            &completion,
            PresenceCompletion {
                generation: 1,
                observed_at: now + Duration::from_millis(1),
                outcome: FetchOutcome::Transient,
            },
        );

        assert!(matches!(
            completion.lock().unwrap().as_ref().unwrap().outcome,
            FetchOutcome::Transient
        ));
    }

    #[test]
    fn stale_request_generations_cannot_replace_the_snapshot() {
        let now = Instant::now();
        let mut presence = NormPresence::with_socket_path(PathBuf::new(), now);
        presence.request_generation = 2;
        let snapshot = parse_response(PRESENCE.as_bytes()).unwrap();
        assert!(!presence.accept_completion(
            PresenceCompletion {
                generation: 1,
                observed_at: now,
                outcome: FetchOutcome::Snapshot(snapshot),
            },
            now,
        ));
        assert!(!presence.is_available());
    }

    #[test]
    fn successful_responses_authoritatively_replace_previous_agents() {
        let now = Instant::now();
        let mut presence = NormPresence::with_socket_path(PathBuf::new(), now);
        presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                observed_at: now,
                outcome: FetchOutcome::Snapshot(parse_response(PRESENCE.as_bytes()).unwrap()),
            },
            now,
        );
        assert_eq!(presence.agents().len(), 1);

        let empty = PRESENCE.replacen(
            "\"agents\":[{\"id\":42,\"generation\":3,\"sequence\":9,\"workspace\":\"/work/repo\",\"harness\":\"opencode\",\"lifecycle\":\"Running\",\"activity\":\"Blocked\",\"session_id\":\"session-a\",\"title\":\"Fix parser\",\"open_views\":2,\"future_agent_field\":true}]",
            "\"agents\":[]",
            1,
        );
        presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                observed_at: now + Duration::from_secs(1),
                outcome: FetchOutcome::Snapshot(parse_response(empty.as_bytes()).unwrap()),
            },
            now + Duration::from_secs(1),
        );
        assert!(presence.agents().is_empty());
        assert!(presence.is_available());
    }

    #[test]
    fn lifecycle_precedes_activity_in_card_status() {
        let mut snapshot = parse_response(PRESENCE.as_bytes()).unwrap();
        let agent = &mut snapshot.agents[0];
        agent.lifecycle = NormLifecycle::Starting;
        agent.activity = NormActivity::Working;
        assert_eq!(agent.status(), AgentStatus::Unknown);
        assert_eq!(agent.status_label(), "starting");

        agent.lifecycle = NormLifecycle::Terminal;
        assert_eq!(agent.status(), AgentStatus::Done);
        assert_eq!(agent.status_label(), "terminal");
    }

    #[test]
    fn detached_idle_agents_are_hidden_but_detached_work_remains_visible() {
        let idle = PRESENCE
            .replacen("\"activity\":\"Blocked\"", "\"activity\":\"Idle\"", 1)
            .replacen("\"open_views\":2", "\"open_views\":0", 1);
        assert!(parse_response(idle.as_bytes()).unwrap().agents.is_empty());

        let working = PRESENCE
            .replacen("\"activity\":\"Blocked\"", "\"activity\":\"Working\"", 1)
            .replacen("\"open_views\":2", "\"open_views\":0", 1);
        assert_eq!(parse_response(working.as_bytes()).unwrap().agents.len(), 1);
    }

    #[test]
    fn polling_sends_the_wire_request_and_keeps_one_request_in_flight() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("norm.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            request_tx.send(request).unwrap();
            reply_rx.recv().unwrap();
            stream.write_all(PRESENCE.as_bytes()).unwrap();
            stream.write_all(b"\n").unwrap();
        });

        let now = Instant::now();
        let mut presence = NormPresence::with_socket_path(socket, now);
        assert!(!presence.poll_at(now));
        let generation = presence.request_generation;
        assert!(!presence.poll_at(now + Duration::from_millis(500)));
        assert_eq!(presence.request_generation, generation);
        assert_eq!(
            request_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "{\"ListPresence\":{\"version\":2,\"watch\":true}}\n"
        );
        reply_tx.send(()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        while !presence.is_available() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
            presence.poll_at(now + Duration::from_millis(10));
        }
        server.join().unwrap();
        assert!(presence.is_available());
        assert_eq!(presence.agents()[0].identity.id, 42);
        presence.shutdown();
    }

    #[test]
    fn persistent_stream_delivers_tab_switches_with_low_latency() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("norm-stream.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let (snapshot_tx, snapshot_rx) = mpsc::channel::<String>();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            request_tx.send(request).unwrap();
            while let Ok(snapshot) = snapshot_rx.recv() {
                stream.write_all(snapshot.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
                stream.flush().unwrap();
            }
        });

        let mut presence = NormPresence::with_socket_path(socket, Instant::now());
        presence.poll();
        assert_eq!(
            request_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "{\"ListPresence\":{\"version\":2,\"watch\":true}}\n"
        );
        snapshot_tx
            .send(two_tab_presence("epoch-stream", 8))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !presence.is_available() && Instant::now() < deadline {
            thread::yield_now();
            presence.poll();
        }
        assert!(presence.is_available());
        assert!(presence.take_workspace_changes().is_empty());

        let mut latencies = Vec::new();
        for index in 0..40 {
            let active_tab_id = if index % 2 == 0 { 9 } else { 8 };
            let expected = if active_tab_id == 9 {
                PathBuf::from("/work/two")
            } else {
                PathBuf::from("/work/one")
            };
            let started = Instant::now();
            snapshot_tx
                .send(two_tab_presence("epoch-stream", active_tab_id))
                .unwrap();
            let deadline = started + Duration::from_millis(500);
            loop {
                presence.poll();
                if let Some(change) = presence.take_workspace_changes().pop() {
                    assert_eq!(change.workspace, expected);
                    latencies.push(started.elapsed());
                    break;
                }
                assert!(Instant::now() < deadline, "streamed tab switch timed out");
                thread::yield_now();
            }
        }
        latencies.sort_unstable();
        let p50 = latencies[latencies.len() / 2];
        let p95 = latencies[latencies.len() * 95 / 100];
        eprintln!(
            "Norm presence stream: p50={p50:?}, p95={p95:?}, max={:?}",
            latencies.last().unwrap()
        );
        assert!(p95 < Duration::from_millis(20));

        presence.shutdown();
        drop(snapshot_tx);
        server.join().unwrap();
    }

    #[test]
    #[ignore = "requires HUNKLE_NORM_PRESENCE_SOCKET pointing to a live Norm daemon with an open view"]
    fn reads_live_norm_presence_snapshot() {
        let socket = env::var_os("HUNKLE_NORM_PRESENCE_SOCKET")
            .map(PathBuf::from)
            .expect("HUNKLE_NORM_PRESENCE_SOCKET must be set");
        let FetchOutcome::Snapshot(snapshot) = fetch_presence(&socket) else {
            panic!("live Norm presence request did not return a snapshot");
        };
        assert!(!snapshot.agents.is_empty());
        assert!(snapshot.agents.iter().any(|agent| agent.open_views > 0));
    }
}
