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
    AgentPaneDirection, AgentPromptDelivery, LinkedWorktreeCandidate, LinkedWorktreeObservation,
    settings::AgentTimeDisplay,
};
use crate::filesystem::atomic_write;

mod client;
mod latest_message;
mod scheduler;
mod stash;
mod timings;

pub(crate) use stash::StashedAgent;

pub(crate) use client::HerdrPaneLayout;
#[cfg(test)]
pub(crate) use client::HerdrPaneRect;
pub(crate) use scheduler::{
    ProjectTaskStatus, ScheduledRun, ScheduledRunStatus, ScheduledTask, ScheduledTaskDestination,
    ScheduledTaskEdit,
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const AGENT_MESSAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const TIMING_LAST_SEEN_INTERVAL_MS: u64 = 60_000;
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);
const LAYOUT_INDEX_VERSION: u8 = 1;
const MAX_AGENT_LAYOUTS: usize = 64;
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

fn agent_has_opencode_session(agent: &AgentPane, session_id: &str) -> bool {
    [&agent.runtime.timing_key]
        .into_iter()
        .chain(agent.runtime.session_timing_key.as_ref())
        .any(|key| {
            matches!(
                key,
                AgentTimingKey::Session(identity)
                    if identity.agent == "opencode" && identity.value == session_id
            )
        })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentPromptOutcome {
    Sending,
    Queued,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OpenCodeConversationIdentity {
    session_id: String,
}

impl OpenCodeConversationIdentity {
    fn from_agent_session(identity: &AgentSessionIdentity) -> Option<Self> {
        (identity.agent == "opencode").then(|| Self {
            session_id: identity.value.clone(),
        })
    }

    fn from_session_id(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
        }
    }
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
    pub(crate) terminal_id: Option<String>,
    pub(crate) instance_name: Option<String>,
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
    #[serde(default)]
    last_used_ms: u64,
}

struct SavedAgentLayout {
    layout: client::AgentLayout,
    last_used_ms: u64,
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
    pub(crate) fullscreen_result: Option<Result<bool, String>>,
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
    LatestUserMessage {
        identity: OpenCodeConversationIdentity,
        result: Result<latest_message::TranscriptFetch, String>,
    },
    AgentPrompt {
        key: AgentTimingKey,
        result: Result<(), String>,
    },
    ScheduledPrompt {
        run_id: i64,
        session_id: Option<String>,
        result: Result<(), String>,
    },
    ScheduledSession {
        run_id: i64,
        result: Result<String, String>,
    },
    ScheduledConversation {
        identity: OpenCodeConversationIdentity,
        result: Result<latest_message::TranscriptFetch, String>,
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

#[derive(Clone, Copy)]
pub(crate) struct AgentTranscript<'a> {
    pub(crate) identity: &'a str,
    pub(crate) revision: u64,
    pub(crate) messages: &'a [AgentUserMessage],
}

struct AgentTranscriptEntry {
    revision: u64,
    messages: Vec<AgentUserMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRequestPreview {
    pub(crate) parts: Vec<AgentRequestPartPreview>,
    pub(crate) reasoning_active: bool,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) reasoning_duration_ms: Option<u64>,
    pub(crate) tool_call_count: u64,
}

fn scheduled_conversation_identity(session_id: &str) -> OpenCodeConversationIdentity {
    OpenCodeConversationIdentity::from_session_id(session_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentListMode {
    Agents,
    Scheduled,
    Stash,
}

impl AgentListMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Agents => Self::Scheduled,
            Self::Scheduled => Self::Stash,
            Self::Stash => Self::Agents,
        }
    }
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
    observed_agents: Vec<AgentPane>,
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
    latest_user_messages: HashMap<OpenCodeConversationIdentity, AgentTranscriptEntry>,
    transcript_revision: u64,
    latest_user_message_requests: HashSet<OpenCodeConversationIdentity>,
    latest_user_message_refreshes: HashMap<OpenCodeConversationIdentity, Instant>,
    latest_user_message_statuses: HashMap<OpenCodeConversationIdentity, AgentStatus>,
    latest_user_message_errors: HashMap<OpenCodeConversationIdentity, String>,
    agent_prompt_requests: HashSet<AgentTimingKey>,
    agent_prompts_on_idle: HashMap<AgentTimingKey, String>,
    agent_prompt_errors: HashMap<AgentTimingKey, String>,
    scheduled_prompt_requests: HashSet<i64>,
    scheduled_prompt_errors: HashMap<i64, String>,
    agent_prompt_notice: Option<String>,
    scheduled_preview_identity: Option<OpenCodeConversationIdentity>,
    scheduled_session_requests: HashSet<i64>,
    scheduled_session_refreshes: HashMap<i64, Instant>,
    scheduled_session_errors: HashMap<i64, String>,
    scheduled_conversation_errors: HashMap<OpenCodeConversationIdentity, String>,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    next_refresh: Instant,
    spinner_frame: usize,
    next_spinner: Instant,
    agent_timings: HashMap<AgentTimingKey, AgentTiming>,
    agent_timing_persistence: Option<timings::Persistence>,
    agent_timing_clear_generation: u64,
    agent_timing_persistence_notice: Option<String>,
    agent_layouts: HashMap<AgentTimingKey, SavedAgentLayout>,
    agent_layouts_path: Option<PathBuf>,
    displayed_agent_key: Option<AgentTimingKey>,
    stash: stash::AgentStashStore,
    agent_list_mode: AgentListMode,
    pub(crate) stash_scroll: usize,
    pub(crate) scheduled_run_scroll: usize,
    agent_stash_running: bool,
    pending_agent_stash: Option<PendingAgentStash>,
    stashed_pane_ids: HashSet<String>,
    scheduler: Option<scheduler::SchedulerService>,
    scheduler_error: Option<String>,
}

impl HerdrSession {
    pub(crate) fn detect(
        config_dir: Option<&Path>,
        discord_webhooks: Vec<crate::app::DiscordWebhookConfig>,
    ) -> Self {
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
            let files_root = crate::paths::data_root().map_err(|error| error.to_string());
            match files_root.and_then(|files_root| {
                scheduler::SchedulerService::open(
                    config_dir.map(|path| path.join("scheduler.sqlite3")),
                    Some(files_root),
                    discord_webhooks,
                )
            }) {
                Ok(scheduler) => session.scheduler = Some(scheduler),
                Err(error) => session.scheduler_error = Some(error),
            }
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
            observed_agents: Vec::new(),
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
            latest_user_messages: HashMap::new(),
            transcript_revision: 0,
            latest_user_message_requests: HashSet::new(),
            latest_user_message_refreshes: HashMap::new(),
            latest_user_message_statuses: HashMap::new(),
            latest_user_message_errors: HashMap::new(),
            agent_prompt_requests: HashSet::new(),
            agent_prompts_on_idle: HashMap::new(),
            agent_prompt_errors: HashMap::new(),
            scheduled_prompt_requests: HashSet::new(),
            scheduled_prompt_errors: HashMap::new(),
            agent_prompt_notice: None,
            scheduled_preview_identity: None,
            scheduled_session_requests: HashSet::new(),
            scheduled_session_refreshes: HashMap::new(),
            scheduled_session_errors: HashMap::new(),
            scheduled_conversation_errors: HashMap::new(),
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
            agent_list_mode: AgentListMode::Agents,
            stash_scroll: 0,
            scheduled_run_scroll: 0,
            agent_stash_running: false,
            pending_agent_stash: None,
            stashed_pane_ids: HashSet::new(),
            scheduler: None,
            scheduler_error: None,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn scheduled_tasks(&self) -> &[ScheduledTask] {
        self.scheduler
            .as_ref()
            .map_or(&[], |scheduler| scheduler.tasks.as_slice())
    }

    pub(crate) fn scheduled_runs(&self) -> &[ScheduledRun] {
        self.scheduler
            .as_ref()
            .map_or(&[], |scheduler| scheduler.runs.as_slice())
    }

    pub(crate) fn save_scheduled_task(
        &self,
        id: Option<i64>,
        edit: ScheduledTaskEdit,
    ) -> Result<(), String> {
        self.scheduler_service()?.save_task(id, edit)
    }

    pub(crate) fn configure_project_task(
        &self,
        id: i64,
        discord_webhook_id: String,
    ) -> Result<(), String> {
        self.scheduler_service()?
            .configure_project_task(id, discord_webhook_id)
    }

    pub(crate) fn discover_project_tasks(
        &self,
        destination: ScheduledTaskDestination,
        repository_identity: PathBuf,
    ) -> Result<(), String> {
        self.scheduler_service()?
            .discover_project_tasks(destination, repository_identity)
    }

    pub(crate) fn toggle_scheduled_task(&self, id: i64, enabled: bool) -> Result<(), String> {
        self.scheduler_service()?.toggle_task(id, enabled)
    }

    pub(crate) fn delete_scheduled_task(&self, id: i64) -> Result<(), String> {
        self.scheduler_service()?.delete_task(id)
    }

    pub(crate) fn run_scheduled_task_now(&self, id: i64) -> Result<(), String> {
        self.scheduler_service()?.run_now(id)
    }

    pub(crate) fn refresh_scheduled_run(&self, id: i64) -> Result<(), String> {
        self.scheduler_service()?.refresh_run(id)
    }

    pub(crate) fn configure_discord_webhooks(
        &self,
        webhooks: Vec<crate::app::DiscordWebhookConfig>,
    ) -> Result<(), String> {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return Ok(());
        };
        scheduler.configure_discord_webhooks(webhooks)
    }

    pub(crate) fn test_discord_webhook(&self, channel: String) -> Result<(), String> {
        self.scheduler_service()?.test_discord_webhook(channel)
    }

    pub(crate) fn resolve_scheduled_run_session(
        &mut self,
        run_id: i64,
        directory: PathBuf,
        prompt: String,
        run_created_at_ms: i64,
    ) {
        let now = Instant::now();
        if self.scheduled_session_requests.contains(&run_id)
            || self
                .scheduled_session_refreshes
                .get(&run_id)
                .is_some_and(|refresh| now < *refresh)
        {
            return;
        }
        self.scheduled_session_requests.insert(run_id);
        self.scheduled_session_refreshes
            .insert(run_id, now + AGENT_MESSAGE_REFRESH_INTERVAL);
        self.scheduled_session_errors.remove(&run_id);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = latest_message::resolve_scheduled_session_id(
                &directory,
                &prompt,
                run_created_at_ms,
            );
            let _ = sender.send(Completion::ScheduledSession { run_id, result });
        });
    }

    pub(crate) fn scheduled_session_error(&self, run_id: i64) -> Option<&str> {
        self.scheduled_session_errors
            .get(&run_id)
            .map(String::as_str)
    }

    pub(crate) fn request_scheduled_conversation(&mut self, session_id: &str, active: bool) {
        let identity = scheduled_conversation_identity(session_id);
        self.scheduled_preview_identity = Some(identity.clone());
        let now = Instant::now();
        if (self.latest_user_messages.contains_key(&identity) && !active)
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
        self.scheduled_conversation_errors.remove(&identity);
        let session_id = session_id.to_owned();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = latest_message::fetch(&session_id, false);
            let _ = sender.send(Completion::ScheduledConversation { identity, result });
        });
    }

    pub(crate) fn refresh_scheduled_conversation(&mut self, session_id: &str) {
        let identity = scheduled_conversation_identity(session_id);
        self.scheduled_preview_identity = Some(identity.clone());
        if self.latest_user_message_requests.contains(&identity) {
            return;
        }
        self.latest_user_message_requests.insert(identity.clone());
        self.latest_user_message_refreshes.remove(&identity);
        self.scheduled_conversation_errors.remove(&identity);
        let session_id = session_id.to_owned();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = latest_message::fetch(&session_id, false);
            let _ = sender.send(Completion::ScheduledConversation { identity, result });
        });
    }

    pub(crate) fn scheduled_conversation(&self, session_id: &str) -> Option<&[AgentUserMessage]> {
        self.scheduled_transcript(session_id)
            .map(|transcript| transcript.messages)
    }

    pub(crate) fn scheduled_transcript(&self, session_id: &str) -> Option<AgentTranscript<'_>> {
        let identity = scheduled_conversation_identity(session_id);
        let (identity, transcript) = self.latest_user_messages.get_key_value(&identity)?;
        Some(AgentTranscript {
            identity: identity.session_id.as_str(),
            revision: transcript.revision,
            messages: transcript.messages.as_slice(),
        })
    }

    pub(crate) fn scheduled_conversation_error(&self, session_id: &str) -> Option<&str> {
        self.scheduled_conversation_errors
            .get(&scheduled_conversation_identity(session_id))
            .map(String::as_str)
    }

    pub(crate) fn clear_scheduled_conversation(&mut self) {
        self.scheduled_preview_identity = None;
    }

    fn scheduler_service(&self) -> Result<&scheduler::SchedulerService, String> {
        self.scheduler
            .as_ref()
            .ok_or_else(|| "scheduler is unavailable".to_owned())
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(scheduler) = self.scheduler.as_mut() {
            scheduler.shutdown();
        }
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

    pub(crate) fn poll(&mut self, agent_animation_presented: bool) -> HerdrSessionPoll {
        if !self.enabled {
            return HerdrSessionPoll::default();
        }
        let mut poll = HerdrSessionPoll::default();
        if let Some(scheduler) = self.scheduler.as_mut() {
            let (changed, error) = scheduler.poll_completions();
            poll.changed |= changed;
            if let Some(error) = error {
                poll.notice = Some(format!("Scheduler: {error}"));
            }
        }
        if let Some(error) = self.scheduler_error.take() {
            poll.notice = Some(format!("Could not open scheduler: {error}"));
        }
        if let Some(notice) = self.agent_timing_persistence_notice.take() {
            poll.notice = Some(notice);
        }
        if let Some(notice) = self.agent_prompt_notice.take() {
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
            let mut completion_changed = true;
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
                                saved.layout.remap_known(&pane_locations);
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
                            let last_used_ms = unix_time_ms();
                            if let (Some(key), Some(layout)) = (outgoing_key, parked) {
                                self.agent_layouts.insert(
                                    key,
                                    SavedAgentLayout {
                                        layout,
                                        last_used_ms,
                                    },
                                );
                            }
                            self.agent_layouts.insert(
                                selected_key.clone(),
                                SavedAgentLayout {
                                    layout: displayed,
                                    last_used_ms,
                                },
                            );
                            self.displayed_agent_key = Some(selected_key);
                            self.prune_agent_layout_history();
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
                    poll.notice = Some(match &result {
                        Ok(fullscreen) => {
                            self.fullscreen = *fullscreen;
                            if *fullscreen {
                                "Hunkle is fullscreen".to_owned()
                            } else {
                                "Restored Herdr tab layout".to_owned()
                            }
                        }
                        Err(error) => format!("Could not toggle fullscreen: {error}"),
                    });
                    poll.fullscreen_result = Some(result);
                }
                Completion::LatestUserMessage { identity, result } => {
                    completion_changed = false;
                    self.latest_user_message_requests.remove(&identity);
                    // Snapshot pruning removes this marker to invalidate departed agents' requests.
                    if self.latest_user_message_refreshes.contains_key(&identity) {
                        match result {
                            Ok(latest_message::TranscriptFetch::Changed(messages)) => {
                                completion_changed =
                                    self.latest_user_message_errors.remove(&identity).is_some();
                                if self.update_transcript(identity, messages) {
                                    completion_changed = true;
                                }
                            }
                            Ok(latest_message::TranscriptFetch::Unchanged) => {
                                completion_changed =
                                    self.latest_user_message_errors.remove(&identity).is_some();
                            }
                            Err(error) => {
                                self.latest_user_message_errors.insert(identity, error);
                                completion_changed = true;
                            }
                        }
                    }
                }
                Completion::AgentPrompt { key, result } => {
                    self.agent_prompt_requests.remove(&key);
                    self.next_refresh = Instant::now();
                    poll.notice = Some(match result {
                        Ok(()) => {
                            self.agent_prompt_errors.remove(&key);
                            if let Some(identity) = self.agents.iter().find_map(|agent| {
                                (agent.runtime.timing_key == key)
                                    .then_some(agent.runtime.session_timing_key.as_ref())
                                    .flatten()
                                    .and_then(|key| match key {
                                        AgentTimingKey::Session(identity) => {
                                            OpenCodeConversationIdentity::from_agent_session(
                                                identity,
                                            )
                                        }
                                        _ => None,
                                    })
                            }) {
                                self.latest_user_message_refreshes.remove(&identity);
                                self.latest_user_message_statuses.remove(&identity);
                            }
                            "Message sent to agent".to_owned()
                        }
                        Err(error) => {
                            self.agent_prompt_errors.insert(key, error.clone());
                            format!("Could not message agent: {error}")
                        }
                    });
                }
                Completion::ScheduledPrompt {
                    run_id,
                    session_id,
                    result,
                } => {
                    self.scheduled_prompt_requests.remove(&run_id);
                    poll.notice = Some(match result {
                        Ok(()) => {
                            self.scheduled_prompt_errors.remove(&run_id);
                            if let Some(session_id) = session_id {
                                self.refresh_scheduled_conversation(&session_id);
                            }
                            "Message sent to scheduled agent".to_owned()
                        }
                        Err(error) => {
                            self.scheduled_prompt_errors.insert(run_id, error.clone());
                            format!("Could not message scheduled agent: {error}")
                        }
                    });
                }
                Completion::ScheduledSession { run_id, result } => {
                    self.scheduled_session_requests.remove(&run_id);
                    match result {
                        Ok(session_id) => {
                            self.scheduled_session_errors.remove(&run_id);
                            if let Some(scheduler) = self.scheduler.as_mut() {
                                scheduler.bind_session(run_id, session_id);
                            }
                        }
                        Err(error) => {
                            self.scheduled_session_errors.insert(run_id, error);
                        }
                    }
                }
                Completion::ScheduledConversation { identity, result } => {
                    self.latest_user_message_requests.remove(&identity);
                    match result {
                        Ok(latest_message::TranscriptFetch::Changed(messages)) => {
                            completion_changed = self
                                .scheduled_conversation_errors
                                .remove(&identity)
                                .is_some();
                            if self.update_transcript(identity, messages) {
                                completion_changed = true;
                            }
                        }
                        Ok(latest_message::TranscriptFetch::Unchanged) => {
                            completion_changed = self
                                .scheduled_conversation_errors
                                .remove(&identity)
                                .is_some();
                        }
                        Err(error) => {
                            self.scheduled_conversation_errors.insert(identity, error);
                            completion_changed = true;
                        }
                    }
                }
            }
            poll.changed |= completion_changed;
        }
        if !self.loading && Instant::now() >= self.next_refresh {
            self.start_snapshot();
            poll.changed = true;
        }
        poll.changed |= self.poll_spinner(Instant::now(), agent_animation_presented);
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
        self.observed_agents = agents.clone();
        let observed_keys = agents
            .iter()
            .map(|agent| agent.runtime.timing_key.clone())
            .collect::<HashSet<_>>();
        let queued_before = self.agent_prompts_on_idle.len();
        self.agent_prompts_on_idle
            .retain(|key, _| observed_keys.contains(key));
        if self.agent_prompts_on_idle.len() != queued_before {
            self.agent_prompt_notice =
                Some("Queued message cancelled because the agent departed".to_owned());
        }
        self.bind_legacy_scheduled_agents();
        let agents = if self.cross_workspace_agents || self.host_workspace_id.is_none() {
            agents
        } else {
            agents
                .into_iter()
                .filter(|agent| self.host_workspace_id.as_deref() == Some(&agent.workspace_id))
                .collect()
        };
        let active_identities = agents
            .iter()
            .filter_map(|agent| match agent.runtime.session_timing_key.as_ref() {
                Some(AgentTimingKey::Session(identity)) => {
                    OpenCodeConversationIdentity::from_agent_session(identity)
                }
                _ => None,
            })
            .chain(self.scheduled_preview_identity.iter().cloned())
            .collect::<HashSet<_>>();
        self.latest_user_messages
            .retain(|identity, _| active_identities.contains(identity));
        self.latest_user_message_requests
            .retain(|identity| active_identities.contains(identity));
        self.latest_user_message_refreshes
            .retain(|identity, _| active_identities.contains(identity));
        self.latest_user_message_statuses
            .retain(|identity, _| active_identities.contains(identity));
        self.latest_user_message_errors
            .retain(|identity, _| active_identities.contains(identity));
        let active_agent_keys = agents
            .iter()
            .map(|agent| agent.runtime.timing_key.clone())
            .collect::<HashSet<_>>();
        self.agent_prompt_errors
            .retain(|key, _| active_agent_keys.contains(key));
        self.scheduled_conversation_errors
            .retain(|identity, _| active_identities.contains(identity));
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
        self.dispatch_idle_agent_prompts();
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
        self.prune_agent_layout_history();
        self.agent_scroll = self
            .agent_scroll
            .min(self.agent_card_count().saturating_sub(1));
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

    pub(crate) fn prompt_agent(
        &mut self,
        index: usize,
        prompt: String,
        delivery: AgentPromptDelivery,
    ) -> Result<AgentPromptOutcome, String> {
        if prompt.trim().is_empty() {
            return Err("Enter a message".to_owned());
        }
        let agent = self
            .agents
            .get(index)
            .ok_or_else(|| "Agent is no longer available".to_owned())?;
        let key = agent.runtime.timing_key.clone();
        if self.agent_prompt_requests.contains(&key)
            || self.agent_prompts_on_idle.contains_key(&key)
        {
            return Err("A message is already queued for this agent".to_owned());
        }
        if delivery == AgentPromptDelivery::OnIdle
            && !matches!(agent.runtime.status, AgentStatus::Idle | AgentStatus::Done)
        {
            self.agent_prompt_errors.remove(&key);
            self.agent_prompts_on_idle.insert(key, prompt);
            return Ok(AgentPromptOutcome::Queued);
        }
        let pane_id = agent.pane_id.clone();
        self.start_agent_prompt(key, pane_id, prompt);
        Ok(AgentPromptOutcome::Sending)
    }

    pub(crate) fn prompt_scheduled_run(
        &mut self,
        run_id: i64,
        prompt: String,
    ) -> Result<(), String> {
        if prompt.trim().is_empty() {
            return Err("Enter a message".to_owned());
        }
        if self.scheduled_prompt_requests.contains(&run_id) {
            return Err("A message is already being sent to this run".to_owned());
        }
        let run = self
            .scheduled_runs()
            .iter()
            .find(|run| run.id == run_id)
            .ok_or_else(|| "Scheduled run is no longer available".to_owned())?;
        let pane_id = self
            .scheduled_run_prompt_pane(run)
            .ok_or_else(|| "Scheduled run is no longer attached to its Herdr agent".to_owned())?;
        let session_id = run.session_id.clone();
        self.scheduled_prompt_requests.insert(run_id);
        self.scheduled_prompt_errors.remove(&run_id);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = client::prompt_agent(pane_id, prompt);
            let _ = sender.send(Completion::ScheduledPrompt {
                run_id,
                session_id,
                result,
            });
        });
        Ok(())
    }

    pub(crate) fn scheduled_prompt_sending(&self, run_id: i64) -> bool {
        self.scheduled_prompt_requests.contains(&run_id)
    }

    pub(crate) fn scheduled_prompt_available(&self, run_id: i64) -> bool {
        self.scheduled_runs()
            .iter()
            .find(|run| run.id == run_id)
            .and_then(|run| self.scheduled_run_prompt_pane(run))
            .is_some()
    }

    pub(crate) fn scheduled_prompt_error(&self, run_id: i64) -> Option<&str> {
        self.scheduled_prompt_errors
            .get(&run_id)
            .map(String::as_str)
    }

    pub(crate) fn clear_scheduled_prompt_error(&mut self, run_id: i64) {
        self.scheduled_prompt_errors.remove(&run_id);
    }

    fn scheduled_run_prompt_pane(&self, run: &ScheduledRun) -> Option<String> {
        let pane_id = run.pane_id.as_deref()?;
        self.observed_agents
            .iter()
            .find(|agent| {
                agent.pane_id == pane_id
                    && run
                        .session_id
                        .as_deref()
                        .is_some_and(|session_id| agent_has_opencode_session(agent, session_id))
            })
            .map(|agent| agent.pane_id.clone())
    }

    fn start_agent_prompt(&mut self, key: AgentTimingKey, pane_id: String, prompt: String) {
        self.agent_prompt_requests.insert(key.clone());
        self.agent_prompt_errors.remove(&key);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = client::prompt_agent(pane_id, prompt);
            let _ = sender.send(Completion::AgentPrompt { key, result });
        });
    }

    fn dispatch_idle_agent_prompts(&mut self) {
        let ready = self.take_idle_agent_prompts();
        for (key, pane_id, prompt) in ready {
            self.start_agent_prompt(key, pane_id, prompt);
        }
    }

    fn take_idle_agent_prompts(&mut self) -> Vec<(AgentTimingKey, String, String)> {
        let ready = self
            .observed_agents
            .iter()
            .filter(|agent| matches!(agent.runtime.status, AgentStatus::Idle | AgentStatus::Done))
            .filter_map(|agent| {
                self.agent_prompts_on_idle
                    .contains_key(&agent.runtime.timing_key)
                    .then(|| (agent.runtime.timing_key.clone(), agent.pane_id.clone()))
            })
            .collect::<Vec<_>>();
        let mut prompts = Vec::with_capacity(ready.len());
        for (key, pane_id) in ready {
            if let Some(prompt) = self.agent_prompts_on_idle.remove(&key) {
                prompts.push((key, pane_id, prompt));
            }
        }
        prompts
    }

    pub(crate) fn agent_prompt_sending(&self, index: usize) -> bool {
        self.agent_prompt_pending(index).is_some()
    }

    pub(crate) fn agent_prompt_pending(&self, index: usize) -> Option<AgentPromptOutcome> {
        let key = &self.agents.get(index)?.runtime.timing_key;
        if self.agent_prompts_on_idle.contains_key(key) {
            Some(AgentPromptOutcome::Queued)
        } else if self.agent_prompt_requests.contains(key) {
            Some(AgentPromptOutcome::Sending)
        } else {
            None
        }
    }

    pub(crate) fn agent_prompt_error(&self, index: usize) -> Option<&str> {
        let key = &self.agents.get(index)?.runtime.timing_key;
        self.agent_prompt_errors.get(key).map(String::as_str)
    }

    pub(crate) fn clear_agent_prompt_error(&mut self, index: usize) {
        if let Some(key) = self
            .agents
            .get(index)
            .map(|agent| agent.runtime.timing_key.clone())
        {
            self.agent_prompt_errors.remove(&key);
        }
    }

    pub(crate) fn scheduled_run_agent_index(&self, run: &ScheduledRun) -> Option<usize> {
        self.agents.iter().position(|agent| {
            if let Some(session_id) = run.session_id.as_deref() {
                return agent_has_opencode_session(agent, session_id);
            }
            if let Some(terminal_id) = run.terminal_id.as_deref() {
                return agent.terminal_id.as_deref() == Some(terminal_id);
            }
            run.pane_id.as_deref() == Some(agent.pane_id.as_str())
        })
    }

    fn bind_legacy_scheduled_agents(&mut self) {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return;
        };
        let bindings = scheduler
            .runs
            .iter()
            .filter(|run| run.terminal_id.is_none() || run.session_id.is_none())
            .filter_map(|run| {
                let exact = run.pane_id.as_deref().and_then(|pane_id| {
                    self.observed_agents
                        .iter()
                        .find(|agent| agent.pane_id == pane_id)
                });
                let agent = exact.or_else(|| {
                    let task = scheduler.tasks.iter().find(|task| task.id == run.task_id)?;
                    let name = client::scheduler_agent_name(&format!(
                        "Hunkle: {} #{}",
                        task.title, task.id
                    ));
                    let run_name = client::scheduler_run_agent_name(
                        &format!("Hunkle: {} #{}", task.title, task.id),
                        run.id,
                    );
                    let mut candidates = self.observed_agents.iter().filter(|agent| {
                        agent
                            .instance_name
                            .as_deref()
                            .is_some_and(|candidate| candidate == name || candidate == run_name)
                            && agent
                                .destination_cwd
                                .as_deref()
                                .or(agent.cwd.as_deref())
                                .is_some_and(|path| {
                                    crate::filesystem::same_path(path, &task.destination)
                                })
                    });
                    let candidate = candidates.next()?;
                    candidates.next().is_none().then_some(candidate)
                })?;
                let session_id = match agent.runtime.session_timing_key.as_ref() {
                    Some(AgentTimingKey::Session(identity)) => {
                        OpenCodeConversationIdentity::from_agent_session(identity)
                            .map(|identity| identity.session_id)
                    }
                    _ => None,
                };
                Some((
                    run.id,
                    run.terminal_id.is_none(),
                    agent.pane_id.clone(),
                    agent.terminal_id.clone(),
                    run.session_id.is_none().then_some(session_id).flatten(),
                ))
            })
            .collect::<Vec<_>>();
        let Some(scheduler) = self.scheduler.as_mut() else {
            return;
        };
        for (run_id, bind_agent, pane_id, terminal_id, session_id) in bindings {
            if bind_agent && let Some(terminal_id) = terminal_id {
                scheduler.bind_agent(run_id, pane_id, terminal_id);
            }
            if let Some(session_id) = session_id {
                scheduler.bind_session(run_id, session_id);
            }
        }
    }

    pub(crate) fn agent_index(&self, key: &AgentKey) -> Option<usize> {
        self.agents
            .iter()
            .position(|agent| agent.runtime.timing_key == key.0)
    }

    pub(crate) fn agent_card_groups(&self) -> Vec<(usize, usize)> {
        let mut groups: Vec<(usize, usize)> = Vec::new();
        for index in 0..self.agents.len() {
            let agent = &self.agents[index];
            if let Some((_, count)) = groups.iter_mut().find(|(representative, _)| {
                let representative = &self.agents[*representative];
                representative.workspace_id == agent.workspace_id
                    && representative.tab_id == agent.tab_id
            }) {
                *count += 1;
            } else {
                groups.push((index, 1));
            }
        }
        groups
    }

    pub(crate) fn agent_card_count(&self) -> usize {
        self.agent_card_groups().len()
    }

    pub(crate) fn agent_card_index(&self, index: usize) -> Option<usize> {
        let agent = self.agents.get(index)?;
        self.agent_card_groups()
            .iter()
            .position(|(representative, _)| {
                let representative = &self.agents[*representative];
                representative.workspace_id == agent.workspace_id
                    && representative.tab_id == agent.tab_id
            })
    }

    pub(crate) fn stashed_agents(&self) -> &[StashedAgent] {
        &self.stash.agents
    }

    pub(crate) fn agent_list_mode(&self) -> AgentListMode {
        self.agent_list_mode
    }

    pub(crate) fn cycle_agent_list_mode(&mut self) {
        self.agent_list_mode = self.agent_list_mode.next();
        self.stash_scroll = 0;
        self.scheduled_run_scroll = 0;
        self.agent_scroll = 0;
    }

    pub(crate) fn show_live_agents(&mut self) {
        self.agent_list_mode = AgentListMode::Agents;
        self.stash_scroll = 0;
        self.scheduled_run_scroll = 0;
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
        self.agent_scroll = self
            .agent_scroll
            .min(self.agent_card_count().saturating_sub(1));
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
        self.agent_transcript(index)
            .map(|transcript| transcript.messages)
    }

    pub(crate) fn agent_transcript(&self, index: usize) -> Option<AgentTranscript<'_>> {
        let AgentTimingKey::Session(identity) = self
            .agents
            .get(index)?
            .runtime
            .session_timing_key
            .as_ref()?
        else {
            return None;
        };
        let conversation = OpenCodeConversationIdentity::from_agent_session(identity)?;
        let transcript = self.latest_user_messages.get(&conversation)?;
        Some(AgentTranscript {
            identity: identity.value.as_str(),
            revision: transcript.revision,
            messages: transcript.messages.as_slice(),
        })
    }

    pub(crate) fn has_transcript(&self, identity: &str) -> bool {
        self.latest_user_messages
            .keys()
            .any(|candidate| candidate.session_id == identity)
    }

    fn update_transcript(
        &mut self,
        identity: OpenCodeConversationIdentity,
        messages: Vec<AgentUserMessage>,
    ) -> bool {
        if self
            .latest_user_messages
            .get(&identity)
            .is_some_and(|transcript| transcript.messages == messages)
        {
            return false;
        }
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.latest_user_messages.insert(
            identity,
            AgentTranscriptEntry {
                revision: self.transcript_revision,
                messages,
            },
        );
        true
    }

    pub(crate) fn agent_user_message_error(&self, index: usize) -> Option<&str> {
        let AgentTimingKey::Session(identity) = self
            .agents
            .get(index)?
            .runtime
            .session_timing_key
            .as_ref()?
        else {
            return None;
        };
        let identity = OpenCodeConversationIdentity::from_agent_session(identity)?;
        self.latest_user_message_errors
            .get(&identity)
            .map(String::as_str)
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
        let Some(identity) = OpenCodeConversationIdentity::from_agent_session(identity) else {
            return;
        };
        let now = Instant::now();
        let allow_unchanged = self.latest_user_messages.contains_key(&identity);
        let needs_refresh = !allow_unchanged
            || self.latest_user_message_statuses.get(&identity) != Some(&status)
            || status == AgentStatus::Working;
        if !needs_refresh
            || self.latest_user_message_requests.contains(&identity)
            || self
                .latest_user_message_refreshes
                .get(&identity)
                .is_some_and(|refresh| now < *refresh)
        {
            return;
        }

        self.latest_user_message_requests.insert(identity.clone());
        self.latest_user_message_errors.remove(&identity);
        self.latest_user_message_refreshes
            .insert(identity.clone(), now + AGENT_MESSAGE_REFRESH_INTERVAL);
        self.latest_user_message_statuses
            .insert(identity.clone(), status);
        let session_id = identity.session_id.clone();
        let sender = self.sender.clone();
        let _ = thread::Builder::new()
            .name("agent-latest-message".to_owned())
            .spawn(move || {
                let result = latest_message::fetch(&session_id, allow_unchanged);
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

    pub(crate) fn agent_stats_destinations(&self) -> impl Iterator<Item = &Path> {
        self.agents
            .iter()
            .filter_map(|agent| agent.destination_cwd.as_deref())
    }

    pub(crate) fn agent_destination(&self, index: usize) -> Option<&Path> {
        let agent = self.agents.get(index)?;
        agent.destination_cwd.as_deref().or(agent.cwd.as_deref())
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
            saved_layout: self
                .agent_layouts
                .get(&agent.runtime.timing_key)
                .map(|saved| saved.layout.clone()),
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

    fn prune_agent_layout_history(&mut self) {
        let relevant = self
            .agents
            .iter()
            .map(|agent| agent.runtime.timing_key.clone())
            .chain(self.displayed_agent_key.iter().cloned())
            .collect::<HashSet<_>>();
        prune_agent_layouts(&mut self.agent_layouts, relevant.iter());
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
        match self.agent_list_mode {
            AgentListMode::Agents => {
                self.agent_scroll = self.agent_scroll.saturating_add_signed(delta);
            }
            AgentListMode::Scheduled => {
                self.scheduled_run_scroll = self.scheduled_run_scroll.saturating_add_signed(delta);
            }
            AgentListMode::Stash => {
                self.stash_scroll = self.stash_scroll.saturating_add_signed(delta);
            }
        }
    }

    pub(crate) fn spinner_frame(&self) -> usize {
        self.spinner_frame
    }

    fn poll_spinner(&mut self, now: Instant, presented: bool) -> bool {
        if !presented {
            self.next_spinner = now;
            return false;
        }
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
        self.observe_agent_status_event_at(event, now_ms);
        self.dispatch_idle_agent_prompts();
    }

    fn observe_agent_status_event_at(&mut self, event: client::AgentStatusEvent, now_ms: u64) {
        if let Some(agent) = self.observed_agents.iter_mut().find(|agent| {
            agent.workspace_id == event.workspace_id && agent.pane_id == event.pane_id
        }) {
            agent.runtime.status = event.status;
        }
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
    pub(crate) fn set_scheduled_tasks_for_test(&mut self, tasks: Vec<ScheduledTask>) {
        let mut scheduler = scheduler::SchedulerService::open(None, None, Vec::new()).unwrap();
        scheduler.tasks = tasks;
        self.scheduler = Some(scheduler);
    }

    #[cfg(test)]
    pub(crate) fn set_scheduled_runs_for_test(&mut self, runs: Vec<ScheduledRun>) {
        self.scheduler.as_mut().unwrap().runs = runs;
    }

    #[cfg(test)]
    pub(crate) fn set_scheduled_conversation_for_test(
        &mut self,
        session_id: &str,
        user: &str,
        response: &str,
    ) {
        self.update_transcript(
            scheduled_conversation_identity(session_id),
            vec![AgentUserMessage {
                text: user.to_owned(),
                requests: vec![AgentRequestPreview {
                    parts: vec![AgentRequestPartPreview::Text(response.to_owned())],
                    reasoning_active: false,
                    duration_ms: None,
                    reasoning_duration_ms: None,
                    tool_call_count: 0,
                }],
            }],
        );
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
    pub(crate) fn set_fullscreen_for_test(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen;
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
        let identity = OpenCodeConversationIdentity::from_agent_session(identity)
            .expect("test agent is not OpenCode");
        let status = self.agents[index].runtime.status;
        let messages = messages
            .iter()
            .map(
                |(text, latest_agent_text, request_count, tool_call_count)| {
                    let request_count = usize::try_from(*request_count).unwrap_or(usize::MAX);
                    let request_count = request_count.max(usize::from(latest_agent_text.is_some()));
                    let mut requests = (0..request_count)
                        .map(|_| AgentRequestPreview {
                            parts: Vec::new(),
                            reasoning_active: false,
                            duration_ms: None,
                            reasoning_duration_ms: None,
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
            .collect();
        self.update_transcript(identity.clone(), messages);
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
        duration_ms: Option<u64>,
        reasoning_duration_ms: Option<u64>,
    ) {
        let AgentTimingKey::Session(identity) = self.agents[index]
            .runtime
            .session_timing_key
            .as_ref()
            .expect("test agent has no session identity")
        else {
            panic!("test agent has no session identity");
        };
        let identity = OpenCodeConversationIdentity::from_agent_session(identity)
            .expect("test agent is not OpenCode");
        {
            let message = &mut self
                .latest_user_messages
                .get_mut(&identity)
                .expect("test agent has no messages")
                .messages[message];
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
            request.duration_ms = duration_ms;
            request.reasoning_duration_ms = reasoning_duration_ms;
        }
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.latest_user_messages
            .get_mut(&identity)
            .expect("test agent has no messages")
            .revision = self.transcript_revision;
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
        let identity = OpenCodeConversationIdentity::from_agent_session(identity)
            .expect("test agent is not OpenCode");
        self.latest_user_messages
            .get_mut(&identity)
            .expect("test agent has no messages")
            .messages[message]
            .requests[request] = preview;
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.latest_user_messages
            .get_mut(&identity)
            .expect("test agent has no messages")
            .revision = self.transcript_revision;
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

fn load_agent_layouts(path: &Path) -> std::io::Result<HashMap<AgentTimingKey, SavedAgentLayout>> {
    let index: AgentLayoutIndex =
        serde_json::from_slice(&fs::read(path)?).map_err(std::io::Error::other)?;
    if index.version != LAYOUT_INDEX_VERSION {
        return Ok(HashMap::new());
    }
    let layouts = index
        .layouts
        .into_iter()
        .map(|record| {
            (
                record.key,
                SavedAgentLayout {
                    layout: record.layout,
                    last_used_ms: record.last_used_ms,
                },
            )
        })
        .collect();
    Ok(layouts)
}

fn save_agent_layouts(
    path: &Path,
    layouts: &HashMap<AgentTimingKey, SavedAgentLayout>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut records = layouts
        .iter()
        .map(|(key, saved)| AgentLayoutRecord {
            key: key.clone(),
            layout: saved.layout.clone(),
            last_used_ms: saved.last_used_ms,
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

fn prune_agent_layouts<'a>(
    layouts: &mut HashMap<AgentTimingKey, SavedAgentLayout>,
    relevant: impl IntoIterator<Item = &'a AgentTimingKey>,
) {
    if layouts.len() <= MAX_AGENT_LAYOUTS {
        return;
    }
    let relevant = relevant.into_iter().collect::<HashSet<_>>();
    let relevant_count = layouts.keys().filter(|key| relevant.contains(key)).count();
    let target = MAX_AGENT_LAYOUTS.max(relevant_count);
    let mut candidates = layouts
        .iter()
        .filter(|(key, _)| !relevant.contains(key))
        .map(|(key, saved)| (saved.last_used_ms, key.stable_id(), key.clone()))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    for (_, _, key) in candidates.into_iter().take(layouts.len() - target) {
        layouts.remove(&key);
    }
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
mod presentation_interest_tests {
    use super::*;

    fn working_agent(destination: Option<PathBuf>) -> AgentPane {
        AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: "w1:p1".to_owned(),
            terminal_id: None,
            instance_name: None,
            cwd: destination.clone(),
            destination_cwd: destination,
            focused: false,
            runtime: AgentRuntime {
                name: "opencode".to_owned(),
                session_name: None,
                status: AgentStatus::Working,
                timing_key: AgentTimingKey::Pane("opencode@w1:p1".to_owned()),
                session_timing_key: None,
                state_change_seq: 1,
            },
        }
    }

    fn session_without_snapshot() -> HerdrSession {
        let mut session = HerdrSession::new(true, None, None);
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        session
    }

    #[test]
    fn on_idle_prompt_waits_for_the_matching_agent() {
        let mut session = session_without_snapshot();
        let agent = working_agent(None);
        let key = agent.runtime.timing_key.clone();
        session.observed_agents = vec![agent.clone()];
        session.agents = vec![agent];

        assert_eq!(
            session.prompt_agent(0, "later".to_owned(), AgentPromptDelivery::OnIdle),
            Ok(AgentPromptOutcome::Queued)
        );
        assert!(session.agent_prompt_sending(0));
        assert!(session.take_idle_agent_prompts().is_empty());

        session.observed_agents[0].runtime.status = AgentStatus::Idle;
        assert_eq!(
            session.take_idle_agent_prompts(),
            vec![(key, "w1:p1".to_owned(), "later".to_owned())]
        );
        assert!(!session.agent_prompt_sending(0));
    }

    #[test]
    fn on_idle_prompt_is_cancelled_when_agent_departs() {
        let mut session = session_without_snapshot();
        let agent = working_agent(None);
        session.observed_agents = vec![agent.clone()];
        session.agents = vec![agent];
        assert_eq!(
            session.prompt_agent(0, "later".to_owned(), AgentPromptDelivery::OnIdle),
            Ok(AgentPromptOutcome::Queued)
        );

        session.apply_agent_snapshot_at(Vec::new(), unix_time_ms());

        assert!(session.agent_prompts_on_idle.is_empty());
        assert_eq!(
            session.poll(false).notice.as_deref(),
            Some("Queued message cancelled because the agent departed")
        );
    }

    #[test]
    fn on_idle_prompt_uses_the_authoritative_observed_inventory() {
        let mut session = session_without_snapshot();
        let mut agent = working_agent(None);
        agent.runtime.status = AgentStatus::Idle;
        let key = agent.runtime.timing_key.clone();
        session.observed_agents = vec![agent];
        session
            .agent_prompts_on_idle
            .insert(key.clone(), "later".to_owned());

        assert_eq!(
            session.take_idle_agent_prompts(),
            vec![(key, "w1:p1".to_owned(), "later".to_owned())]
        );
    }

    #[test]
    fn idle_status_event_makes_a_queued_prompt_dispatchable_without_a_snapshot() {
        let mut session = session_without_snapshot();
        let agent = working_agent(None);
        let key = agent.runtime.timing_key.clone();
        session.observed_agents = vec![agent.clone()];
        session.agents = vec![agent];
        session
            .agent_prompts_on_idle
            .insert(key.clone(), "later".to_owned());

        session.observe_agent_status_event_at(
            client::AgentStatusEvent {
                workspace_id: "w1".to_owned(),
                pane_id: "w1:p1".to_owned(),
                status: AgentStatus::Idle,
            },
            10,
        );

        assert_eq!(
            session.take_idle_agent_prompts(),
            vec![(key, "w1:p1".to_owned(), "later".to_owned())]
        );
    }

    #[test]
    fn spinner_advances_only_when_an_animation_was_presented() {
        let mut session = session_without_snapshot();
        session.agents = vec![working_agent(None)];
        assert!(!session.poll(false).changed);
        assert_eq!(session.spinner_frame, 0);
        assert!(session.poll(true).changed);
        assert_eq!(session.spinner_frame, 1);
    }
}

#[cfg(test)]
mod latest_user_message_cache_tests {
    use super::*;

    fn identity(value: &str) -> AgentSessionIdentity {
        AgentSessionIdentity {
            source: "herdr:opencode".to_owned(),
            agent: "opencode".to_owned(),
            kind: "id".to_owned(),
            value: value.to_owned(),
        }
    }

    fn conversation_identity(value: &str) -> OpenCodeConversationIdentity {
        OpenCodeConversationIdentity::from_session_id(value)
    }

    fn agent(pane_id: &str, identity: AgentSessionIdentity) -> AgentPane {
        let terminal_id = format!("term-{pane_id}");
        AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: pane_id.to_owned(),
            terminal_id: Some(terminal_id.clone()),
            instance_name: None,
            cwd: None,
            destination_cwd: None,
            focused: false,
            runtime: AgentRuntime {
                name: "opencode".to_owned(),
                session_name: None,
                status: AgentStatus::Idle,
                timing_key: AgentTimingKey::Terminal(format!("opencode@{terminal_id}")),
                session_timing_key: Some(AgentTimingKey::Session(identity)),
                state_change_seq: 1,
            },
        }
    }

    fn message(text: &str) -> Vec<AgentUserMessage> {
        vec![AgentUserMessage {
            text: text.to_owned(),
            requests: Vec::new(),
        }]
    }

    #[test]
    fn authoritative_snapshot_prunes_departed_agent_message_state() {
        let mut session = HerdrSession::new(true, None, None);
        session.cross_workspace_agents = true;
        let retained = identity("ses_retained");
        let departed = identity("ses_departed");
        let retained_agent = agent("w1:p1", retained.clone());
        let departed_agent = agent("w1:p2", departed.clone());
        session
            .apply_agent_snapshot_at(vec![retained_agent.clone(), departed_agent], unix_time_ms());
        for identity in [&retained, &departed] {
            let conversation_identity = conversation_identity(&identity.value);
            session.update_transcript(conversation_identity.clone(), message(&identity.value));
            session.latest_user_message_refreshes.insert(
                conversation_identity.clone(),
                Instant::now() + AGENT_MESSAGE_REFRESH_INTERVAL,
            );
            session
                .latest_user_message_statuses
                .insert(conversation_identity, AgentStatus::Idle);
        }

        session.apply_agent_snapshot_at(vec![retained_agent], unix_time_ms());

        let retained = conversation_identity(&retained.value);

        assert_eq!(
            session.latest_user_messages.keys().collect::<Vec<_>>(),
            vec![&retained]
        );
        assert_eq!(
            session
                .latest_user_message_refreshes
                .keys()
                .collect::<Vec<_>>(),
            vec![&retained]
        );
        assert_eq!(
            session
                .latest_user_message_statuses
                .keys()
                .collect::<Vec<_>>(),
            vec![&retained]
        );
    }

    #[test]
    fn binds_a_scheduled_run_to_its_live_terminal_and_session() {
        let mut session = HerdrSession::new(true, None, None);
        session.scheduler =
            Some(scheduler::SchedulerService::open(None, None, Vec::new()).unwrap());
        let scheduler = session.scheduler.as_mut().unwrap();
        scheduler.tasks.push(scheduler::ScheduledTask {
            id: 7,
            title: "Nightly review".to_owned(),
            description: String::new(),
            prompt: "Review".to_owned(),
            model: String::new(),
            discord_webhook_id: String::new(),
            destination: PathBuf::from("/repo"),
            repository: "repo".to_owned(),
            branch: "main".to_owned(),
            enabled: true,
            interval_minutes: 60,
            next_run_ms: 3_600_000,
            source: None,
            project_status: None,
        });
        scheduler.runs.push(scheduler::ScheduledRun {
            id: 9,
            task_id: 7,
            created_at_ms: 1,
            completed_at_ms: Some(2),
            status: scheduler::ScheduledRunStatus::Completed,
            pane_id: Some("departed:pane".to_owned()),
            terminal_id: None,
            session_id: None,
            error: None,
        });
        let mut remote = agent("w2:p9", identity("ses_remote"));
        remote.workspace_id = "w2".to_owned();
        remote.cwd = Some(PathBuf::from("/repo"));
        remote.instance_name = Some(client::scheduler_run_agent_name(
            "Hunkle: Nightly review #7",
            9,
        ));

        session.apply_agent_snapshot_at(vec![remote], unix_time_ms());

        let run = &session.scheduler.as_ref().unwrap().runs[0];
        assert_eq!(run.pane_id.as_deref(), Some("w2:p9"));
        assert_eq!(run.terminal_id.as_deref(), Some("term-w2:p9"));
        assert_eq!(run.session_id.as_deref(), Some("ses_remote"));
    }

    #[test]
    fn retains_a_scheduled_conversation_without_a_live_agent() {
        let mut session = HerdrSession::new(true, None, None);
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        let identity = scheduled_conversation_identity("ses_scheduled");
        session.scheduled_preview_identity = Some(identity.clone());
        session
            .latest_user_message_requests
            .insert(identity.clone());
        session
            .sender
            .send(Completion::ScheduledConversation {
                identity: identity.clone(),
                result: Ok(latest_message::TranscriptFetch::Changed(message(
                    "durable history",
                ))),
            })
            .unwrap();

        session.poll(false);
        session.apply_agent_snapshot_at(Vec::new(), unix_time_ms());

        assert_eq!(
            session.scheduled_conversation("ses_scheduled").unwrap()[0].text,
            "durable history"
        );
        assert!(session.agents.is_empty());
    }

    #[test]
    fn scheduler_history_is_visible_from_a_live_identity_alias() {
        let mut session = HerdrSession::new(true, None, None);
        session.cross_workspace_agents = true;
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        let live_identity = identity("ses_shared");
        session.apply_agent_snapshot_at(vec![agent("w1:p1", live_identity)], unix_time_ms());
        let scheduled_identity = scheduled_conversation_identity("ses_shared");
        session
            .latest_user_message_requests
            .insert(scheduled_identity.clone());
        session
            .sender
            .send(Completion::ScheduledConversation {
                identity: scheduled_identity,
                result: Ok(latest_message::TranscriptFetch::Changed(message(
                    "shared history",
                ))),
            })
            .unwrap();

        session.poll(false);

        assert_eq!(
            session.agent_user_messages(0).unwrap()[0].text,
            "shared history"
        );
        assert_eq!(
            session.scheduled_conversation("ses_shared").unwrap()[0].text,
            "shared history"
        );
    }

    #[test]
    fn envelope_changes_preserve_in_flight_state_and_departure_prunes_it() {
        let mut session = HerdrSession::new(true, None, None);
        session.cross_workspace_agents = true;
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        let first_identity = identity("ses_stable");
        let replacement_identity = AgentSessionIdentity {
            source: "opencode".to_owned(),
            agent: "opencode".to_owned(),
            kind: "session_id".to_owned(),
            value: "ses_stable".to_owned(),
        };
        let conversation_identity = conversation_identity("ses_stable");
        session.apply_agent_snapshot_at(vec![agent("w1:p1", first_identity)], unix_time_ms());
        session.update_transcript(conversation_identity.clone(), message("cached"));
        session
            .latest_user_message_requests
            .insert(conversation_identity.clone());
        session.latest_user_message_refreshes.insert(
            conversation_identity.clone(),
            Instant::now() + AGENT_MESSAGE_REFRESH_INTERVAL,
        );
        session
            .latest_user_message_statuses
            .insert(conversation_identity.clone(), AgentStatus::Idle);
        session
            .scheduled_conversation_errors
            .insert(conversation_identity.clone(), "old error".to_owned());

        session.apply_agent_snapshot_at(vec![agent("w1:p1", replacement_identity)], unix_time_ms());
        assert!(
            session
                .latest_user_message_requests
                .contains(&conversation_identity)
        );
        session
            .sender
            .send(Completion::LatestUserMessage {
                identity: conversation_identity.clone(),
                result: Ok(latest_message::TranscriptFetch::Changed(message(
                    "completed",
                ))),
            })
            .unwrap();
        session.poll(false);

        assert_eq!(session.agent_user_messages(0).unwrap()[0].text, "completed");
        assert!(
            session
                .latest_user_message_refreshes
                .contains_key(&conversation_identity)
        );
        assert!(
            session
                .latest_user_message_statuses
                .contains_key(&conversation_identity)
        );
        assert!(
            session
                .scheduled_conversation_errors
                .contains_key(&conversation_identity)
        );
        session
            .latest_user_message_requests
            .insert(conversation_identity.clone());

        session.apply_agent_snapshot_at(Vec::new(), unix_time_ms());

        assert!(
            !session
                .latest_user_messages
                .contains_key(&conversation_identity)
        );
        assert!(
            !session
                .latest_user_message_requests
                .contains(&conversation_identity)
        );
        assert!(
            !session
                .latest_user_message_refreshes
                .contains_key(&conversation_identity)
        );
        assert!(
            !session
                .latest_user_message_statuses
                .contains_key(&conversation_identity)
        );
        assert!(
            !session
                .scheduled_conversation_errors
                .contains_key(&conversation_identity)
        );
    }

    #[test]
    fn departed_agent_completion_does_not_restore_message_state() {
        let mut session = HerdrSession::new(true, None, None);
        session.cross_workspace_agents = true;
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        let departed = identity("ses_departed");
        session.apply_agent_snapshot_at(vec![agent("w1:p1", departed.clone())], unix_time_ms());
        let departed_conversation = conversation_identity(&departed.value);
        session
            .latest_user_message_requests
            .insert(departed_conversation.clone());
        session.latest_user_message_refreshes.insert(
            departed_conversation.clone(),
            Instant::now() + AGENT_MESSAGE_REFRESH_INTERVAL,
        );
        session
            .latest_user_message_statuses
            .insert(departed_conversation.clone(), AgentStatus::Idle);

        session.apply_agent_snapshot_at(Vec::new(), unix_time_ms());
        session
            .sender
            .send(Completion::LatestUserMessage {
                identity: departed_conversation.clone(),
                result: Ok(latest_message::TranscriptFetch::Changed(message("stale"))),
            })
            .unwrap();
        session.poll(false);

        assert!(
            !session
                .latest_user_messages
                .contains_key(&departed_conversation)
        );
        assert!(
            !session
                .latest_user_message_requests
                .contains(&departed_conversation)
        );
        assert!(
            !session
                .latest_user_message_refreshes
                .contains_key(&departed_conversation)
        );
        assert!(
            !session
                .latest_user_message_statuses
                .contains_key(&departed_conversation)
        );
    }

    #[test]
    fn unchanged_and_equal_completions_preserve_cached_transcript_without_redraw() {
        let mut session = HerdrSession::new(true, None, None);
        session.next_refresh = Instant::now() + Duration::from_secs(60);
        let identity = conversation_identity("ses_cached");
        session.update_transcript(identity.clone(), message("cached"));
        session.latest_user_message_refreshes.insert(
            identity.clone(),
            Instant::now() + AGENT_MESSAGE_REFRESH_INTERVAL,
        );
        let original = session.latest_user_messages[&identity].messages.as_ptr();
        let original_revision = session.latest_user_messages[&identity].revision;

        session
            .latest_user_message_requests
            .insert(identity.clone());
        session
            .sender
            .send(Completion::LatestUserMessage {
                identity: identity.clone(),
                result: Ok(latest_message::TranscriptFetch::Unchanged),
            })
            .unwrap();
        assert!(!session.poll(false).changed);
        assert_eq!(
            session.latest_user_messages[&identity].messages.as_ptr(),
            original
        );
        assert_eq!(
            session.latest_user_messages[&identity].revision,
            original_revision
        );

        session
            .latest_user_message_requests
            .insert(identity.clone());
        session
            .sender
            .send(Completion::LatestUserMessage {
                identity: identity.clone(),
                result: Ok(latest_message::TranscriptFetch::Changed(message("cached"))),
            })
            .unwrap();
        assert!(!session.poll(false).changed);
        assert_eq!(
            session.latest_user_messages[&identity].messages.as_ptr(),
            original
        );
        assert_eq!(
            session.latest_user_messages[&identity].revision,
            original_revision
        );

        session
            .latest_user_message_requests
            .insert(identity.clone());
        session
            .sender
            .send(Completion::LatestUserMessage {
                identity: identity.clone(),
                result: Ok(latest_message::TranscriptFetch::Changed(message("updated"))),
            })
            .unwrap();
        assert!(session.poll(false).changed);
        assert_eq!(
            session.latest_user_messages[&identity].messages[0].text,
            "updated"
        );
        assert_ne!(
            session.latest_user_messages[&identity].revision,
            original_revision
        );
    }
}

#[cfg(test)]
mod stash_flow_tests {
    use super::*;

    fn unresolved_agent() -> AgentPane {
        AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: "w1:p2".to_owned(),
            terminal_id: Some("term-2".to_owned()),
            instance_name: None,
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
            let poll = session.poll(false);
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

    fn layout(pane_id: &str) -> client::AgentLayout {
        serde_json::from_value(serde_json::json!({
            "root": { "Pane": pane_id }
        }))
        .unwrap()
    }

    fn layout_key(index: usize) -> AgentTimingKey {
        AgentTimingKey::Terminal(format!("terminal-{index:03}"))
    }

    fn saved_layout(index: usize) -> SavedAgentLayout {
        SavedAgentLayout {
            layout: layout(&format!("w1:p{index}")),
            last_used_ms: index as u64,
        }
    }

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
        let layouts = HashMap::from([(
            key.clone(),
            SavedAgentLayout {
                layout: layout.clone(),
                last_used_ms: 42,
            },
        )]);

        save_agent_layouts(&path, &layouts).unwrap();

        let loaded = load_agent_layouts(&path).unwrap();
        assert_eq!(loaded.get(&key).map(|saved| &saved.layout), Some(&layout));
        assert_eq!(loaded.get(&key).map(|saved| saved.last_used_ms), Some(42));
        assert_eq!(
            path,
            directory
                .path()
                .join("agent-layouts")
                .join("77313a7037.json")
        );
    }

    #[test]
    fn first_snapshot_prunes_legacy_layouts_without_discarding_an_active_agent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-layouts.json");
        let layouts = (0..=MAX_AGENT_LAYOUTS)
            .map(|index| {
                serde_json::json!({
                    "key": layout_key(index),
                    "layout": saved_layout(index).layout,
                })
            })
            .collect::<Vec<_>>();
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "version": LAYOUT_INDEX_VERSION,
                "layouts": layouts,
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_agent_layouts(&path).unwrap();
        let active_key = layout_key(0);

        assert_eq!(loaded.len(), MAX_AGENT_LAYOUTS + 1);
        assert!(loaded.contains_key(&active_key));

        let mut session = HerdrSession::new(true, None, None);
        session.agent_layouts = loaded;
        session.apply_agent_snapshot_at(
            vec![AgentPane {
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                pane_id: "w1:p2".to_owned(),
                terminal_id: Some("term-2".to_owned()),
                instance_name: None,
                cwd: None,
                destination_cwd: None,
                focused: false,
                runtime: AgentRuntime {
                    name: "opencode".to_owned(),
                    session_name: None,
                    status: AgentStatus::Idle,
                    timing_key: active_key.clone(),
                    session_timing_key: None,
                    state_change_seq: 1,
                },
            }],
            unix_time_ms(),
        );

        assert_eq!(session.agent_layouts.len(), MAX_AGENT_LAYOUTS);
        assert!(session.agent_layouts.contains_key(&active_key));
        assert!(!session.agent_layouts.contains_key(&layout_key(1)));
        assert!(
            session
                .agent_layouts
                .contains_key(&layout_key(MAX_AGENT_LAYOUTS))
        );
    }

    #[test]
    fn pruning_preserves_active_and_displayed_agent_layouts() {
        let mut session = HerdrSession::new(true, None, None);
        let active_key = layout_key(0);
        let displayed_key = layout_key(1);
        session.agent_layouts = (0..MAX_AGENT_LAYOUTS + 2)
            .map(|index| (layout_key(index), saved_layout(index)))
            .collect();
        session.agents.push(AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t2".to_owned(),
            pane_id: "w1:p2".to_owned(),
            terminal_id: Some("term-2".to_owned()),
            instance_name: None,
            cwd: None,
            destination_cwd: None,
            focused: false,
            runtime: AgentRuntime {
                name: "opencode".to_owned(),
                session_name: None,
                status: AgentStatus::Idle,
                timing_key: active_key.clone(),
                session_timing_key: None,
                state_change_seq: 1,
            },
        });
        session.displayed_agent_key = Some(displayed_key.clone());

        session.prune_agent_layout_history();

        assert_eq!(session.agent_layouts.len(), MAX_AGENT_LAYOUTS);
        assert!(session.agent_layouts.contains_key(&active_key));
        assert!(session.agent_layouts.contains_key(&displayed_key));
        assert!(!session.agent_layouts.contains_key(&layout_key(2)));
        assert!(!session.agent_layouts.contains_key(&layout_key(3)));
    }

    #[test]
    fn showing_the_current_agent_keeps_it_visible() {
        let mut session = HerdrSession::new(true, None, None);
        session.set_host_for_test("w1", "w1:t1", "w1:p1");
        session.agents.push(AgentPane {
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: "w1:p2".to_owned(),
            terminal_id: Some("term-2".to_owned()),
            instance_name: None,
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
