use std::{
    env,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
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

const PRESENCE_VERSION: u32 = 1;
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const STALE_GRACE: Duration = Duration::from_secs(6);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct NormPresence {
    socket_path: PathBuf,
    snapshot: Option<PresenceSnapshot>,
    last_success: Option<Instant>,
    next_refresh: Instant,
    request_generation: u64,
    loading: bool,
    scroll: usize,
    workspace_changes: Vec<NormWorkspaceChange>,
    completions_tx: Sender<PresenceCompletion>,
    completions_rx: Receiver<PresenceCompletion>,
    #[cfg(test)]
    disabled: bool,
}

impl NormPresence {
    pub(crate) fn new() -> Self {
        Self::with_socket_path(daemon_socket_path(), Instant::now())
    }

    fn with_socket_path(socket_path: PathBuf, now: Instant) -> Self {
        let (completions_tx, completions_rx) = mpsc::channel();
        Self {
            socket_path,
            snapshot: None,
            last_success: None,
            next_refresh: now,
            request_generation: 0,
            loading: false,
            scroll: 0,
            workspace_changes: Vec::new(),
            completions_tx,
            completions_rx,
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

        let mut changed = false;
        while let Ok(completion) = self.completions_rx.try_recv() {
            changed |= self.accept_completion(completion, now);
        }
        changed |= self.expire_stale_snapshot(now);

        if !self.loading && now >= self.next_refresh {
            self.request_generation = self.request_generation.wrapping_add(1);
            self.loading = true;
            self.next_refresh = now + REFRESH_INTERVAL;
            let generation = self.request_generation;
            let socket_path = self.socket_path.clone();
            let completions = self.completions_tx.clone();
            thread::spawn(move || {
                let outcome = fetch_presence(&socket_path);
                let _ = completions.send(PresenceCompletion {
                    generation,
                    outcome,
                });
            });
        }
        changed
    }

    fn accept_completion(&mut self, completion: PresenceCompletion, now: Instant) -> bool {
        if completion.generation != self.request_generation {
            return false;
        }
        self.loading = false;
        match completion.outcome {
            FetchOutcome::Snapshot(snapshot) => {
                let changed = self.snapshot.as_ref() != Some(&snapshot);
                self.workspace_changes
                    .extend(active_workspace_changes(self.snapshot.as_ref(), &snapshot));
                self.scroll = self.scroll.min(snapshot.agents.len().saturating_sub(1));
                self.snapshot = Some(snapshot);
                self.last_success = Some(now);
                changed
            }
            FetchOutcome::Absent => {
                self.last_success = None;
                self.scroll = 0;
                self.snapshot.take().is_some()
            }
            FetchOutcome::Transient => self.expire_stale_snapshot(now),
        }
    }

    fn expire_stale_snapshot(&mut self, now: Instant) -> bool {
        let expired = self
            .last_success
            .is_some_and(|success| now.saturating_duration_since(success) >= STALE_GRACE);
        if !expired {
            return false;
        }
        self.last_success = None;
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

    #[cfg(test)]
    pub(crate) fn set_snapshot_for_test(&mut self, response: &str) {
        self.accept_completion(
            PresenceCompletion {
                generation: self.request_generation,
                outcome: FetchOutcome::Snapshot(
                    parse_response(response.as_bytes()).expect("valid Norm test presence"),
                ),
            },
            Instant::now(),
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PresenceSnapshot {
    daemon_epoch: String,
    _revision: u64,
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
    outcome: FetchOutcome,
}

enum FetchOutcome {
    Snapshot(PresenceSnapshot),
    Absent,
    Transient,
}

#[derive(Serialize)]
enum PresenceRequest {
    ListPresence { version: u32 },
}

#[derive(Deserialize)]
enum PresenceResponse {
    Presence(PresenceDto),
}

#[derive(Deserialize)]
struct PresenceDto {
    version: u32,
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

fn fetch_presence(path: &Path) -> FetchOutcome {
    match try_fetch_presence(path) {
        Ok(snapshot) => FetchOutcome::Snapshot(snapshot),
        Err(FetchError::Absent) => FetchOutcome::Absent,
        Err(FetchError::Transient) => FetchOutcome::Transient,
    }
}

fn try_fetch_presence(path: &Path) -> Result<PresenceSnapshot, FetchError> {
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
        .set_send_timeout(Some(IO_TIMEOUT))
        .map_err(|_| FetchError::Transient)?;
    stream
        .set_recv_timeout(Some(IO_TIMEOUT))
        .map_err(|_| FetchError::Transient)?;

    serde_json::to_writer(
        &mut stream,
        &PresenceRequest::ListPresence {
            version: PRESENCE_VERSION,
        },
    )
    .map_err(|_| FetchError::Transient)?;
    stream
        .write_all(b"\n")
        .and_then(|()| stream.flush())
        .map_err(|_| FetchError::Transient)?;

    let mut response = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|_| FetchError::Transient)?;
        if read == 0 {
            return Err(FetchError::Transient);
        }
        let chunk = &buffer[..read];
        let line_end = chunk.iter().position(|byte| *byte == b'\n');
        let body = line_end.map_or(chunk, |index| &chunk[..index]);
        if response.len().saturating_add(body.len()) > MAX_RESPONSE_BYTES {
            return Err(FetchError::Transient);
        }
        response.extend_from_slice(body);
        if line_end.is_some() {
            break;
        }
    }
    parse_response(&response).map_err(|_| FetchError::Transient)
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

    const PRESENCE: &str = r#"{"Presence":{"version":1,"daemon_epoch":"epoch-a","revision":7,"agents":[{"id":42,"generation":3,"sequence":9,"workspace":"/work/repo","lifecycle":"Running","activity":"Blocked","session_id":"session-a","title":"Fix parser","open_views":2,"future_agent_field":true}],"instances":[{"instance_id":"terminal-a","revision":4,"active_tab_id":8,"herdr_pane_id":"w9:p4","tabs":[{"tab_id":8,"ordinal":0,"agent_id":42,"generation":3,"workspace":"/work/repo","label":"parser","connection":"Ready","activity":"Working","writable":false,"session_title":"Fix parser","future_tab_field":17}],"future_instance_field":{}}],"future_presence_field":"ignored"}}"#;

    fn two_tab_presence(epoch: &str, active_tab_id: u64) -> String {
        serde_json::json!({
            "Presence": {
                "version": 1,
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
        let response = PRESENCE.replacen("\"version\":1", "\"version\":2", 1);
        assert_eq!(
            parse_response(response.as_bytes()).unwrap_err(),
            "unsupported Norm presence version 2"
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
                outcome: FetchOutcome::Snapshot(snapshot.clone()),
            },
            now,
        ));
        assert!(presence.is_available());

        assert!(!presence.accept_completion(
            PresenceCompletion {
                generation: 0,
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
                outcome: FetchOutcome::Snapshot(snapshot),
            },
            now,
        );
        assert!(presence.accept_completion(
            PresenceCompletion {
                generation: 0,
                outcome: FetchOutcome::Absent,
            },
            now,
        ));
        assert!(!presence.is_available());
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
                outcome: FetchOutcome::Snapshot(parse_response(PRESENCE.as_bytes()).unwrap()),
            },
            now,
        );
        assert_eq!(presence.agents().len(), 1);

        let empty = PRESENCE.replacen(
            "\"agents\":[{\"id\":42,\"generation\":3,\"sequence\":9,\"workspace\":\"/work/repo\",\"lifecycle\":\"Running\",\"activity\":\"Blocked\",\"session_id\":\"session-a\",\"title\":\"Fix parser\",\"open_views\":2,\"future_agent_field\":true}]",
            "\"agents\":[]",
            1,
        );
        presence.accept_completion(
            PresenceCompletion {
                generation: 0,
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
        assert!(presence.loading);
        let generation = presence.request_generation;
        assert!(!presence.poll_at(now + REFRESH_INTERVAL));
        assert_eq!(presence.request_generation, generation);
        assert_eq!(
            request_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "{\"ListPresence\":{\"version\":1}}\n"
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
