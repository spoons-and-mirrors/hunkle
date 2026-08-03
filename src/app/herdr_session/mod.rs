use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

use super::{
    AgentPaneDirection, LinkedWorktreeCandidate, LinkedWorktreeObservation,
    settings::AgentTimeDisplay,
};
use crate::filesystem::atomic_write;
use crate::git;

mod client;
mod latest_message;
mod stash;
mod timings;

pub(crate) use stash::StashedAgent;

pub(crate) use client::HerdrPaneLayout;
#[cfg(test)]
pub(crate) use client::HerdrPaneRect;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const AGENT_CHANGE_STATS_INTERVAL: Duration = Duration::from_secs(5);
const AGENT_MESSAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const TIMING_LAST_SEEN_INTERVAL_MS: u64 = 60_000;
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);
const LAYOUT_INDEX_VERSION: u8 = 1;
pub(crate) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) fn send_command_below(command: String) -> Result<String, String> {
    client::send_command_below(command)
}

pub(super) fn replace_pane_with_agent(
    path: PathBuf,
    workspace_id: String,
    pane_id: String,
    session_id: Option<String>,
) -> Result<String, String> {
    client::replace_pane_with_agent(path, workspace_id, pane_id, session_id)
}

pub(super) fn split_pane_with_agent(
    path: PathBuf,
    pane_id: String,
    direction: AgentPaneDirection,
    session_id: Option<String>,
) -> Result<String, String> {
    client::split_pane_with_agent(path, pane_id, direction, session_id)
}

pub(super) fn pane_layout(pane_id: String) -> Result<HerdrPaneLayout, String> {
    client::pane_layout(pane_id)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    fn should_track_timing(self) -> bool {
        matches!(self, Self::Working | Self::Blocked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrWorkspace {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) branch: Option<String>,
    pub(crate) parent_workspace_id: Option<String>,
    repo_root: Option<PathBuf>,
    linked_worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
struct AgentSessionIdentity {
    source: String,
    agent: String,
    kind: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(tag = "scope", content = "identity", rename_all = "snake_case")]
enum AgentTimingKey {
    Session(AgentSessionIdentity),
    Terminal(String),
    Pane(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentKey(AgentTimingKey);

impl AgentTimingKey {
    fn stable_id(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentPane {
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) destination_cwd: Option<PathBuf>,
    pub(crate) focused: bool,
    pub(crate) runtime: AgentRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRuntime {
    pub(crate) name: String,
    pub(crate) session_name: Option<String>,
    pub(crate) status: AgentStatus,
    timing_key: AgentTimingKey,
    session_timing_key: Option<AgentTimingKey>,
    state_change_seq: u64,
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
struct AgentTiming {
    elapsed_ms: u64,
    #[serde(default)]
    session_elapsed_ms: u64,
    running_since_ms: Option<u64>,
    status: AgentStatus,
    state_change_seq: u64,
    #[serde(default)]
    last_seen_ms: u64,
    #[serde(default)]
    awaiting_sequence: bool,
}

#[derive(Default, Deserialize, Serialize)]
struct AgentLayoutIndex {
    version: u8,
    layouts: Vec<AgentLayoutRecord>,
}

#[derive(Deserialize, Serialize)]
struct AgentLayoutRecord {
    key: AgentTimingKey,
    layout: client::AgentLayout,
}

impl AgentTiming {
    fn new(status: AgentStatus, state_change_seq: u64, now_ms: u64) -> Self {
        Self {
            elapsed_ms: 0,
            session_elapsed_ms: 0,
            running_since_ms: (status == AgentStatus::Working).then_some(now_ms),
            status,
            state_change_seq,
            last_seen_ms: now_ms,
            awaiting_sequence: false,
        }
    }

    fn observe(&mut self, status: AgentStatus, state_change_seq: u64, now_ms: u64) {
        let sequence_changed = self.state_change_seq != 0
            && state_change_seq != 0
            && self.state_change_seq != state_change_seq
            && !self.awaiting_sequence;
        let reconciles_event = self.awaiting_sequence
            && state_change_seq != 0
            && self.state_change_seq != state_change_seq;
        let transition = self.status != status || sequence_changed;
        match status {
            AgentStatus::Working => match self.status {
                AgentStatus::Working if sequence_changed => self.start_loop(now_ms),
                AgentStatus::Working => {}
                AgentStatus::Blocked | AgentStatus::Unknown => self.running_since_ms = Some(now_ms),
                AgentStatus::Idle | AgentStatus::Done => self.start_loop(now_ms),
            },
            AgentStatus::Blocked | AgentStatus::Unknown => {
                if !matches!(
                    self.status,
                    AgentStatus::Working | AgentStatus::Blocked | AgentStatus::Unknown
                ) {
                    self.elapsed_ms = 0;
                }
                self.pause(now_ms);
            }
            AgentStatus::Idle | AgentStatus::Done => self.pause(now_ms),
        }
        self.status = status;
        self.state_change_seq = state_change_seq;
        if reconciles_event {
            self.awaiting_sequence = false;
        }
        if transition || now_ms.saturating_sub(self.last_seen_ms) >= TIMING_LAST_SEEN_INTERVAL_MS {
            self.last_seen_ms = now_ms;
        }
    }

    fn observe_event(&mut self, status: AgentStatus, now_ms: u64) {
        self.observe(status, self.state_change_seq, now_ms);
        self.awaiting_sequence = true;
    }

    fn start_loop(&mut self, now_ms: u64) {
        self.pause(now_ms);
        self.session_elapsed_ms = self.session_elapsed_ms.saturating_add(self.elapsed_ms);
        self.elapsed_ms = 0;
        self.running_since_ms = Some(now_ms);
    }

    fn pause(&mut self, now_ms: u64) {
        if let Some(started_ms) = self.running_since_ms.take() {
            self.elapsed_ms = self
                .elapsed_ms
                .saturating_add(now_ms.saturating_sub(started_ms));
        }
    }

    fn elapsed_at(&self, display: AgentTimeDisplay, now_ms: u64) -> Duration {
        let running_ms = self
            .running_since_ms
            .map(|started_ms| now_ms.saturating_sub(started_ms))
            .unwrap_or_default();
        let latest_ms = self.elapsed_ms.saturating_add(running_ms);
        Duration::from_millis(match display {
            AgentTimeDisplay::LatestLoop => latest_ms,
            AgentTimeDisplay::AgentTotal => self.session_elapsed_ms.saturating_add(latest_ms),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AgentEntryState {
    pub(crate) selected: bool,
}

#[derive(Default)]
pub(crate) struct HerdrSessionPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
    pub(crate) reopen_path: Option<PathBuf>,
}

#[allow(clippy::large_enum_variant)]
enum Completion {
    Snapshot {
        result: Result<(Vec<HerdrWorkspace>, Vec<AgentPane>), String>,
        observed_at_ms: u64,
    },
    Event {
        event: client::Event,
        observed_at_ms: u64,
    },
    AgentDisplay {
        result: Result<Box<client::DisplayAgentResult>, String>,
        selected_key: AgentTimingKey,
        outgoing_key: Option<AgentTimingKey>,
        reopen_path: Option<PathBuf>,
        host_workspace_id: String,
        host_tab_id: String,
        selected_workspace_id: String,
        selected_tab_id: String,
    },
    AgentStash {
        session_id: String,
        name: String,
        result: Result<(), String>,
    },
    AgentStashIdentity {
        result: Result<AgentSessionIdentity, String>,
    },
    Fullscreen {
        result: Result<bool, String>,
    },
    AgentChangeStats(Vec<(PathBuf, Option<(u64, u64)>)>),
    LatestUserMessage {
        identity: AgentSessionIdentity,
        result: Result<Vec<AgentUserMessage>, String>,
    },
}

struct PendingAgentStash {
    agent: AgentPane,
    index: usize,
    repository: PathBuf,
    repository_label: String,
    worktree: PathBuf,
    branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentActivityPreview {
    Reasoning,
    Tool {
        name: String,
        title: Option<String>,
        running: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentUserMessage {
    pub(crate) text: String,
    pub(crate) requests: Vec<AgentRequestPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRequestPreview {
    pub(crate) parts: Vec<AgentRequestPartPreview>,
    pub(crate) reasoning_active: bool,
    pub(crate) tool_call_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentRequestPartPreview {
    Text(String),
    Activity(AgentActivityPreview),
}

pub(crate) struct HerdrSession {
    enabled: bool,
    pub(crate) workspaces: Vec<HerdrWorkspace>,
    pub(crate) agents: Vec<AgentPane>,
    pub(crate) agent_scroll: usize,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    host_workspace_id: Option<String>,
    host_tab_id: Option<String>,
    host_pane_id: Option<String>,
    cross_workspace_agents: bool,
    agent_display_running: bool,
    fullscreen_running: bool,
    fullscreen: bool,
    agent_change_stats: HashMap<PathBuf, (u64, u64)>,
    agent_change_stats_loading: bool,
    next_agent_change_stats: Instant,
    latest_user_messages: HashMap<AgentSessionIdentity, Vec<AgentUserMessage>>,
    latest_user_message_requests: HashSet<AgentSessionIdentity>,
    latest_user_message_refreshes: HashMap<AgentSessionIdentity, Instant>,
    latest_user_message_statuses: HashMap<AgentSessionIdentity, AgentStatus>,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    next_refresh: Instant,
    spinner_frame: usize,
    next_spinner: Instant,
    agent_timings: HashMap<AgentTimingKey, AgentTiming>,
    agent_timing_persistence: Option<timings::Persistence>,
    agent_timing_clear_generation: u64,
    agent_timing_persistence_notice: Option<String>,
    agent_layouts: HashMap<AgentTimingKey, client::AgentLayout>,
    agent_layouts_path: Option<PathBuf>,
    displayed_agent_key: Option<AgentTimingKey>,
    stash: stash::AgentStashStore,
    pub(crate) showing_stash: bool,
    pub(crate) stash_scroll: usize,
    agent_stash_running: bool,
    pending_agent_stash: Option<PendingAgentStash>,
    stashed_pane_ids: HashSet<String>,
}

impl HerdrSession {
    pub(crate) fn detect(config_dir: Option<&Path>) -> Self {
        #[cfg(test)]
        let environment: Option<client::Environment> = None;
        #[cfg(not(test))]
        let environment = client::environment();
        let enabled = environment.is_some();
        let mut session = Self::new(
            enabled,
            config_dir.map(|path| path.join("agent-timings.json")),
            config_dir.map(|path| path.join("agent-stash.json")),
        );
        if let Some(environment) = environment {
            session.host_workspace_id = environment.workspace_id;
            session.host_tab_id = environment.tab_id;
            session.host_pane_id = environment.pane_id;
            if let (Some(config_dir), Some(host_pane_id)) =
                (config_dir, session.host_pane_id.as_deref())
            {
                let path = agent_layouts_path(config_dir, host_pane_id);
                session.agent_layouts = load_agent_layouts(&path).unwrap_or_default();
                session.agent_layouts_path = Some(path);
            }
            session.start_event_listener();
        }
        session
    }

    fn new(
        enabled: bool,
        agent_timings_path: Option<PathBuf>,
        agent_stash_path: Option<PathBuf>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            enabled,
            workspaces: Vec::new(),
            agents: Vec::new(),
            agent_scroll: 0,
            loading: false,
            error: None,
            host_workspace_id: None,
            host_tab_id: None,
            host_pane_id: None,
            cross_workspace_agents: false,
            agent_display_running: false,
            fullscreen_running: false,
            fullscreen: false,
            agent_change_stats: HashMap::new(),
            agent_change_stats_loading: false,
            next_agent_change_stats: Instant::now(),
            latest_user_messages: HashMap::new(),
            latest_user_message_requests: HashSet::new(),
            latest_user_message_refreshes: HashMap::new(),
            latest_user_message_statuses: HashMap::new(),
            sender,
            receiver,
            next_refresh: Instant::now(),
            spinner_frame: 0,
            next_spinner: Instant::now(),
            agent_timings: HashMap::new(),
            agent_timing_persistence: agent_timings_path
                .filter(|_| enabled)
                .map(timings::Persistence::new),
            agent_timing_clear_generation: 0,
            agent_timing_persistence_notice: None,
            agent_layouts: HashMap::new(),
            agent_layouts_path: None,
            displayed_agent_key: None,
            stash: stash::AgentStashStore::new(agent_stash_path),
            showing_stash: false,
            stash_scroll: 0,
            agent_stash_running: false,
            pending_agent_stash: None,
            stashed_pane_ids: HashSet::new(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(persistence) = self.agent_timing_persistence.as_mut() {
            persistence.shutdown();
        }
    }

    pub(crate) fn linked_worktree_observation(&self) -> LinkedWorktreeObservation {
        let candidates = self
            .workspaces
            .iter()
            .filter_map(|workspace| {
                workspace
                    .path
                    .clone()
                    .map(|path| LinkedWorktreeCandidate { path })
            })
            .collect();
        LinkedWorktreeObservation { candidates }
    }

    pub(crate) fn poll(&mut self) -> HerdrSessionPoll {
        if !self.enabled {
            return HerdrSessionPoll::default();
        }
        let mut poll = HerdrSessionPoll::default();
        if let Some(notice) = self.agent_timing_persistence_notice.take() {
            poll.notice = Some(notice);
        }
        if let Some(persistence) = self.agent_timing_persistence.as_mut() {
            while let Some(result) =
                persistence.poll(&mut self.agent_timings, self.agent_timing_clear_generation)
            {
                match result {
                    Ok(changed) => poll.changed |= changed,
                    Err(error) => {
                        poll.notice =
                            Some(format!("Could not persist agent timing history: {error}"));
                    }
                }
            }
        }
        while let Ok(completion) = self.receiver.try_recv() {
            poll.changed = true;
            match completion {
                Completion::Snapshot {
                    result,
                    observed_at_ms,
                } => {
                    self.loading = false;
                    match result {
                        Ok((workspaces, agents)) => {
                            self.workspaces = workspaces;
                            self.apply_agent_snapshot_at(agents, observed_at_ms);
                            self.error = None;
                        }
                        Err(error) => {
                            self.error = Some(error);
                        }
                    }
                }
                Completion::Event {
                    event,
                    observed_at_ms,
                } => {
                    let client::Event::AgentStatus(event) = event;
                    self.apply_agent_status_event_at(event, observed_at_ms);
                    self.next_refresh = Instant::now();
                }
                Completion::AgentDisplay {
                    result,
                    selected_key,
                    outgoing_key,
                    reopen_path,
                    host_workspace_id,
                    host_tab_id,
                    selected_workspace_id,
                    selected_tab_id,
                } => {
                    self.agent_display_running = false;
                    self.next_refresh = Instant::now();
                    match result {
                        Ok(result) => {
                            let client::DisplayAgentResult {
                                displayed,
                                parked,
                                pane_locations,
                            } = *result;
                            for saved in self.agent_layouts.values_mut() {
                                saved.remap_known(&pane_locations);
                            }
                            for agent in &mut self.agents {
                                let Some(location) = pane_locations.get(&agent.pane_id) else {
                                    continue;
                                };
                                agent.pane_id = location.pane_id.clone();
                                agent.tab_id = location.tab_id.clone();
                                if location.tab_id == host_tab_id {
                                    agent.workspace_id = host_workspace_id.clone();
                                } else if location.tab_id == selected_tab_id {
                                    agent.workspace_id = selected_workspace_id.clone();
                                }
                            }
                            if let (Some(key), Some(layout)) = (outgoing_key, parked) {
                                self.agent_layouts.insert(key, layout);
                            }
                            self.agent_layouts.insert(selected_key.clone(), displayed);
                            self.displayed_agent_key = Some(selected_key);
                            if let Some(path) = self.agent_layouts_path.as_deref()
                                && let Err(error) = save_agent_layouts(path, &self.agent_layouts)
                            {
                                poll.notice = Some(format!(
                                    "Agent displayed, but its layout could not be saved: {error}"
                                ));
                            }
                            poll.reopen_path = reopen_path;
                        }
                        Err(error) => poll.notice = Some(error),
                    }
                }
                Completion::AgentStash {
                    session_id,
                    name,
                    result,
                } => {
                    self.agent_stash_running = false;
                    self.next_refresh = Instant::now();
                    poll.notice = Some(match result {
                        Ok(()) => {
                            if let Some(pending) = self.pending_agent_stash.take() {
                                self.stashed_pane_ids.insert(pending.agent.pane_id);
                            }
                            format!("Stashed agent {name}")
                        }
                        Err(error) => {
                            let rollback = self.stash.remove(&session_id).err();
                            self.restore_pending_agent_stash();
                            rollback.map_or_else(
                                || format!("Could not stash agent: {error}"),
                                |rollback| format!("Could not stash agent: {error}; {rollback}"),
                            )
                        }
                    });
                }
                Completion::AgentStashIdentity { result } => {
                    match result.and_then(|identity| self.close_pending_agent_stash(identity)) {
                        Ok(()) => {
                            poll.notice = Some("Closing and stashing agent".to_owned());
                        }
                        Err(error) => {
                            self.agent_stash_running = false;
                            self.restore_pending_agent_stash();
                            poll.notice = Some(format!("Could not stash agent: {error}"));
                        }
                    }
                }
                Completion::Fullscreen { result } => {
                    self.fullscreen_running = false;
                    poll.notice = Some(match result {
                        Ok(fullscreen) => {
                            self.fullscreen = fullscreen;
                            if fullscreen {
                                "Hunkle is fullscreen".to_owned()
                            } else {
                                "Restored Herdr tab layout".to_owned()
                            }
                        }
                        Err(error) => format!("Could not toggle fullscreen: {error}"),
                    });
                }
                Completion::AgentChangeStats(stats) => {
                    self.agent_change_stats_loading = false;
                    self.next_agent_change_stats = Instant::now() + AGENT_CHANGE_STATS_INTERVAL;
                    self.agent_change_stats = stats
                        .into_iter()
                        .filter_map(|(path, stats)| stats.map(|stats| (path, stats)))
                        .collect();
                }
                Completion::LatestUserMessage { identity, result } => {
                    self.latest_user_message_requests.remove(&identity);
                    if let Ok(message) = result {
                        self.latest_user_messages.insert(identity, message);
                    }
                }
            }
        }
        if !self.loading && Instant::now() >= self.next_refresh {
            self.start_snapshot();
            poll.changed = true;
        }
        self.start_agent_change_stats_if_due(Instant::now());
        poll.changed |= self.poll_spinner(Instant::now());
        poll
    }

    fn apply_agent_snapshot_at(&mut self, agents: Vec<AgentPane>, now_ms: u64) {
        let observed_panes = agents
            .iter()
            .map(|agent| agent.pane_id.as_str())
            .collect::<HashSet<_>>();
        self.stashed_pane_ids
            .retain(|pane_id| observed_panes.contains(pane_id.as_str()));
        let pending_pane = self
            .pending_agent_stash
            .as_ref()
            .map(|pending| pending.agent.pane_id.as_str());
        let agents = agents
            .into_iter()
            .filter(|agent| {
                pending_pane != Some(agent.pane_id.as_str())
                    && !self.stashed_pane_ids.contains(&agent.pane_id)
            })
            .collect::<Vec<_>>();
        let agents = if self.cross_workspace_agents || self.host_workspace_id.is_none() {
            agents
        } else {
            agents
                .into_iter()
                .filter(|agent| self.host_workspace_id.as_deref() == Some(&agent.workspace_id))
                .collect()
        };
        timings::update_snapshot(&mut self.agent_timings, &agents, now_ms);
        if let Some(persistence) = self.agent_timing_persistence.as_ref()
            && let Err(error) = persistence.sync(
                &self.agent_timings,
                &agents,
                now_ms,
                self.agent_timing_clear_generation,
            )
        {
            self.agent_timing_persistence_notice =
                Some(format!("Could not persist agent timing history: {error}"));
        }
        let previous = &self.agents;
        let mut ranked = agents.into_iter().enumerate().collect::<Vec<_>>();
        ranked.sort_by_key(|(incoming_index, agent)| {
            (
                Reverse(agent.runtime.state_change_seq),
                previous
                    .iter()
                    .position(|existing| existing.pane_id == agent.pane_id)
                    .unwrap_or(usize::MAX),
                *incoming_index,
            )
        });
        self.agents = ranked.into_iter().map(|(_, agent)| agent).collect();
        if !self.agent_stash_running {
            let live_sessions = self
                .agents
                .iter()
                .filter_map(|agent| match agent.runtime.session_timing_key.as_ref() {
                    Some(AgentTimingKey::Session(identity)) => Some(identity.value.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let _ = self
                .stash
                .remove_live(live_sessions.iter().map(String::as_str));
        }
        let displayed_is_present = self.displayed_agent_key.as_ref().is_some_and(|key| {
            self.agents.iter().any(|agent| {
                &agent.runtime.timing_key == key
                    && self.host_workspace_id.as_deref() == Some(&agent.workspace_id)
                    && self.host_tab_id.as_deref() == Some(&agent.tab_id)
            })
        });
        if !displayed_is_present {
            self.displayed_agent_key = self
                .agents
                .iter()
                .find(|agent| {
                    self.host_workspace_id.as_deref() == Some(&agent.workspace_id)
                        && self.host_tab_id.as_deref() == Some(&agent.tab_id)
                })
                .map(|agent| agent.runtime.timing_key.clone());
        }
        self.agent_scroll = self.agent_scroll.min(self.agents.len().saturating_sub(1));
    }

    pub(crate) fn agent_elapsed(
        &self,
        index: usize,
        display: AgentTimeDisplay,
    ) -> Option<Duration> {
        let agent = self.agents.get(index)?;
        self.agent_timings
            .get(&agent.runtime.timing_key)
            .map(|timing| timing.elapsed_at(display, unix_time_ms()))
    }

    pub(crate) fn agent_display_name(&self, index: usize) -> Option<&str> {
        self.agents.get(index)?.runtime.session_name.as_deref()
    }

    pub(crate) fn agent_key(&self, index: usize) -> Option<AgentKey> {
        self.agents
            .get(index)
            .map(|agent| AgentKey(agent.runtime.timing_key.clone()))
    }

    pub(crate) fn agent_index(&self, key: &AgentKey) -> Option<usize> {
        self.agents
            .iter()
            .position(|agent| agent.runtime.timing_key == key.0)
    }

    pub(crate) fn stashed_agents(&self) -> &[StashedAgent] {
        &self.stash.agents
    }

    pub(crate) fn toggle_stash(&mut self) {
        self.showing_stash = !self.showing_stash;
        self.stash_scroll = 0;
        self.agent_scroll = 0;
    }

    pub(crate) fn show_live_agents(&mut self) {
        self.showing_stash = false;
        self.stash_scroll = 0;
        self.agent_scroll = 0;
    }

    pub(crate) fn stash_agent(
        &mut self,
        index: usize,
        repository: PathBuf,
        repository_label: String,
        branch: String,
    ) -> Result<(), String> {
        self.stash_agent_with_resolver(
            index,
            repository,
            repository_label,
            branch,
            latest_message::resolve_session_id,
        )
    }

    fn stash_agent_with_resolver<F>(
        &mut self,
        index: usize,
        repository: PathBuf,
        repository_label: String,
        branch: String,
        resolver: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&Path, &str) -> Result<String, String> + Send + 'static,
    {
        if self.agent_stash_running || self.agent_layout_running() {
            return Err("Another agent operation is still in progress".to_owned());
        }
        let agent = self
            .agents
            .get(index)
            .cloned()
            .ok_or_else(|| "Agent is no longer available".to_owned())?;
        let worktree = agent
            .destination_cwd
            .clone()
            .or_else(|| agent.cwd.clone())
            .ok_or_else(|| "Agent has not reported its working directory".to_owned())?;
        let identity = match agent.runtime.session_timing_key.as_ref() {
            Some(AgentTimingKey::Session(identity)) => Some(identity.clone()),
            _ if agent.runtime.name == "opencode" => {
                agent.runtime.session_name.as_deref().ok_or_else(|| {
                    "OpenCode session could not be identified without its title".to_owned()
                })?;
                None
            }
            _ => return Err("Agent has not reported a resumable session".to_owned()),
        };
        if identity
            .as_ref()
            .is_some_and(|identity| identity.agent != "opencode")
        {
            return Err(format!(
                "{} sessions cannot be restored yet",
                agent.runtime.name
            ));
        }
        let title = agent.runtime.session_name.clone();
        self.pending_agent_stash = Some(PendingAgentStash {
            agent,
            index,
            repository,
            repository_label,
            worktree: worktree.clone(),
            branch,
        });
        self.agents.remove(index);
        self.agent_scroll = self.agent_scroll.min(self.agents.len().saturating_sub(1));
        self.agent_stash_running = true;

        if let Some(identity) = identity {
            if let Err(error) = self.close_pending_agent_stash(identity) {
                self.agent_stash_running = false;
                self.restore_pending_agent_stash();
                return Err(error);
            }
            return Ok(());
        }

        let title = title.expect("fallback session title was validated");
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = resolver(&worktree, &title).map(|value| AgentSessionIdentity {
                source: "hunkle:opencode".to_owned(),
                agent: "opencode".to_owned(),
                kind: "id".to_owned(),
                value,
            });
            let _ = sender.send(Completion::AgentStashIdentity { result });
        });
        Ok(())
    }

    fn close_pending_agent_stash(&mut self, identity: AgentSessionIdentity) -> Result<(), String> {
        let pending = self
            .pending_agent_stash
            .as_ref()
            .ok_or_else(|| "Agent stash is no longer pending".to_owned())?;
        let record = StashedAgent {
            harness: identity.agent,
            agent_name: pending.agent.runtime.name.clone(),
            session_source: identity.source,
            session_kind: identity.kind,
            session_id: identity.value,
            session_name: pending.agent.runtime.session_name.clone(),
            repository: pending.repository.clone(),
            repository_label: pending.repository_label.clone(),
            worktree: pending.worktree.clone(),
            branch: pending.branch.clone(),
            workspace_id: pending.agent.workspace_id.clone(),
            tab_id: pending.agent.tab_id.clone(),
            pane_id: pending.agent.pane_id.clone(),
            cwd: pending.agent.cwd.clone(),
            destination_cwd: pending.agent.destination_cwd.clone(),
            focused: pending.agent.focused,
            status: pending.agent.runtime.status,
            stashed_at_ms: unix_time_ms(),
        };
        self.stash.add(record.clone())?;
        let pane_id = pending.agent.pane_id.clone();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = client::close_pane(pane_id);
            let _ = sender.send(Completion::AgentStash {
                session_id: record.session_id,
                name: record.session_name.unwrap_or(record.harness),
                result,
            });
        });
        Ok(())
    }

    fn restore_pending_agent_stash(&mut self) {
        let Some(pending) = self.pending_agent_stash.take() else {
            return;
        };
        let index = pending.index.min(self.agents.len());
        self.agents.insert(index, pending.agent);
    }

    pub(crate) fn agent_user_messages(&self, index: usize) -> Option<&[AgentUserMessage]> {
        let AgentTimingKey::Session(identity) = self
            .agents
            .get(index)?
            .runtime
            .session_timing_key
            .as_ref()?
        else {
            return None;
        };
        self.latest_user_messages.get(identity).map(Vec::as_slice)
    }

    pub(crate) fn request_agent_latest_user_message(&mut self, index: usize) {
        let Some((AgentTimingKey::Session(identity), status)) =
            self.agents.get(index).and_then(|agent| {
                agent
                    .runtime
                    .session_timing_key
                    .as_ref()
                    .map(|key| (key, agent.runtime.status))
            })
        else {
            return;
        };
        let identity = identity.clone();
        let now = Instant::now();
        let needs_refresh = !self.latest_user_messages.contains_key(&identity)
            || self.latest_user_message_statuses.get(&identity) != Some(&status)
            || status == AgentStatus::Working;
        if identity.agent != "opencode"
            || !needs_refresh
            || self.latest_user_message_requests.contains(&identity)
            || self
                .latest_user_message_refreshes
                .get(&identity)
                .is_some_and(|refresh| now < *refresh)
        {
            return;
        }

        self.latest_user_message_requests.insert(identity.clone());
        self.latest_user_message_refreshes
            .insert(identity.clone(), now + AGENT_MESSAGE_REFRESH_INTERVAL);
        self.latest_user_message_statuses
            .insert(identity.clone(), status);
        let session_id = identity.value.clone();
        let sender = self.sender.clone();
        let _ = thread::Builder::new()
            .name("agent-latest-message".to_owned())
            .spawn(move || {
                let result = latest_message::fetch(&session_id);
                let _ = sender.send(Completion::LatestUserMessage { identity, result });
            });
    }

    pub(crate) fn clear_agent_timing_history(&mut self) -> Result<(), String> {
        let now_ms = unix_time_ms();
        self.agent_timing_clear_generation = self.agent_timing_clear_generation.wrapping_add(1);
        timings::reset_local(&mut self.agent_timings, &self.agents, now_ms);
        if let Some(persistence) = self.agent_timing_persistence.as_ref() {
            persistence
                .reset(
                    &self.agent_timings,
                    &self.agents,
                    now_ms,
                    self.agent_timing_clear_generation,
                )
                .map_err(|error| format!("Could not clear agent timing history: {error}"))?;
        }
        Ok(())
    }

    pub(crate) fn agent_change_stats(&self, index: usize) -> Option<(u64, u64)> {
        self.agents
            .get(index)
            .and_then(|agent| agent.destination_cwd.as_ref())
            .and_then(|path| self.agent_change_stats.get(path))
            .copied()
    }

    pub(crate) fn agent_destination(&self, index: usize) -> Option<&Path> {
        let agent = self.agents.get(index)?;
        agent.destination_cwd.as_deref().or(agent.cwd.as_deref())
    }

    fn start_agent_change_stats_if_due(&mut self, now: Instant) {
        if self.agent_change_stats_loading || now < self.next_agent_change_stats {
            return;
        }
        let paths = self
            .agents
            .iter()
            .filter_map(|agent| agent.destination_cwd.clone())
            .collect::<HashSet<_>>();
        if paths.is_empty() {
            self.next_agent_change_stats = now + AGENT_CHANGE_STATS_INTERVAL;
            return;
        }
        self.agent_change_stats_loading = true;
        let sender = self.sender.clone();
        if thread::Builder::new()
            .name("agent-change-stats".to_owned())
            .spawn(move || {
                let stats = paths
                    .into_iter()
                    .map(|path| {
                        let stats = git::load_change_line_counts(&path).ok();
                        (path, stats)
                    })
                    .collect();
                let _ = sender.send(Completion::AgentChangeStats(stats));
            })
            .is_err()
        {
            self.agent_change_stats_loading = false;
            self.next_agent_change_stats = now + AGENT_CHANGE_STATS_INTERVAL;
        }
    }

    pub(crate) fn agent_entry_state(&self, index: usize) -> AgentEntryState {
        AgentEntryState {
            selected: self.agents.get(index).is_some_and(|agent| {
                self.displayed_agent_key
                    .as_ref()
                    .map_or(agent.focused, |key| key == &agent.runtime.timing_key)
            }),
        }
    }

    pub(crate) fn agent_is_in_host_tab(&self, index: usize) -> bool {
        self.agents.get(index).is_some_and(|agent| {
            self.host_workspace_id.as_deref() == Some(&agent.workspace_id)
                && self.host_tab_id.as_deref() == Some(&agent.tab_id)
        })
    }

    pub(crate) fn agent_layout_running(&self) -> bool {
        self.agent_display_running || self.fullscreen_running
    }

    pub(crate) fn fullscreen_running(&self) -> bool {
        self.fullscreen_running
    }

    pub(crate) fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub(crate) fn toggle_fullscreen(&mut self) -> Result<(), String> {
        if self.agent_layout_running() {
            return Err("Another Herdr layout change is still in progress".to_owned());
        }
        let pane_id = self
            .host_pane_id
            .clone()
            .ok_or_else(|| "Hunkle is not attached to a Herdr pane".to_owned())?;
        self.fullscreen_running = true;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = client::toggle_pane_zoom(pane_id);
            let _ = sender.send(Completion::Fullscreen { result });
        });
        Ok(())
    }

    pub(crate) fn show_agent(&mut self, index: usize) -> Result<(), String> {
        if self.agent_layout_running() {
            return Err("Another agent layout change is still in progress".to_owned());
        }
        if self.agent_is_in_host_tab(index) {
            return Ok(());
        }
        self.display_agent(index)
    }

    fn display_agent(&mut self, index: usize) -> Result<(), String> {
        let agent = self
            .agents
            .get(index)
            .ok_or_else(|| "Agent is no longer available".to_owned())?;
        let (Some(host_workspace_id), Some(host_tab_id), Some(host_pane_id)) = (
            self.host_workspace_id.clone(),
            self.host_tab_id.clone(),
            self.host_pane_id.clone(),
        ) else {
            return Err("Hunkle is not attached to a Herdr pane".to_owned());
        };
        let selected_workspace_id = agent.workspace_id.clone();
        let selected_tab_id = agent.tab_id.clone();
        let request = client::DisplayAgentRequest {
            pane_id: agent.pane_id.clone(),
            workspace_id: agent.workspace_id.clone(),
            tab_id: agent.tab_id.clone(),
            host_pane_id,
            host_workspace_id: host_workspace_id.clone(),
            host_tab_id: host_tab_id.clone(),
            allow_cross_workspace: self.cross_workspace_agents,
            saved_layout: self.agent_layouts.get(&agent.runtime.timing_key).cloned(),
        };
        let selected_key = agent.runtime.timing_key.clone();
        let outgoing_key = self.displayed_agent_key.clone().or_else(|| {
            self.agents
                .iter()
                .find(|agent| self.agent_is_in_host_tab_by_agent(agent))
                .map(|agent| agent.runtime.timing_key.clone())
        });
        let reopen_path = agent.cwd.clone();
        let completion_host_workspace_id = host_workspace_id.clone();
        let completion_host_tab_id = host_tab_id.clone();
        self.agent_display_running = true;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = client::display_agent(request).map(Box::new);
            let reopen_path = result.as_ref().ok().and(reopen_path);
            let _ = sender.send(Completion::AgentDisplay {
                result,
                selected_key,
                outgoing_key,
                reopen_path,
                host_workspace_id: completion_host_workspace_id,
                host_tab_id: completion_host_tab_id,
                selected_workspace_id,
                selected_tab_id,
            });
        });
        Ok(())
    }

    fn agent_is_in_host_tab_by_agent(&self, agent: &AgentPane) -> bool {
        self.host_workspace_id.as_deref() == Some(&agent.workspace_id)
            && self.host_tab_id.as_deref() == Some(&agent.tab_id)
    }

    pub(crate) fn agent_repository_name(&self, index: usize) -> Option<&str> {
        let agent = self.agents.get(index)?;
        if let Some(repository) = agent
            .destination_cwd
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
        {
            return Some(repository);
        }
        let workspace = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == agent.workspace_id)?;
        workspace
            .repo_root
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .or(Some(workspace.label.as_str()))
    }

    pub(crate) fn set_cross_workspace_agents(&mut self, enabled: bool) {
        self.cross_workspace_agents = enabled;
        if !enabled {
            self.agents.retain(|agent| {
                self.host_workspace_id.as_deref() == Some(agent.workspace_id.as_str())
            });
        }
        self.next_refresh = Instant::now();
    }

    pub(crate) fn scroll_agents(&mut self, delta: isize) {
        if self.showing_stash {
            self.stash_scroll = self.stash_scroll.saturating_add_signed(delta);
        } else {
            self.agent_scroll = self.agent_scroll.saturating_add_signed(delta);
        }
    }

    pub(crate) fn spinner_frame(&self) -> usize {
        self.spinner_frame
    }

    fn poll_spinner(&mut self, now: Instant) -> bool {
        let working = self.enabled
            && self
                .agents
                .iter()
                .any(|agent| agent.runtime.status == AgentStatus::Working);
        if !working {
            self.spinner_frame = 0;
            self.next_spinner = now;
            return false;
        }
        if now < self.next_spinner {
            return false;
        }
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
        self.next_spinner = now + SPINNER_INTERVAL;
        true
    }

    fn start_event_listener(&self) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            loop {
                if client::watch_events(|event| {
                    sender
                        .send(Completion::Event {
                            event,
                            observed_at_ms: unix_time_ms(),
                        })
                        .is_ok()
                })
                .is_ok()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(250));
            }
        });
    }

    fn apply_agent_status_event_at(&mut self, event: client::AgentStatusEvent, now_ms: u64) {
        let Some(agent) = self.agents.iter_mut().find(|agent| {
            agent.workspace_id == event.workspace_id && agent.pane_id == event.pane_id
        }) else {
            return;
        };
        agent.runtime.status = event.status;
        let key = agent.runtime.timing_key.clone();
        let state_change_seq = agent.runtime.state_change_seq;
        timings::observe_status_local(
            &mut self.agent_timings,
            &key,
            event.status,
            state_change_seq,
            now_ms,
        );
        if let Some(persistence) = self.agent_timing_persistence.as_ref()
            && let Err(error) = persistence.observe_status(
                &self.agent_timings,
                key,
                event.status,
                state_change_seq,
                now_ms,
                self.agent_timing_clear_generation,
            )
        {
            self.agent_timing_persistence_notice =
                Some(format!("Could not persist agent timing history: {error}"));
        }
    }

    fn start_snapshot(&mut self) {
        self.loading = true;
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = client::session_snapshot().map(|(mut workspaces, agents)| {
                populate_workspace_branches(&mut workspaces);
                (workspaces, agents)
            });
            let _ = sender.send(Completion::Snapshot {
                result,
                observed_at_ms: unix_time_ms(),
            });
        });
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test(value: &Value) -> Self {
        let mut session = Self::new(true, None, None);
        session.cross_workspace_agents = true;
        let (mut workspaces, agents) = client::parse_snapshot(value).unwrap();
        populate_workspace_branches(&mut workspaces);
        session.workspaces = workspaces;
        session.apply_agent_snapshot_at(agents, unix_time_ms());
        session
    }

    #[cfg(test)]
    pub(crate) fn apply_snapshot_for_test(&mut self, value: &Value) {
        let (mut workspaces, agents) = client::parse_snapshot(value).unwrap();
        populate_workspace_branches(&mut workspaces);
        self.workspaces = workspaces;
        self.apply_agent_snapshot_at(agents, unix_time_ms());
    }

    #[cfg(test)]
    pub(crate) fn set_host_for_test(&mut self, workspace: &str, tab: &str, pane: &str) {
        self.host_workspace_id = Some(workspace.to_owned());
        self.host_tab_id = Some(tab.to_owned());
        self.host_pane_id = Some(pane.to_owned());
    }

    #[cfg(test)]
    pub(crate) fn set_agent_change_stats_for_test(&mut self, path: PathBuf, stats: (u64, u64)) {
        self.agent_change_stats.insert(path, stats);
    }

    #[cfg(test)]
    pub(crate) fn set_stashed_agents_for_test(&mut self, agents: Vec<StashedAgent>) {
        self.stash.agents = agents;
    }

    #[cfg(test)]
    pub(crate) fn set_agent_user_messages_for_test(
        &mut self,
        index: usize,
        messages: &[(&str, Option<&str>, u64, u64)],
    ) {
        let Some(AgentTimingKey::Session(identity)) = self
            .agents
            .get(index)
            .and_then(|agent| agent.runtime.session_timing_key.as_ref())
        else {
            panic!("test agent has no session identity");
        };
        let identity = identity.clone();
        let status = self.agents[index].runtime.status;
        self.latest_user_messages.insert(
            identity.clone(),
            messages
                .iter()
                .map(
                    |(text, latest_agent_text, request_count, tool_call_count)| {
                        let request_count = usize::try_from(*request_count).unwrap_or(usize::MAX);
                        let request_count =
                            request_count.max(usize::from(latest_agent_text.is_some()));
                        let mut requests = (0..request_count)
                            .map(|_| AgentRequestPreview {
                                parts: Vec::new(),
                                reasoning_active: false,
                                tool_call_count: 0,
                            })
                            .collect::<Vec<_>>();
                        if let Some(request) = requests.last_mut() {
                            if let Some(text) = latest_agent_text {
                                request
                                    .parts
                                    .push(AgentRequestPartPreview::Text((*text).to_owned()));
                            }
                            request.tool_call_count = *tool_call_count;
                        }
                        AgentUserMessage {
                            text: (*text).to_owned(),
                            requests,
                        }
                    },
                )
                .collect(),
        );
        self.latest_user_message_refreshes.insert(
            identity.clone(),
            Instant::now() + AGENT_MESSAGE_REFRESH_INTERVAL,
        );
        self.latest_user_message_statuses.insert(identity, status);
    }

    #[cfg(test)]
    pub(crate) fn set_agent_message_activity_for_test(
        &mut self,
        index: usize,
        message: usize,
        activities: &[AgentActivityPreview],
        reasoning_active: bool,
    ) {
        let AgentTimingKey::Session(identity) = self.agents[index]
            .runtime
            .session_timing_key
            .as_ref()
            .expect("test agent has no session identity")
        else {
            panic!("test agent has no session identity");
        };
        let message = &mut self
            .latest_user_messages
            .get_mut(identity)
            .expect("test agent has no messages")[message];
        let request = message
            .requests
            .last_mut()
            .expect("test message has no requests");
        request
            .parts
            .retain(|part| matches!(part, AgentRequestPartPreview::Text(_)));
        request.parts.extend(
            activities
                .iter()
                .cloned()
                .map(AgentRequestPartPreview::Activity),
        );
        request.reasoning_active = reasoning_active;
    }

    #[cfg(test)]
    pub(crate) fn set_agent_request_for_test(
        &mut self,
        index: usize,
        message: usize,
        request: usize,
        preview: AgentRequestPreview,
    ) {
        let AgentTimingKey::Session(identity) = self.agents[index]
            .runtime
            .session_timing_key
            .as_ref()
            .expect("test agent has no session identity")
        else {
            panic!("test agent has no session identity");
        };
        self.latest_user_messages
            .get_mut(identity)
            .expect("test agent has no messages")[message]
            .requests[request] = preview;
    }
}

fn agent_layouts_path(config_dir: &Path, host_pane_id: &str) -> PathBuf {
    let key = host_pane_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    config_dir.join("agent-layouts").join(format!("{key}.json"))
}

fn load_agent_layouts(
    path: &Path,
) -> std::io::Result<HashMap<AgentTimingKey, client::AgentLayout>> {
    let index: AgentLayoutIndex =
        serde_json::from_slice(&fs::read(path)?).map_err(std::io::Error::other)?;
    if index.version != LAYOUT_INDEX_VERSION {
        return Ok(HashMap::new());
    }
    Ok(index
        .layouts
        .into_iter()
        .map(|record| (record.key, record.layout))
        .collect())
}

fn save_agent_layouts(
    path: &Path,
    layouts: &HashMap<AgentTimingKey, client::AgentLayout>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut records = layouts
        .iter()
        .map(|(key, layout)| AgentLayoutRecord {
            key: key.clone(),
            layout: layout.clone(),
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record.key.stable_id());
    let bytes = serde_json::to_vec(&AgentLayoutIndex {
        version: LAYOUT_INDEX_VERSION,
        layouts: records,
    })
    .map_err(std::io::Error::other)?;
    atomic_write(path, &bytes)
}

fn populate_workspace_branches(workspaces: &mut [HerdrWorkspace]) {
    for workspace in workspaces {
        workspace.branch = workspace.path.as_deref().and_then(workspace_branch);
    }
}

fn workspace_branch(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let mut directory = if path.is_dir() { path } else { path.parent()? };
    loop {
        let dot_git = directory.join(".git");
        if dot_git.is_dir() {
            return branch_from_head(&dot_git.join("HEAD"));
        }
        if dot_git.is_file() {
            let git_file = fs::read_to_string(&dot_git).ok()?;
            let git_dir = git_file.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = Path::new(git_dir);
            let git_dir = if git_dir.is_absolute() {
                git_dir.to_path_buf()
            } else {
                directory.join(git_dir)
            };
            return branch_from_head(&git_dir.join("HEAD"));
        }
        directory = directory.parent()?;
    }
}

fn branch_from_head(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .trim()
        .strip_prefix("ref: refs/heads/")
        .filter(|branch| !branch.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod stash_flow_tests {
    use super::*;

    fn unresolved_agent() -> AgentPane {
        AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: "w1:p2".to_owned(),
            cwd: Some(PathBuf::from("/code/hunkle")),
            destination_cwd: Some(PathBuf::from("/code/hunkle")),
            focused: false,
            runtime: AgentRuntime {
                name: "opencode".to_owned(),
                session_name: Some("Resume this later".to_owned()),
                status: AgentStatus::Idle,
                timing_key: AgentTimingKey::Pane("opencode@w1:p2".to_owned()),
                session_timing_key: None,
                state_change_seq: 1,
            },
        }
    }

    #[test]
    fn fallback_stash_hides_the_agent_immediately_and_restores_it_on_failure() {
        let mut session = HerdrSession::new(true, None, None);
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        let agent = unresolved_agent();
        session.agents.push(agent.clone());
        let (release, wait) = mpsc::channel();

        session
            .stash_agent_with_resolver(
                0,
                PathBuf::from("/code/hunkle"),
                "hunkle".to_owned(),
                "main".to_owned(),
                move |_, _| {
                    wait.recv().unwrap();
                    Err("session lookup failed".to_owned())
                },
            )
            .unwrap();

        assert!(session.agents.is_empty());
        session.apply_agent_snapshot_at(vec![agent.clone()], unix_time_ms());
        assert!(session.agents.is_empty());

        release.send(()).unwrap();
        let notice = (0..100).find_map(|_| {
            let poll = session.poll();
            if poll.notice.is_none() {
                thread::sleep(Duration::from_millis(5));
            }
            poll.notice
        });

        assert_eq!(
            notice.as_deref(),
            Some("Could not stash agent: session lookup failed")
        );
        assert_eq!(session.agents, vec![agent]);
        assert!(!session.agent_stash_running);
        assert!(session.stashed_agents().is_empty());
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn saves_agent_layouts_per_hunkle_pane() {
        let directory = tempfile::tempdir().unwrap();
        let path = agent_layouts_path(directory.path(), "w1:p7");
        let key = AgentTimingKey::Terminal("terminal-1".to_owned());
        let layout: client::AgentLayout = serde_json::from_value(serde_json::json!({
            "root": {
                "Split": {
                    "direction": "Right",
                    "ratio": 0.6,
                    "first": { "Pane": "w1:p7" },
                    "second": { "Pane": "w1:p8" }
                }
            }
        }))
        .unwrap();
        let layouts = HashMap::from([(key.clone(), layout.clone())]);

        save_agent_layouts(&path, &layouts).unwrap();

        assert_eq!(load_agent_layouts(&path).unwrap().get(&key), Some(&layout));
        assert_eq!(
            path,
            directory
                .path()
                .join("agent-layouts")
                .join("77313a7037.json")
        );
    }

    #[test]
    fn showing_the_current_agent_keeps_it_visible() {
        let mut session = HerdrSession::new(true, None, None);
        session.set_host_for_test("w1", "w1:t1", "w1:p1");
        session.agents.push(AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: "w1:p2".to_owned(),
            cwd: Some(PathBuf::from("/code/hunkle")),
            destination_cwd: Some(PathBuf::from("/code/hunkle")),
            focused: false,
            runtime: AgentRuntime {
                name: "opencode".to_owned(),
                session_name: None,
                status: AgentStatus::Idle,
                timing_key: AgentTimingKey::Pane("opencode@w1:p2".to_owned()),
                session_timing_key: None,
                state_change_seq: 1,
            },
        });

        session.show_agent(0).unwrap();

        assert!(session.agent_is_in_host_tab(0));
        assert!(!session.agent_display_running);
    }
}
