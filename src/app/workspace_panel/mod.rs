pub(super) use std::{
    cmp::Reverse,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub(super) use serde::{Deserialize, Serialize};
#[cfg(test)]
pub(super) use serde_json::Value;

pub(super) use super::{
    HerdrOwnedWorktree, HerdrOwnership, LinkedWorktreeCandidate, LinkedWorktreeObservation,
    TextInput, settings::AgentTimeDisplay,
};
pub(super) use crate::filesystem::atomic_write;

mod focus;
mod herdr;
mod presets;
mod timings;

use focus::{WorkspaceFocusCompletion, WorkspaceFocusState};

mod dialogs;
pub(crate) use dialogs::*;
mod selection;
pub(crate) use selection::*;
mod snapshot;
pub(crate) use snapshot::*;
#[cfg(test)]
mod tests;

pub(super) fn send_command_below(command: String) -> Result<String, String> {
    herdr::send_command_below(command)
}

pub(crate) fn create_managed_worktree(
    cwd: PathBuf,
    path: PathBuf,
    branch: String,
    base: String,
) -> Result<(), String> {
    herdr::perform(herdr::Action::CreateWorktreeAt {
        cwd,
        path,
        branch,
        base,
    })
}

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const TIMING_LAST_SEEN_INTERVAL_MS: u64 = 60_000;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);
pub(crate) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SNAPSHOT_SAVE_ITEM: usize = 1;

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn load_agent_names(path: &Path) -> Result<HashMap<String, String>, String> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(format!("Could not read agent names: {error}")),
    };
    serde_json::from_slice::<AgentNamesFile>(&content)
        .map(|file| file.names)
        .map_err(|error| format!("Could not parse agent names: {error}"))
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
    pub(crate) pane_count: usize,
    pub(crate) focused: bool,
    pub(crate) status: AgentStatus,
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

impl AgentTimingKey {
    fn stable_id(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrAgent {
    pub(crate) name: String,
    pub(crate) session_name: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) focused: bool,
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

#[derive(Debug, Default, Deserialize, Serialize)]
struct AgentNamesFile {
    #[serde(default)]
    names: HashMap<String, String>,
}

pub(crate) struct WorkspacePanel {
    enabled: bool,
    visible: bool,
    pub(crate) workspaces: Vec<HerdrWorkspace>,
    pub(crate) agents: Vec<HerdrAgent>,
    pub(crate) groups: Vec<WorkspaceGroup>,
    pub(crate) selected: Option<usize>,
    pub(crate) workspace_scroll: usize,
    pub(crate) agent_scroll: usize,
    pub(crate) workspace_scroll_follows_selection: bool,
    pub(crate) agent_scroll_follows_selection: bool,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    inventory_verified: bool,
    pub(crate) group_input: TextInput,
    pub(crate) group_editing: bool,
    pub(crate) group_error: Option<String>,
    pub(crate) create_menu_open: bool,
    pub(crate) create_menu_choice: usize,
    pub(crate) snapshot_menu_open: bool,
    pub(crate) snapshot_menu_choice: usize,
    pub(crate) snapshot_input: TextInput,
    pub(crate) snapshot_editing: bool,
    pub(crate) snapshot_error: Option<String>,
    pub(crate) snapshots: Vec<WorkspaceSnapshot>,
    snapshot_loading: bool,
    pub(crate) rename_dialog: Option<WorkspaceRenameDialog>,
    pub(crate) delete_dialog: Option<WorkspaceDeleteDialog>,
    pub(crate) snapshot_load_dialog: Option<SnapshotLoadDialog>,
    preset_store: presets::PresetStore,
    agent_names: HashMap<String, String>,
    agent_names_path: Option<PathBuf>,
    agent_names_writable: bool,
    workspace_drag: Option<WorkspaceDrag>,
    last_click: Option<(SelectionKey, Instant)>,
    focus: WorkspaceFocusState,
    host_tab_id: Option<String>,
    destructive_actions_running: usize,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    next_refresh: Instant,
    spinner_frame: usize,
    next_spinner: Instant,
    agent_timings: HashMap<AgentTimingKey, AgentTiming>,
    agent_timings_path: Option<PathBuf>,
}

pub(crate) struct WorkspacePanelEntryState {
    pub(crate) active: bool,
    pub(crate) loaded: bool,
    pub(crate) selected: bool,
}

#[derive(Default)]
pub(crate) struct WorkspacePanelPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
    pub(crate) reopen_path: Option<PathBuf>,
    pub(crate) workspace_focus_succeeded: bool,
}

impl WorkspacePanel {
    pub(crate) fn detect(groups_path: Option<PathBuf>, snapshots_path: Option<PathBuf>) -> Self {
        #[cfg(test)]
        let environment: Option<herdr::Environment> = None;
        #[cfg(not(test))]
        let environment = herdr::environment();
        let enabled = environment.is_some();
        let mut panel = Self::new(enabled, groups_path, snapshots_path);
        if let Some(environment) = environment {
            panel.focus.set_host(environment.workspace_id);
            panel.host_tab_id = environment.tab_id;
            panel.start_event_listener();
        }
        panel
    }

    fn new(enabled: bool, groups_path: Option<PathBuf>, snapshots_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let agent_timings_path = groups_path
            .as_deref()
            .and_then(Path::parent)
            .map(|path| path.join("agent-timings.json"));
        let agent_names_path = groups_path
            .as_deref()
            .and_then(Path::parent)
            .map(|path| path.join("agent-names.json"));
        let preset_store = presets::PresetStore::new(groups_path, snapshots_path);
        let (mut groups, snapshots, preset_error) = if enabled {
            preset_store.load()
        } else {
            (Vec::new(), Vec::new(), None)
        };
        let (agent_names, agent_names_writable, agent_names_error) = if enabled {
            agent_names_path.as_deref().map_or_else(
                || (HashMap::new(), true, None),
                |path| match load_agent_names(path) {
                    Ok(names) => (names, true, None),
                    Err(error) => (HashMap::new(), false, Some(error)),
                },
            )
        } else {
            (HashMap::new(), true, None)
        };
        let preset_error = match (preset_error, agent_names_error) {
            (Some(preset), Some(names)) => Some(format!("{preset}; {names}")),
            (Some(error), None) | (None, Some(error)) => Some(error),
            (None, None) => None,
        };
        presets::sort_groups(&mut groups);
        Self {
            enabled,
            visible: false,
            workspaces: Vec::new(),
            agents: Vec::new(),
            groups,
            selected: None,
            workspace_scroll: 0,
            agent_scroll: 0,
            workspace_scroll_follows_selection: true,
            agent_scroll_follows_selection: true,
            loading: false,
            error: preset_error,
            inventory_verified: !enabled,
            group_input: TextInput::default(),
            group_editing: false,
            group_error: None,
            create_menu_open: false,
            create_menu_choice: 0,
            snapshot_menu_open: false,
            snapshot_menu_choice: 0,
            snapshot_input: TextInput::default(),
            snapshot_editing: false,
            snapshot_error: None,
            snapshots,
            snapshot_loading: false,
            rename_dialog: None,
            delete_dialog: None,
            snapshot_load_dialog: None,
            preset_store,
            agent_names,
            agent_names_path,
            agent_names_writable,
            workspace_drag: None,
            last_click: None,
            focus: WorkspaceFocusState::default(),
            host_tab_id: None,
            destructive_actions_running: 0,
            sender,
            receiver,
            next_refresh: Instant::now(),
            spinner_frame: 0,
            next_spinner: Instant::now(),
            agent_timings: HashMap::new(),
            agent_timings_path,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn is_available(&self) -> bool {
        self.enabled
    }

    pub(crate) fn linked_worktree_observation(&self) -> LinkedWorktreeObservation {
        let mut candidates = Vec::new();
        for group_index in 0..self.groups.len() {
            let group = self.groups[group_index].name.clone();
            candidates.extend(
                self.sorted_group_workspace_indices(group_index)
                    .into_iter()
                    .filter_map(|index| {
                        self.workspaces[index]
                            .path
                            .clone()
                            .map(|path| LinkedWorktreeCandidate {
                                path,
                                group: Some(group.clone()),
                            })
                    }),
            );
        }
        for (index, workspace) in self.workspaces.iter().enumerate().filter(|(_, workspace)| {
            !workspace.linked_worktree && self.group_for_workspace_id(&workspace.id).is_none()
        }) {
            candidates.extend(
                std::iter::once(index)
                    .chain(self.child_workspace_indices(&workspace.id))
                    .filter_map(|index| {
                        self.workspaces[index]
                            .path
                            .clone()
                            .map(|path| LinkedWorktreeCandidate { path, group: None })
                    }),
            );
        }
        candidates.extend(
            self.workspaces
                .iter()
                .filter(|workspace| {
                    workspace.linked_worktree && workspace.parent_workspace_id.is_none()
                })
                .filter_map(|workspace| {
                    workspace
                        .path
                        .clone()
                        .map(|path| LinkedWorktreeCandidate { path, group: None })
                }),
        );
        let ownership = if !self.enabled {
            HerdrOwnership::Disabled
        } else if !self.inventory_verified {
            HerdrOwnership::Unverified
        } else {
            HerdrOwnership::Verified(
                self.workspaces
                    .iter()
                    .filter(|workspace| workspace.linked_worktree)
                    .filter_map(|workspace| {
                        workspace.path.clone().map(|path| HerdrOwnedWorktree {
                            path,
                            workspace_id: workspace.id.clone(),
                        })
                    })
                    .collect(),
            )
        };
        LinkedWorktreeObservation {
            candidates,
            ownership,
        }
    }

    pub(crate) fn refresh_worktree_inventory(&mut self) {
        if self.enabled && !self.loading {
            self.start_snapshot();
        }
    }

    pub(crate) fn set_visible(&mut self, visible: bool) {
        if visible && !self.visible {
            self.next_refresh = Instant::now();
        }
        self.visible = visible;
    }

    pub(crate) fn entry_count(&self) -> usize {
        self.workspaces.len().saturating_add(self.agents.len())
    }

    pub(crate) fn rows(&self) -> Vec<WorkspacePanelRow> {
        let mut rows = vec![WorkspacePanelRow::Header];
        rows.extend(self.workspace_rows());
        rows.push(WorkspacePanelRow::Spacer);
        rows.push(WorkspacePanelRow::AgentHeader);
        rows.extend(self.agent_rows());
        rows
    }

    pub(crate) fn workspace_rows(&self) -> Vec<WorkspacePanelRow> {
        let mut rows = Vec::new();
        for (group_index, group) in self.groups.iter().enumerate() {
            rows.push(WorkspacePanelRow::Spacer);
            rows.push(WorkspacePanelRow::Group(group_index));
            if group.expanded {
                rows.extend(
                    self.sorted_group_workspace_indices(group_index)
                        .into_iter()
                        .map(WorkspacePanelRow::Workspace),
                );
            }
        }
        for (index, workspace) in self.workspaces.iter().enumerate().filter(|(_, workspace)| {
            !workspace.linked_worktree && self.group_for_workspace_id(&workspace.id).is_none()
        }) {
            rows.push(WorkspacePanelRow::Workspace(index));
            rows.extend(
                self.child_workspace_indices(&workspace.id)
                    .into_iter()
                    .map(WorkspacePanelRow::Workspace),
            );
        }
        rows.extend(
            self.workspaces
                .iter()
                .enumerate()
                .filter(|(_, workspace)| {
                    workspace.linked_worktree && workspace.parent_workspace_id.is_none()
                })
                .map(|(index, _)| WorkspacePanelRow::Workspace(index)),
        );
        rows
    }

    pub(crate) fn agent_rows(&self) -> Vec<WorkspacePanelRow> {
        if self.agents.is_empty() {
            return vec![WorkspacePanelRow::EmptyAgents];
        }

        let mut rows = vec![WorkspacePanelRow::Spacer];
        append_agent_cards(&mut rows, 0..self.agents.len());
        rows
    }

    pub(crate) fn poll(&mut self) -> WorkspacePanelPoll {
        if !self.enabled {
            return WorkspacePanelPoll::default();
        }

        let mut changed = false;
        let mut action_error = None;
        let mut reopen_path = None;
        let mut workspace_focus_succeeded = false;
        while let Ok(completion) = self.receiver.try_recv() {
            changed = true;
            match completion {
                Completion::Snapshot {
                    result,
                    observed_at_ms,
                } => {
                    self.loading = false;
                    if self.snapshot_loading {
                        continue;
                    }
                    match result {
                        Ok((workspaces, agents)) => {
                            let previous = self.selection_key();
                            self.focus.apply_snapshot(&workspaces);
                            self.workspaces = workspaces;
                            self.apply_agent_snapshot_at(agents, observed_at_ms);
                            self.error = None;
                            self.inventory_verified = true;
                            if self.reconcile_group_workspace_ids()
                                && let Err(error) = self.preset_store.save_groups(&self.groups)
                            {
                                action_error = Some(error);
                            }
                            self.restore_selection(previous);
                        }
                        Err(error) => {
                            self.error = Some(error);
                            self.inventory_verified = false;
                        }
                    }
                }
                Completion::HerdrEvent {
                    event,
                    observed_at_ms,
                } => {
                    match event {
                        herdr::Event::Focus(event) => self.apply_focus_event(event),
                        herdr::Event::AgentStatus(event) => {
                            self.apply_agent_status_event_at(event, observed_at_ms);
                        }
                    }
                    self.next_refresh = Instant::now();
                }
                Completion::WorkspaceFocus { request_id, result } => {
                    match self.focus.complete(request_id, result) {
                        WorkspaceFocusCompletion::Ignored => {}
                        WorkspaceFocusCompletion::Succeeded => {
                            self.select_host_workspace();
                            self.next_refresh = Instant::now();
                            workspace_focus_succeeded = true;
                        }
                        WorkspaceFocusCompletion::Failed(error) => {
                            action_error = Some(error);
                        }
                    }
                }
                Completion::Action {
                    result,
                    reopen_path: action_reopen_path,
                    warning,
                    destructive,
                } => {
                    self.next_refresh = Instant::now();
                    if destructive {
                        self.destructive_actions_running =
                            self.destructive_actions_running.saturating_sub(1);
                    }
                    match result {
                        Ok(()) => {
                            self.inventory_verified = false;
                            if action_reopen_path.is_some() {
                                reopen_path = action_reopen_path;
                            }
                            action_error = warning;
                        }
                        Err(error) => action_error = Some(error),
                    }
                }
                Completion::SnapshotRecall { name, result } => match result {
                    Ok(result) => {
                        self.snapshot_loading = false;
                        self.groups = result.groups;
                        action_error = Some(match self.preset_store.save_groups(&self.groups) {
                            Ok(()) => format!("Preset loaded: {name}"),
                            Err(error) => error,
                        });
                        self.next_refresh = Instant::now();
                    }
                    Err(error) => {
                        self.snapshot_loading = false;
                        action_error = Some(error);
                    }
                },
            }
        }

        if self.should_start_snapshot(Instant::now()) {
            self.start_snapshot();
            changed = true;
        }
        changed |= self.poll_spinner(Instant::now());
        WorkspacePanelPoll {
            changed,
            notice: action_error,
            reopen_path,
            workspace_focus_succeeded,
        }
    }

    #[cfg(test)]
    fn apply_agent_snapshot(&mut self, agents: Vec<HerdrAgent>) {
        self.apply_agent_snapshot_at(agents, unix_time_ms());
    }

    fn apply_agent_snapshot_at(&mut self, agents: Vec<HerdrAgent>, now_ms: u64) {
        let synced = self.agent_timings_path.as_deref().is_some_and(|path| {
            timings::sync(path, &mut self.agent_timings, &agents, now_ms).is_ok()
        });
        if !synced {
            timings::update(&mut self.agent_timings, &agents, now_ms);
        }

        let previous = &self.agents;
        let mut ranked = agents.into_iter().enumerate().collect::<Vec<_>>();
        ranked.sort_by_key(|(incoming_index, agent)| {
            let previous_index = previous
                .iter()
                .position(|existing| existing.pane_id == agent.pane_id)
                .unwrap_or(usize::MAX);
            (
                Reverse(agent.state_change_seq),
                previous_index,
                *incoming_index,
            )
        });
        self.agents = ranked.into_iter().map(|(_, agent)| agent).collect();
    }

    pub(crate) fn agent_elapsed(
        &self,
        index: usize,
        display: AgentTimeDisplay,
    ) -> Option<Duration> {
        self.agent_elapsed_for_at(index, display, unix_time_ms())
    }

    pub(crate) fn agent_display_name(&self, index: usize) -> Option<&str> {
        let agent = self.agents.get(index)?;
        self.agent_names
            .get(&agent.timing_key.stable_id())
            .map(String::as_str)
            .or(agent.session_name.as_deref())
    }

    fn save_agent_names(&self) -> Result<(), String> {
        if !self.agent_names_writable {
            return Err(
                "Could not save agent names: repair or remove the malformed agent names file first"
                    .to_owned(),
            );
        }
        let Some(path) = self.agent_names_path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create Hunkle config directory: {error}"))?;
        }
        let content = serde_json::to_vec_pretty(&AgentNamesFile {
            names: self.agent_names.clone(),
        })
        .map_err(|error| format!("Could not serialize agent names: {error}"))?;
        atomic_write(
            path,
            format!("{}\n", String::from_utf8_lossy(&content)).as_bytes(),
        )
        .map_err(|error| format!("Could not save agent names: {error}"))
    }

    pub(crate) fn rename_agent(&mut self, identity: String, label: String) -> Result<(), String> {
        let previous = self.agent_names.insert(identity.clone(), label.clone());
        if label.is_empty() {
            self.agent_names.remove(&identity);
        }
        if let Err(error) = self.save_agent_names() {
            match previous {
                Some(previous) => {
                    self.agent_names.insert(identity, previous);
                }
                None => {
                    self.agent_names.remove(&identity);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    #[cfg(test)]
    fn agent_elapsed_at(&self, index: usize, now_ms: u64) -> Option<Duration> {
        self.agent_elapsed_for_at(index, AgentTimeDisplay::LatestLoop, now_ms)
    }

    fn agent_elapsed_for_at(
        &self,
        index: usize,
        display: AgentTimeDisplay,
        now_ms: u64,
    ) -> Option<Duration> {
        let agent = self.agents.get(index)?;
        self.agent_timings
            .get(&agent.timing_key)
            .map(|timing| timing.elapsed_at(display, now_ms))
    }

    pub(crate) fn clear_agent_timing_history(&mut self) -> Result<(), String> {
        let now_ms = unix_time_ms();
        if let Some(path) = self.agent_timings_path.as_deref() {
            timings::reset(path, &mut self.agent_timings, &self.agents, now_ms)
                .map_err(|error| format!("Could not clear agent timing history: {error}"))
        } else {
            self.agent_timings.clear();
            timings::update(&mut self.agent_timings, &self.agents, now_ms);
            Ok(())
        }
    }

    fn poll_spinner(&mut self, now: Instant) -> bool {
        let working = self.visible
            && (self
                .workspaces
                .iter()
                .any(|workspace| workspace.status == AgentStatus::Working)
                || self
                    .agents
                    .iter()
                    .any(|agent| agent.status == AgentStatus::Working));
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

    fn should_start_snapshot(&self, now: Instant) -> bool {
        self.enabled && !self.snapshot_loading && !self.loading && now >= self.next_refresh
    }

    pub(crate) fn refresh(&mut self) {
        if self.enabled && !self.loading {
            self.next_refresh = Instant::now();
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        if self.rename_dialog.is_some() {
            return self.handle_rename_dialog(key);
        }
        if self.snapshot_load_dialog.is_some() {
            return self.handle_snapshot_load_dialog(key);
        }
        if self.delete_dialog.is_some() {
            return self.handle_delete_dialog(key);
        }
        if self.group_editing {
            return self.handle_group_input(key);
        }
        if self.snapshot_editing {
            return self.handle_snapshot_input(key);
        }
        if self.snapshot_menu_open {
            return self.handle_snapshot_menu(key);
        }
        if self.create_menu_open {
            return self.handle_create_menu(key);
        }
        match key.code {
            KeyCode::Esc => WorkspacePanelEffect::Close,
            KeyCode::Char('w') if key.modifiers.is_empty() => WorkspacePanelEffect::Close,
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                WorkspacePanelEffect::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                WorkspacePanelEffect::None
            }
            KeyCode::Home => {
                self.selected = self.visible_selections().first().copied();
                self.follow_selected_section();
                WorkspacePanelEffect::None
            }
            KeyCode::End => {
                self.selected = self.visible_selections().last().copied();
                self.follow_selected_section();
                WorkspacePanelEffect::None
            }
            KeyCode::Enter => {
                self.focus_selected();
                WorkspacePanelEffect::None
            }
            KeyCode::Char('r') => {
                self.refresh();
                WorkspacePanelEffect::None
            }
            KeyCode::Char('g') if key.modifiers.is_empty() => {
                self.begin_group();
                WorkspacePanelEffect::None
            }
            KeyCode::F(2) if key.modifiers.is_empty() => self.begin_rename(),
            KeyCode::Delete if key.modifiers.is_empty() => self.begin_delete(),
            _ => WorkspacePanelEffect::Unhandled,
        }
    }

    fn begin_rename(&mut self) -> WorkspacePanelEffect {
        let Some(selected) = self.selected else {
            return WorkspacePanelEffect::Notice(
                "Select a workspace or agent to rename".to_owned(),
            );
        };
        let (target, original_label) = if let Some(workspace) = self.workspaces.get(selected) {
            (
                WorkspaceRenameTarget::Workspace {
                    workspace_id: workspace.id.clone(),
                },
                workspace.label.clone(),
            )
        } else {
            let Some(agent_index) = selected.checked_sub(self.workspaces.len()) else {
                return WorkspacePanelEffect::Notice(
                    "Select a workspace or agent to rename".to_owned(),
                );
            };
            let Some(agent) = self.agents.get(agent_index) else {
                return WorkspacePanelEffect::Notice(
                    "Select a workspace or agent to rename".to_owned(),
                );
            };
            let identity = agent.timing_key.stable_id();
            let original_label = self
                .agent_display_name(agent_index)
                .unwrap_or("terminal session")
                .to_owned();
            (WorkspaceRenameTarget::Agent { identity }, original_label)
        };
        let mut input = TextInput::default();
        input.set(original_label.clone());
        input.focus();
        input.select_all();
        self.rename_dialog = Some(WorkspaceRenameDialog {
            target,
            original_label,
            input,
            error: None,
        });
        WorkspacePanelEffect::None
    }

    fn handle_rename_dialog(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        let Some(dialog) = self.rename_dialog.as_mut() else {
            return WorkspacePanelEffect::None;
        };
        dialog.input.focus();
        match key.code {
            KeyCode::Esc => {
                self.rename_dialog = None;
            }
            KeyCode::Enter => return self.submit_rename(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                dialog.input.select_all();
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                dialog.input.delete_word();
                dialog.error = None;
            }
            KeyCode::Left => dialog.input.move_left(),
            KeyCode::Right => dialog.input.move_right(),
            KeyCode::Home => dialog.input.move_home(),
            KeyCode::End => dialog.input.move_end(),
            KeyCode::Delete => {
                dialog.input.delete();
                dialog.error = None;
            }
            KeyCode::Backspace => {
                dialog.input.backspace();
                dialog.error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                dialog.input.insert_char(character);
                dialog.error = None;
            }
            _ => {}
        }
        WorkspacePanelEffect::None
    }

    fn submit_rename(&mut self) -> WorkspacePanelEffect {
        let Some(dialog) = self.rename_dialog.as_mut() else {
            return WorkspacePanelEffect::None;
        };
        let label = dialog.input.text().trim();
        if label.is_empty() && matches!(dialog.target, WorkspaceRenameTarget::Workspace { .. }) {
            dialog.error = Some("Workspace name is required".to_owned());
            return WorkspacePanelEffect::None;
        }
        if label == dialog.original_label {
            self.rename_dialog = None;
            return WorkspacePanelEffect::None;
        }
        let target = dialog.target.clone();
        let label = label.to_owned();
        self.rename_dialog = None;
        match target {
            WorkspaceRenameTarget::Workspace { workspace_id } => {
                WorkspacePanelEffect::RenameWorkspace {
                    workspace_id,
                    label,
                }
            }
            WorkspaceRenameTarget::Agent { identity } => {
                WorkspacePanelEffect::RenameAgent { identity, label }
            }
        }
    }

    fn begin_delete(&mut self) -> WorkspacePanelEffect {
        let Some(workspace) = self
            .selected
            .and_then(|selected| self.workspaces.get(selected))
        else {
            return WorkspacePanelEffect::Notice("Select a workspace to close".to_owned());
        };
        let kind = if workspace.linked_worktree {
            let parent_path = workspace
                .parent_workspace_id
                .as_deref()
                .and_then(|parent_id| {
                    self.workspaces
                        .iter()
                        .find(|candidate| candidate.id == parent_id)
                })
                .and_then(|parent| parent.path.clone())
                .or_else(|| workspace.repo_root.clone());
            WorkspaceDeleteKind::Worktree {
                path: workspace.path.clone(),
                parent_path,
            }
        } else {
            WorkspaceDeleteKind::Workspace {
                pane_count: workspace.pane_count,
            }
        };
        let workspace_id = workspace.id.clone();
        let label = workspace.label.clone();
        self.close_create_menu();
        self.delete_dialog = Some(WorkspaceDeleteDialog {
            workspace_id,
            label,
            kind,
        });
        WorkspacePanelEffect::None
    }

    fn handle_delete_dialog(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.delete_dialog = None;
                WorkspacePanelEffect::None
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                let Some(dialog) = self.delete_dialog.take() else {
                    return WorkspacePanelEffect::None;
                };
                match dialog.kind {
                    WorkspaceDeleteKind::Workspace { .. } => {
                        WorkspacePanelEffect::CloseWorkspace(dialog.workspace_id)
                    }
                    WorkspaceDeleteKind::Worktree { path, parent_path } => {
                        WorkspacePanelEffect::DeleteWorktree {
                            workspace_id: dialog.workspace_id,
                            path,
                            parent_path,
                        }
                    }
                }
            }
            _ => WorkspacePanelEffect::None,
        }
    }

    fn handle_snapshot_load_dialog(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.snapshot_load_dialog = None;
                WorkspacePanelEffect::None
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                let Some(dialog) = self.snapshot_load_dialog.take() else {
                    return WorkspacePanelEffect::None;
                };
                if let Some(entry) = dialog
                    .snapshot
                    .entries
                    .iter()
                    .find(|entry| !entry.path.is_dir())
                {
                    return WorkspacePanelEffect::Notice(format!(
                        "Cannot load preset: '{}' is no longer a directory",
                        entry.path.display()
                    ));
                }
                let name = dialog.snapshot.name.clone();
                self.start_snapshot_recall(dialog.snapshot);
                WorkspacePanelEffect::Notice(format!("Loading preset: {name}"))
            }
            _ => WorkspacePanelEffect::None,
        }
    }

    pub(crate) fn paste(&mut self, text: &str) {
        if let Some(dialog) = self.rename_dialog.as_mut() {
            dialog.input.insert(text);
            dialog.error = None;
        } else if self.snapshot_editing {
            self.snapshot_input.insert(text);
            self.snapshot_error = None;
        } else if self.group_editing {
            self.group_input.insert(text);
            self.group_error = None;
        }
    }

    pub(crate) fn begin_group(&mut self) {
        self.group_input.clear();
        self.group_input.focus();
        self.group_error = None;
        self.group_editing = true;
    }

    pub(crate) fn create_workspace(&self, path: Option<&Path>) {
        self.start_action(herdr::Action::CreateWorkspace {
            path: path.map(Path::to_owned),
        });
    }

    pub(crate) fn create_worktree(&self, workspace_id: &str) {
        self.start_action(herdr::Action::CreateWorktree {
            workspace_id: workspace_id.to_owned(),
        });
    }

    pub(crate) fn close_workspace(&mut self, workspace_id: &str) {
        self.start_destructive_action(
            herdr::Action::CloseWorkspace {
                workspace_id: workspace_id.to_owned(),
            },
            workspace_id,
            None,
        );
    }

    pub(crate) fn rename_workspace(&self, workspace_id: String, label: String) {
        self.start_action(herdr::Action::RenameWorkspace {
            workspace_id,
            label,
        });
    }

    pub(crate) fn delete_worktree(&mut self, workspace_id: &str, reopen_path: Option<PathBuf>) {
        self.start_destructive_action(
            herdr::Action::RemoveWorktree {
                workspace_id: workspace_id.to_owned(),
            },
            workspace_id,
            reopen_path,
        );
    }

    pub(crate) fn toggle_create_menu(&mut self) {
        self.close_snapshot_menu();
        self.create_menu_open = !self.create_menu_open;
        self.create_menu_choice = 0;
    }

    pub(crate) fn close_create_menu(&mut self) {
        self.create_menu_open = false;
        self.create_menu_choice = 0;
    }

    #[cfg(test)]
    pub(crate) fn toggle_snapshot_menu(&mut self) {
        self.close_create_menu();
        self.snapshot_menu_open = !self.snapshot_menu_open;
        self.snapshot_menu_choice = 0;
        self.snapshot_error = None;
    }

    pub(crate) fn open_workspace_presets(&mut self) {
        self.close_create_menu();
        self.close_snapshot_menu();
        self.snapshot_editing = false;
        self.snapshot_error = None;
        self.snapshot_menu_choice = usize::from(!self.snapshots.is_empty());
    }

    pub(crate) fn handle_workspace_presets(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        if self.snapshot_load_dialog.is_some() {
            return self.handle_snapshot_load_dialog(key);
        }
        if self.snapshot_editing {
            let effect = self.handle_snapshot_input(key);
            if !self.snapshot_editing {
                self.snapshot_menu_choice = self
                    .snapshots
                    .iter()
                    .position(|snapshot| {
                        snapshot
                            .name
                            .eq_ignore_ascii_case(self.snapshot_input.text().trim())
                    })
                    .map_or(0, |index| index + SNAPSHOT_SAVE_ITEM);
            }
            return effect;
        }

        let item_count = self.snapshots.len() + SNAPSHOT_SAVE_ITEM;
        match key.code {
            KeyCode::Esc => WorkspacePanelEffect::Close,
            KeyCode::Char('q') if key.modifiers.is_empty() => WorkspacePanelEffect::Close,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.snapshot_menu_choice = self
                    .snapshot_menu_choice
                    .checked_sub(1)
                    .unwrap_or(item_count - 1);
                WorkspacePanelEffect::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.snapshot_menu_choice = (self.snapshot_menu_choice + 1) % item_count;
                WorkspacePanelEffect::None
            }
            KeyCode::Char('n') if key.modifiers.is_empty() => self.activate_snapshot_choice(0),
            KeyCode::Char('u') if key.modifiers.is_empty() && self.snapshot_menu_choice > 0 => {
                let index = self.snapshot_menu_choice - SNAPSHOT_SAVE_ITEM;
                let Some(name) = self
                    .snapshots
                    .get(index)
                    .map(|snapshot| snapshot.name.clone())
                else {
                    return WorkspacePanelEffect::None;
                };
                self.snapshot_input.set(name);
                self.save_snapshot()
            }
            KeyCode::Delete if self.snapshot_menu_choice > 0 => {
                self.delete_snapshot_choice();
                WorkspacePanelEffect::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate_snapshot_choice(self.snapshot_menu_choice)
            }
            _ => WorkspacePanelEffect::None,
        }
    }

    pub(crate) fn select_snapshot_choice(&mut self, choice: usize) {
        self.snapshot_menu_choice = choice.min(self.snapshots.len());
    }

    pub(crate) fn close_snapshot_menu(&mut self) {
        self.snapshot_menu_open = false;
        self.snapshot_menu_choice = 0;
        self.snapshot_error = None;
    }

    pub(crate) fn activate_snapshot_choice(&mut self, choice: usize) -> WorkspacePanelEffect {
        if choice == 0 {
            self.snapshot_menu_open = false;
            self.snapshot_input.clear();
            self.snapshot_input.focus();
            self.snapshot_editing = true;
            self.snapshot_error = None;
            return WorkspacePanelEffect::None;
        }
        if self.snapshot_loading {
            self.close_snapshot_menu();
            return WorkspacePanelEffect::Notice("A preset is already loading".to_owned());
        }
        let index = choice - SNAPSHOT_SAVE_ITEM;
        let Some(mut snapshot) = self.snapshots.get(index).cloned() else {
            return WorkspacePanelEffect::None;
        };
        if !snapshot.groups_captured {
            snapshot.capture_groups(&self.groups, &self.workspaces);
            self.snapshots[index] = snapshot.clone();
            if let Err(error) = self.preset_store.save_snapshots(&self.snapshots) {
                return WorkspacePanelEffect::Notice(error);
            }
        }
        self.close_snapshot_menu();
        let plan = presets::recall_plan(&snapshot, &self.workspaces);
        self.snapshot_load_dialog = Some(SnapshotLoadDialog {
            name: snapshot.name.clone(),
            open_count: plan.open_count,
            close_count: plan.close_count,
            close_pane_count: plan.close_pane_count,
            group_count: snapshot.groups.len(),
            snapshot,
        });
        WorkspacePanelEffect::None
    }

    pub(crate) fn selected_workspace_id(&self) -> Option<&str> {
        self.selected
            .and_then(|selected| self.workspaces.get(selected))
            .map(|workspace| workspace.id.as_str())
    }

    pub(crate) fn activate_create_choice(&mut self, choice: usize) -> WorkspacePanelEffect {
        let effect = match choice {
            0 => WorkspacePanelEffect::CreateWorkspace,
            1 => self
                .selected_workspace_id()
                .map(|id| WorkspacePanelEffect::CreateWorktree(id.to_owned()))
                .unwrap_or(WorkspacePanelEffect::None),
            _ => WorkspacePanelEffect::None,
        };
        if effect != WorkspacePanelEffect::None {
            self.close_create_menu();
        }
        effect
    }

    fn handle_create_menu(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        match key.code {
            KeyCode::Esc => {
                self.close_create_menu();
                WorkspacePanelEffect::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                if self.selected_workspace_id().is_some() {
                    self.create_menu_choice = 1;
                }
                WorkspacePanelEffect::None
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.create_menu_choice = 0;
                WorkspacePanelEffect::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate_create_choice(self.create_menu_choice)
            }
            _ => WorkspacePanelEffect::None,
        }
    }

    fn handle_snapshot_menu(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        let item_count = self.snapshots.len() + SNAPSHOT_SAVE_ITEM;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_snapshot_menu(),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.snapshot_menu_choice = self
                    .snapshot_menu_choice
                    .checked_sub(1)
                    .unwrap_or(item_count - 1);
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.snapshot_menu_choice = (self.snapshot_menu_choice + 1) % item_count;
            }
            KeyCode::Delete if self.snapshot_menu_choice > 0 => {
                self.delete_snapshot_choice();
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                return self.activate_snapshot_choice(self.snapshot_menu_choice);
            }
            _ => {}
        }
        WorkspacePanelEffect::None
    }

    fn delete_snapshot_choice(&mut self) {
        let index = self.snapshot_menu_choice - SNAPSHOT_SAVE_ITEM;
        let removed = self.snapshots.remove(index);
        let name = removed.name.clone();
        self.snapshot_menu_choice = self.snapshot_menu_choice.min(self.snapshots.len());
        self.snapshot_error = Some(
            if let Err(error) = self.preset_store.save_snapshots(&self.snapshots) {
                self.snapshots.insert(index, removed);
                error
            } else {
                format!("Deleted preset: {name}")
            },
        );
    }

    fn handle_snapshot_input(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        self.snapshot_input.focus();
        match key.code {
            KeyCode::Esc => {
                self.snapshot_editing = false;
                self.snapshot_error = None;
            }
            KeyCode::Enter => return self.save_snapshot(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.snapshot_input.select_all();
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.snapshot_input.delete_word();
                self.snapshot_error = None;
            }
            KeyCode::Left => self.snapshot_input.move_left(),
            KeyCode::Right => self.snapshot_input.move_right(),
            KeyCode::Home => self.snapshot_input.move_home(),
            KeyCode::End => self.snapshot_input.move_end(),
            KeyCode::Delete => self.snapshot_input.delete(),
            KeyCode::Backspace => self.snapshot_input.backspace(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.snapshot_input.insert_char(character);
                self.snapshot_error = None;
            }
            _ => {}
        }
        WorkspacePanelEffect::None
    }

    fn save_snapshot(&mut self) -> WorkspacePanelEffect {
        let name = self.snapshot_input.text().trim();
        if name.is_empty() {
            self.snapshot_error = Some("Preset name is required".to_owned());
            return WorkspacePanelEffect::None;
        }
        let existing = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.name.eq_ignore_ascii_case(name));
        let entries = match self.snapshot_entries() {
            Ok(entries) => entries,
            Err(error) => {
                self.snapshot_error = Some(error);
                return WorkspacePanelEffect::None;
            }
        };
        let name = name.to_owned();
        let snapshot = WorkspaceSnapshot {
            name: name.clone(),
            entries,
            groups: self
                .groups
                .iter()
                .map(|group| WorkspaceSnapshotGroup {
                    name: group.name.clone(),
                    expanded: group.expanded,
                })
                .collect(),
            groups_captured: true,
        };
        let previous = if let Some(index) = existing {
            Some(std::mem::replace(&mut self.snapshots[index], snapshot))
        } else {
            self.snapshots.push(snapshot);
            None
        };
        self.snapshots
            .sort_by_cached_key(|snapshot| snapshot.name.to_lowercase());
        if let Err(error) = self.preset_store.save_snapshots(&self.snapshots) {
            self.snapshots
                .retain(|snapshot| !snapshot.name.eq_ignore_ascii_case(&name));
            if let Some(previous) = previous {
                self.snapshots.push(previous);
                self.snapshots
                    .sort_by_cached_key(|snapshot| snapshot.name.to_lowercase());
            }
            self.snapshot_error = Some(error);
            return WorkspacePanelEffect::None;
        }
        self.snapshot_editing = false;
        self.snapshot_error = None;
        let action = if existing.is_some() {
            "updated"
        } else {
            "saved"
        };
        WorkspacePanelEffect::Notice(format!("Preset {action}: {name}"))
    }

    fn snapshot_entries(&self) -> Result<Vec<WorkspaceSnapshotEntry>, String> {
        if self.workspaces.is_empty() {
            return Err("There are no workspaces to save in a preset".to_owned());
        }
        self.workspaces
            .iter()
            .map(|workspace| {
                let path = workspace
                    .path
                    .clone()
                    .ok_or_else(|| format!("Workspace '{}' has no directory", workspace.label))?;
                Ok(WorkspaceSnapshotEntry {
                    label: workspace.label.clone(),
                    path,
                    focused: workspace.focused,
                    linked_worktree: workspace.linked_worktree,
                    group: self
                        .group_for_workspace_id(
                            workspace
                                .parent_workspace_id
                                .as_deref()
                                .unwrap_or(&workspace.id),
                        )
                        .map(|index| self.groups[index].name.clone()),
                })
            })
            .collect()
    }

    fn handle_group_input(&mut self, key: KeyEvent) -> WorkspacePanelEffect {
        self.group_input.focus();
        match key.code {
            KeyCode::Esc => {
                self.group_editing = false;
                self.group_error = None;
            }
            KeyCode::Enter => return self.submit_group(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.group_input.select_all();
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.group_input.delete_word();
                self.group_error = None;
            }
            KeyCode::Left => self.group_input.move_left(),
            KeyCode::Right => self.group_input.move_right(),
            KeyCode::Home => self.group_input.move_home(),
            KeyCode::End => self.group_input.move_end(),
            KeyCode::Delete => self.group_input.delete(),
            KeyCode::Backspace => self.group_input.backspace(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.group_input.insert_char(character);
                self.group_error = None;
            }
            _ => {}
        }
        WorkspacePanelEffect::None
    }

    fn submit_group(&mut self) -> WorkspacePanelEffect {
        let name = self.group_input.text().trim();
        if name.is_empty() {
            self.group_error = Some("Group name is required".to_owned());
            return WorkspacePanelEffect::None;
        }
        if self
            .groups
            .iter()
            .any(|group| group.name.eq_ignore_ascii_case(name))
        {
            self.group_error = Some("Group already exists".to_owned());
            return WorkspacePanelEffect::None;
        }
        self.groups.push(WorkspaceGroup {
            name: name.to_owned(),
            expanded: true,
            workspace_ids: Vec::new(),
        });
        presets::sort_groups(&mut self.groups);
        self.group_editing = false;
        self.group_error = None;
        match self.preset_store.save_groups(&self.groups) {
            Ok(()) => WorkspacePanelEffect::None,
            Err(error) => WorkspacePanelEffect::Notice(error),
        }
    }

    pub(crate) fn select_workspace(&mut self, index: usize) -> bool {
        if index >= self.workspaces.len() {
            return false;
        }
        self.selected = Some(index);
        self.workspace_scroll_follows_selection = true;
        true
    }

    pub(crate) fn select_agent(&mut self, index: usize) -> bool {
        if index >= self.agents.len() {
            return false;
        }
        self.selected = Some(self.workspaces.len().saturating_add(index));
        self.agent_scroll_follows_selection = true;
        true
    }

    pub(crate) fn click_workspace(&mut self, index: usize) -> WorkspacePanelEffect {
        if index >= self.workspaces.len() {
            return WorkspacePanelEffect::None;
        }
        if self.register_workspace_click(index) {
            self.focus_selected();
            WorkspacePanelEffect::None
        } else {
            self.workspaces[index].path.clone().map_or(
                WorkspacePanelEffect::None,
                WorkspacePanelEffect::OpenWorkspace,
            )
        }
    }

    fn register_workspace_click(&mut self, index: usize) -> bool {
        if !self.select_workspace(index) {
            return false;
        }
        let key = SelectionKey::Workspace(self.workspaces[index].id.clone());
        if self.is_double_click(&key) {
            self.last_click = None;
            return true;
        }
        let now = Instant::now();
        self.last_click = Some((key, now));
        false
    }

    pub(crate) fn click_agent(&mut self, index: usize) -> WorkspacePanelEffect {
        if !self.select_agent(index) {
            return WorkspacePanelEffect::None;
        }
        self.last_click = None;
        let pane_id = self.agents[index].pane_id.clone();
        self.observe_agent_focus(&pane_id);
        WorkspacePanelEffect::FocusAgent(pane_id)
    }

    fn is_double_click(&self, key: &SelectionKey) -> bool {
        self.last_click
            .as_ref()
            .is_some_and(|(previous, at)| previous == key && at.elapsed() <= DOUBLE_CLICK_INTERVAL)
    }

    pub(crate) fn toggle_group(&mut self, index: usize) {
        let Some(group) = self.groups.get_mut(index) else {
            return;
        };
        group.expanded = !group.expanded;
        self.ensure_visible_selection();
        let _ = self.preset_store.save_groups(&self.groups);
    }

    pub(crate) fn group_for_workspace(&self, index: usize) -> Option<usize> {
        let workspace = self.workspaces.get(index)?;
        let workspace_id = workspace
            .parent_workspace_id
            .as_deref()
            .unwrap_or(&workspace.id);
        self.group_for_workspace_id(workspace_id)
    }

    #[cfg(test)]
    pub(crate) fn workspace_indent(&self, index: usize) -> &'static str {
        let Some(workspace) = self.workspaces.get(index) else {
            return "";
        };
        match (
            self.group_for_workspace(index).is_some(),
            workspace.linked_worktree,
        ) {
            (true, true) => "  ",
            (true, false) | (false, true) => " ",
            (false, false) => "",
        }
    }

    pub(crate) fn workspace_is_linked_worktree(&self, index: usize) -> bool {
        self.workspaces
            .get(index)
            .is_some_and(|workspace| workspace.linked_worktree)
    }

    pub(crate) fn workspace_entry_state(
        &self,
        index: usize,
        panel_focused: bool,
        loaded_workspace_path: Option<&Path>,
    ) -> WorkspacePanelEntryState {
        WorkspacePanelEntryState {
            active: self.workspaces.get(index).is_some_and(|workspace| {
                self.focus.active_workspace_id() == Some(workspace.id.as_str())
            }),
            loaded: self.workspaces.get(index).is_some_and(|workspace| {
                workspace
                    .path
                    .as_deref()
                    .zip(loaded_workspace_path)
                    .is_some_and(|(workspace, loaded)| {
                        crate::filesystem::same_path(workspace, loaded)
                    })
            }),
            selected: panel_focused && self.selected == Some(index),
        }
    }

    pub(crate) fn agent_entry_state(
        &self,
        index: usize,
        panel_focused: bool,
    ) -> WorkspacePanelEntryState {
        let active = self.agents.get(index).is_some_and(|agent| agent.focused);
        WorkspacePanelEntryState {
            active,
            loaded: false,
            selected: self.highlighted_agent_index(panel_focused) == Some(index),
        }
    }

    pub(crate) fn agent_is_in_host_tab(&self, index: usize) -> bool {
        self.agents.get(index).is_some_and(|agent| {
            self.focus.host() == Some(agent.workspace_id.as_str())
                && self.host_tab_id.as_deref() == Some(agent.tab_id.as_str())
        })
    }

    pub(crate) fn highlighted_agent_index(&self, panel_focused: bool) -> Option<usize> {
        self.agents
            .iter()
            .position(|agent| agent.focused)
            .or_else(|| {
                if !panel_focused {
                    return None;
                }
                self.selected?
                    .checked_sub(self.workspaces.len())
                    .filter(|index| *index < self.agents.len())
            })
    }

    #[cfg(test)]
    fn workspace_is_active(&self, index: usize) -> bool {
        self.workspace_entry_state(index, false, None).active
    }

    fn group_for_workspace_id(&self, id: &str) -> Option<usize> {
        self.groups.iter().position(|group| {
            group
                .workspace_ids
                .iter()
                .any(|workspace_id| workspace_id == id)
        })
    }

    pub(crate) fn begin_workspace_drag(&mut self, workspace: usize) -> bool {
        if self
            .workspaces
            .get(workspace)
            .is_none_or(|workspace| workspace.linked_worktree)
        {
            return false;
        }
        self.workspace_drag = Some(WorkspaceDrag {
            workspace,
            active: false,
            target: None,
        });
        true
    }

    pub(crate) fn update_workspace_drag(&mut self, target: Option<WorkspaceDropTarget>) {
        if let Some(drag) = self.workspace_drag.as_mut() {
            drag.active = true;
            drag.target = target;
        }
    }

    pub(crate) fn finish_workspace_drag(&mut self) -> WorkspacePanelEffect {
        let Some(drag) = self.workspace_drag.take() else {
            return WorkspacePanelEffect::None;
        };
        if !drag.active {
            return self.click_workspace(drag.workspace);
        }
        let Some(target) = drag.target else {
            return WorkspacePanelEffect::None;
        };
        let Some(workspace_id) = self
            .workspaces
            .get(drag.workspace)
            .filter(|workspace| !workspace.linked_worktree)
            .map(|workspace| workspace.id.clone())
        else {
            return WorkspacePanelEffect::None;
        };
        for group in &mut self.groups {
            group.workspace_ids.retain(|id| id != &workspace_id);
        }
        if let WorkspaceDropTarget::Group(index) = target {
            let Some(group) = self.groups.get_mut(index) else {
                return WorkspacePanelEffect::None;
            };
            group.workspace_ids.push(workspace_id);
            group.expanded = true;
        }
        self.ensure_visible_selection();
        match self.preset_store.save_groups(&self.groups) {
            Ok(()) => WorkspacePanelEffect::None,
            Err(error) => WorkspacePanelEffect::Notice(error),
        }
    }

    pub(crate) fn workspace_drag_target(&self) -> Option<WorkspaceDropTarget> {
        self.workspace_drag.and_then(|drag| drag.target)
    }

    pub(crate) fn is_dragging_workspace(&self) -> bool {
        self.workspace_drag.is_some()
    }

    fn child_workspace_indices(&self, parent_id: &str) -> Vec<usize> {
        let mut indices = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| workspace.parent_workspace_id.as_deref() == Some(parent_id))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        indices.sort_by_cached_key(|index| self.workspaces[*index].label.to_lowercase());
        indices
    }

    fn sorted_group_workspace_indices(&self, group_index: usize) -> Vec<usize> {
        let Some(group) = self.groups.get(group_index) else {
            return Vec::new();
        };
        let mut parents = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| {
                !workspace.linked_worktree && group.workspace_ids.contains(&workspace.id)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        parents.sort_by_cached_key(|index| self.workspaces[*index].label.to_lowercase());

        let mut indices = Vec::new();
        for parent in parents {
            indices.push(parent);
            indices.extend(self.child_workspace_indices(&self.workspaces[parent].id));
        }
        indices
    }

    fn reconcile_group_workspace_ids(&mut self) -> bool {
        let valid_workspace_ids = self
            .workspaces
            .iter()
            .filter(|workspace| !workspace.linked_worktree)
            .map(|workspace| workspace.id.as_str())
            .collect::<Vec<_>>();
        let mut changed = false;
        for group in &mut self.groups {
            let previous_len = group.workspace_ids.len();
            group
                .workspace_ids
                .retain(|id| valid_workspace_ids.contains(&id.as_str()));
            changed |= group.workspace_ids.len() != previous_len;
        }
        changed
    }

    fn focus_selected(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        if let Some(workspace_id) = self
            .workspaces
            .get(selected)
            .map(|workspace| workspace.id.clone())
        {
            self.start_workspace_focus(workspace_id);
            return;
        }
        let agent_index = selected.saturating_sub(self.workspaces.len());
        let Some(pane_id) = self
            .agents
            .get(agent_index)
            .map(|agent| agent.pane_id.clone())
        else {
            return;
        };
        self.focus_agent(pane_id);
    }

    pub(crate) fn start_workspace_focus(&mut self, workspace_id: String) {
        let request_id = self.focus.begin(workspace_id.clone());
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = herdr::perform(herdr::Action::FocusWorkspace { workspace_id });
            let _ = sender.send(Completion::WorkspaceFocus { request_id, result });
        });
    }

    fn start_event_listener(&self) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            loop {
                if herdr::watch_events(|event| {
                    sender
                        .send(Completion::HerdrEvent {
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

    fn apply_focus_event(&mut self, event: herdr::FocusEvent) {
        let herdr::FocusEvent {
            workspace_id,
            pane_id,
        } = event;
        self.focus.observe(Some(workspace_id.clone()));
        for workspace in &mut self.workspaces {
            workspace.focused = workspace.id == workspace_id;
        }
        for agent in &mut self.agents {
            agent.focused = agent.pane_id == pane_id;
        }
    }

    fn apply_agent_status_event_at(&mut self, event: herdr::AgentStatusEvent, now_ms: u64) {
        let Some(agent) = self.agents.iter_mut().find(|agent| {
            agent.workspace_id == event.workspace_id && agent.pane_id == event.pane_id
        }) else {
            return;
        };
        agent.status = event.status;
        let key = agent.timing_key.clone();
        let state_change_seq = agent.state_change_seq;
        let synced = self.agent_timings_path.as_deref().is_some_and(|path| {
            timings::observe_status(
                path,
                &mut self.agent_timings,
                &key,
                event.status,
                state_change_seq,
                now_ms,
            )
            .is_ok()
        });
        if !synced {
            if let Some(timing) = self.agent_timings.get_mut(&key) {
                timing.observe_event(event.status, now_ms);
            } else if event.status.should_track_timing() {
                let mut timing = AgentTiming::new(event.status, state_change_seq, now_ms);
                timing.awaiting_sequence = true;
                self.agent_timings.insert(key, timing);
            }
        }
    }

    pub(crate) fn focus_agent(&mut self, pane_id: String) {
        self.observe_agent_focus(&pane_id);
        self.start_action(herdr::Action::FocusAgent { pane_id });
    }

    fn observe_agent_focus(&mut self, pane_id: &str) {
        if let Some(workspace_id) = self
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
            .map(|agent| agent.workspace_id.clone())
        {
            self.apply_focus_event(herdr::FocusEvent {
                workspace_id,
                pane_id: pane_id.to_owned(),
            });
        }
    }

    fn start_action(&self, action: herdr::Action) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = herdr::perform(action);
            let _ = sender.send(Completion::Action {
                result,
                reopen_path: None,
                warning: None,
                destructive: false,
            });
        });
    }

    fn start_destructive_action(
        &mut self,
        action: herdr::Action,
        removed_workspace_id: &str,
        reopen_path: Option<PathBuf>,
    ) {
        self.destructive_actions_running = self.destructive_actions_running.saturating_add(1);
        let restore_focus = self.focus_to_restore_after_removing(removed_workspace_id);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = herdr::perform(action);
            let warning = if result.is_ok() {
                restore_focus.and_then(|workspace_id| {
                    herdr::perform(herdr::Action::FocusWorkspace { workspace_id })
                        .err()
                        .map(|error| {
                            format!(
                                "Workspace closed, but Herdr focus could not be restored: {error}"
                            )
                        })
                })
            } else {
                None
            };
            let _ = sender.send(Completion::Action {
                result,
                reopen_path,
                warning,
                destructive: true,
            });
        });
    }

    pub(crate) fn destructive_action_running(&self) -> bool {
        self.destructive_actions_running > 0
    }

    fn focus_to_restore_after_removing(&self, removed_workspace_id: &str) -> Option<String> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.focused && workspace.id != removed_workspace_id)
            .map(|workspace| workspace.id.clone())
    }

    fn start_snapshot_recall(&mut self, snapshot: WorkspaceSnapshot) {
        self.snapshot_loading = true;
        let sender = self.sender.clone();
        let current = self.workspaces.clone();
        thread::spawn(move || {
            let name = snapshot.name.clone();
            let result = recall_snapshot(&snapshot, &current);
            let _ = sender.send(Completion::SnapshotRecall { name, result });
        });
    }

    fn move_selection(&mut self, delta: isize) {
        let selections = self.visible_selections();
        if selections.is_empty() {
            self.selected = None;
            return;
        }
        self.move_selection_within(&selections, delta);
        self.follow_selected_section();
    }

    pub(crate) fn scroll_workspace(&mut self, delta: isize) {
        self.workspace_scroll_follows_selection = false;
        self.workspace_scroll = self.workspace_scroll.saturating_add_signed(delta);
    }

    pub(crate) fn scroll_agents(&mut self, delta: isize) {
        self.agent_scroll_follows_selection = false;
        self.agent_scroll = self.agent_scroll.saturating_add_signed(delta);
    }

    fn follow_selected_section(&mut self) {
        match self.selected {
            Some(selected) if selected < self.workspaces.len() => {
                self.workspace_scroll_follows_selection = true;
            }
            Some(_) => {
                self.agent_scroll_follows_selection = true;
            }
            None => {}
        }
    }

    fn move_selection_within(&mut self, selections: &[usize], delta: isize) {
        if selections.is_empty() {
            return;
        }
        let Some(current) = self
            .selected
            .and_then(|selected| selections.iter().position(|entry| *entry == selected))
        else {
            self.selected = if delta < 0 {
                selections.last().copied()
            } else {
                selections.first().copied()
            };
            return;
        };
        self.selected = selections
            .get(
                current
                    .saturating_add_signed(delta)
                    .min(selections.len() - 1),
            )
            .copied();
    }

    fn visible_selections(&self) -> Vec<usize> {
        self.rows()
            .into_iter()
            .filter_map(|row| match row {
                WorkspacePanelRow::Workspace(index) => Some(index),
                WorkspacePanelRow::Agent(index) => {
                    Some(self.workspaces.len().saturating_add(index))
                }
                _ => None,
            })
            .collect()
    }

    fn ensure_visible_selection(&mut self) {
        let selections = self.visible_selections();
        if !self
            .selected
            .is_some_and(|selected| selections.contains(&selected))
        {
            self.selected = selections.first().copied();
        }
    }

    fn start_snapshot(&mut self) {
        self.loading = true;
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let snapshot = herdr::session_snapshot();
            let observed_at_ms = unix_time_ms();
            let result = snapshot.map(|(mut workspaces, agents)| {
                populate_workspace_branches(&mut workspaces);
                (workspaces, agents)
            });
            let _ = sender.send(Completion::Snapshot {
                result,
                observed_at_ms,
            });
        });
    }

    fn selection_key(&self) -> Option<SelectionKey> {
        let selected = self.selected?;
        self.workspaces
            .get(selected)
            .map(|workspace| SelectionKey::Workspace(workspace.id.clone()))
            .or_else(|| {
                self.agents
                    .get(selected.saturating_sub(self.workspaces.len()))
                    .map(|agent| SelectionKey::Agent(agent.pane_id.clone()))
            })
    }

    fn restore_selection(&mut self, previous: Option<SelectionKey>) {
        self.selected = previous
            .and_then(|key| match key {
                SelectionKey::Workspace(id) => self
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.id == id),
                SelectionKey::Agent(id) => self
                    .agents
                    .iter()
                    .position(|agent| agent.pane_id == id)
                    .map(|index| self.workspaces.len().saturating_add(index)),
            })
            .or_else(|| {
                let host = self.focus.host()?;
                self.workspaces
                    .iter()
                    .position(|workspace| workspace.id == host)
            })
            .or_else(|| {
                self.workspaces
                    .iter()
                    .position(|workspace| workspace.focused)
            })
            .or_else(|| (self.entry_count() > 0).then_some(0));
        self.ensure_visible_selection();
        self.workspace_scroll = self
            .workspace_scroll
            .min(self.workspace_rows().len().saturating_sub(1));
        self.agent_scroll = self
            .agent_scroll
            .min(self.agent_rows().len().saturating_sub(1));
    }

    fn select_host_workspace(&mut self) {
        let Some(host) = self.focus.host() else {
            return;
        };
        if let Some(index) = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == host)
        {
            self.selected = Some(index);
        }
    }

    #[cfg(test)]
    pub(crate) fn selected_visual_row(&self) -> Option<usize> {
        let selected = self.selected?;
        self.rows().iter().position(|row| match row {
            WorkspacePanelRow::Workspace(index) => *index == selected,
            WorkspacePanelRow::Agent(index) => {
                self.workspaces.len().saturating_add(*index) == selected
            }
            _ => false,
        })
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test(value: &Value) -> Self {
        let mut panel = Self::new(true, None, None);
        let (workspaces, agents) = herdr::parse_snapshot(value).unwrap();
        panel.focus.apply_snapshot(&workspaces);
        panel.workspaces = workspaces;
        panel.inventory_verified = true;
        panel.apply_agent_snapshot(agents);
        panel.restore_selection(None);
        panel
    }

    #[cfg(test)]
    pub(crate) fn set_host_location_for_test(&mut self, workspace_id: &str, tab_id: &str) {
        self.focus.set_host(Some(workspace_id.to_owned()));
        self.host_tab_id = Some(tab_id.to_owned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspacePanelEffect {
    None,
    Unhandled,
    Close,
    CreateWorkspace,
    CreateWorktree(String),
    RenameWorkspace {
        workspace_id: String,
        label: String,
    },
    RenameAgent {
        identity: String,
        label: String,
    },
    CloseWorkspace(String),
    DeleteWorktree {
        workspace_id: String,
        path: Option<PathBuf>,
        parent_path: Option<PathBuf>,
    },
    FocusAgent(String),
    OpenWorkspace(PathBuf),
    Notice(String),
}
