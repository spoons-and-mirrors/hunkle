use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
#[cfg(test)]
use serde_json::Value;

use super::TextInput;

mod herdr;
mod presets;

pub(super) fn send_command_below(command: String) -> Result<String, String> {
    herdr::send_command_below(command)
}

pub(crate) const DEFAULT_WIDTH: u16 = 26;
pub(crate) const MINIMUM_WIDTH: u16 = 18;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);
pub(crate) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SNAPSHOT_SAVE_ITEM: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspacePanelPlacement {
    Off,
    Left,
    Right,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrAgent {
    pub(crate) name: String,
    pub(crate) session_name: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) focused: bool,
    pub(crate) status: AgentStatus,
    state_change_seq: u64,
}

struct AgentTiming {
    elapsed: Duration,
    running_since: Option<Instant>,
    status: AgentStatus,
    state_change_seq: u64,
}

impl AgentTiming {
    fn new(status: AgentStatus, state_change_seq: u64, now: Instant) -> Self {
        Self {
            elapsed: Duration::ZERO,
            running_since: (status == AgentStatus::Working).then_some(now),
            status,
            state_change_seq,
        }
    }

    fn observe(&mut self, status: AgentStatus, state_change_seq: u64, now: Instant) {
        let sequence_changed = self.state_change_seq != 0
            && state_change_seq != 0
            && self.state_change_seq != state_change_seq;
        match status {
            AgentStatus::Working => match self.status {
                AgentStatus::Working if sequence_changed => self.restart(now),
                AgentStatus::Working => {}
                AgentStatus::Blocked | AgentStatus::Unknown => self.running_since = Some(now),
                AgentStatus::Idle | AgentStatus::Done => self.restart(now),
            },
            AgentStatus::Blocked | AgentStatus::Unknown => {
                if !matches!(
                    self.status,
                    AgentStatus::Working | AgentStatus::Blocked | AgentStatus::Unknown
                ) {
                    self.elapsed = Duration::ZERO;
                }
                self.pause(now);
            }
            AgentStatus::Idle | AgentStatus::Done => self.pause(now),
        }
        self.status = status;
        self.state_change_seq = state_change_seq;
    }

    fn restart(&mut self, now: Instant) {
        self.elapsed = Duration::ZERO;
        self.running_since = Some(now);
    }

    fn pause(&mut self, now: Instant) {
        if let Some(started) = self.running_since.take() {
            self.elapsed += now.saturating_duration_since(started);
        }
    }

    fn elapsed_at(&self, now: Instant) -> Duration {
        self.elapsed
            + self
                .running_since
                .map(|started| now.saturating_duration_since(started))
                .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceGroup {
    pub(crate) name: String,
    pub(crate) expanded: bool,
    workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) name: String,
    entries: Vec<WorkspaceSnapshotEntry>,
    groups: Vec<WorkspaceSnapshotGroup>,
    groups_captured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceSnapshotEntry {
    label: String,
    path: PathBuf,
    focused: bool,
    linked_worktree: bool,
    group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceSnapshotGroup {
    name: String,
    expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceDeleteKind {
    Workspace {
        pane_count: usize,
    },
    Worktree {
        path: Option<PathBuf>,
        parent_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceDeleteDialog {
    pub(crate) workspace_id: String,
    pub(crate) label: String,
    pub(crate) kind: WorkspaceDeleteKind,
}

pub(crate) struct WorkspaceRenameDialog {
    pub(crate) workspace_id: String,
    pub(crate) original_label: String,
    pub(crate) input: TextInput,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotLoadDialog {
    snapshot: WorkspaceSnapshot,
    pub(crate) name: String,
    pub(crate) open_count: usize,
    pub(crate) close_count: usize,
    pub(crate) close_pane_count: usize,
    pub(crate) group_count: usize,
}

struct SnapshotRecallResult {
    groups: Vec<WorkspaceGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspacePanelRow {
    Header,
    Group(usize),
    Workspace(usize),
    Spacer,
    AgentHeader,
    AgentGroup(usize),
    Agent(usize),
    AgentSession(usize),
    EmptyAgents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceDropTarget {
    Group(usize),
    Ungrouped,
}

fn append_agent_cards(rows: &mut Vec<WorkspacePanelRow>, agents: impl IntoIterator<Item = usize>) {
    for (position, index) in agents.into_iter().enumerate() {
        if position > 0 {
            rows.push(WorkspacePanelRow::Spacer);
        }
        rows.push(WorkspacePanelRow::Agent(index));
        rows.push(WorkspacePanelRow::AgentSession(index));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkspaceDrag {
    workspace: usize,
    active: bool,
    target: Option<WorkspaceDropTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectionKey {
    Workspace(String),
    Agent(String),
}

enum Completion {
    Snapshot(Result<(Vec<HerdrWorkspace>, Vec<HerdrAgent>), String>),
    WorkspaceFocus {
        request_id: u64,
        result: Result<(), String>,
    },
    Action {
        result: Result<(), String>,
        reopen_path: Option<PathBuf>,
        warning: Option<String>,
    },
    SnapshotRecall {
        name: String,
        result: Result<SnapshotRecallResult, String>,
    },
}

struct PendingWorkspaceFocus {
    request_id: u64,
    workspace_id: String,
}

#[derive(Default)]
struct WorkspaceFocusState {
    host_workspace_id: Option<String>,
    observed_workspace_id: Option<String>,
    pending: Option<PendingWorkspaceFocus>,
    next_request_id: u64,
}

enum WorkspaceFocusCompletion {
    Ignored,
    Succeeded,
    Failed(String),
}

impl WorkspaceFocusState {
    fn set_host(&mut self, workspace_id: Option<String>) {
        self.host_workspace_id = workspace_id;
    }

    fn host(&self) -> Option<&str> {
        self.host_workspace_id.as_deref()
    }

    fn apply_snapshot(&mut self, workspaces: &[HerdrWorkspace]) {
        self.observed_workspace_id = workspaces
            .iter()
            .find(|workspace| workspace.focused)
            .map(|workspace| workspace.id.clone());
        if self.pending.as_ref().is_some_and(|pending| {
            !workspaces
                .iter()
                .any(|workspace| workspace.id == pending.workspace_id)
        }) {
            self.pending = None;
        }
    }

    fn begin(&mut self, workspace_id: String) -> u64 {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;
        self.pending = Some(PendingWorkspaceFocus {
            request_id,
            workspace_id,
        });
        request_id
    }

    fn complete(
        &mut self,
        request_id: u64,
        result: Result<(), String>,
    ) -> WorkspaceFocusCompletion {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return WorkspaceFocusCompletion::Ignored;
        }
        self.pending = None;
        match result {
            Ok(()) => WorkspaceFocusCompletion::Succeeded,
            Err(error) => WorkspaceFocusCompletion::Failed(error),
        }
    }

    fn active_workspace_id(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .map(|pending| pending.workspace_id.as_str())
            .or(self.observed_workspace_id.as_deref())
            .or(self.host_workspace_id.as_deref())
    }
}

pub(crate) struct WorkspacePanel {
    enabled: bool,
    layout_available: bool,
    pub(crate) placement: WorkspacePanelPlacement,
    pub(crate) workspaces: Vec<HerdrWorkspace>,
    pub(crate) agents: Vec<HerdrAgent>,
    pub(crate) groups: Vec<WorkspaceGroup>,
    pub(crate) selected: Option<usize>,
    pub(crate) workspace_scroll: usize,
    pub(crate) agent_scroll: usize,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
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
    workspace_drag: Option<WorkspaceDrag>,
    last_click: Option<(SelectionKey, Instant)>,
    focus: WorkspaceFocusState,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    next_refresh: Instant,
    spinner_frame: usize,
    next_spinner: Instant,
    agent_timings: HashMap<String, AgentTiming>,
}

pub(crate) struct WorkspacePanelEntryState {
    pub(crate) active: bool,
    pub(crate) loaded: bool,
    pub(crate) selected: bool,
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
        }
        panel
    }

    fn new(enabled: bool, groups_path: Option<PathBuf>, snapshots_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let preset_store = presets::PresetStore::new(groups_path, snapshots_path);
        let (mut groups, snapshots) = if enabled {
            preset_store.load()
        } else {
            (Vec::new(), Vec::new())
        };
        presets::sort_groups(&mut groups);
        Self {
            enabled,
            layout_available: false,
            placement: if enabled {
                WorkspacePanelPlacement::Left
            } else {
                WorkspacePanelPlacement::Off
            },
            workspaces: Vec::new(),
            agents: Vec::new(),
            groups,
            selected: None,
            workspace_scroll: 0,
            agent_scroll: 0,
            loading: false,
            error: None,
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
            workspace_drag: None,
            last_click: None,
            focus: WorkspaceFocusState::default(),
            sender,
            receiver,
            next_refresh: Instant::now(),
            spinner_frame: 0,
            next_spinner: Instant::now(),
            agent_timings: HashMap::new(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn is_available(&self) -> bool {
        self.enabled && self.layout_available
    }

    pub(crate) fn set_layout_available(&mut self, available: bool) {
        self.layout_available = available;
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.placement != WorkspacePanelPlacement::Off
    }

    pub(crate) fn show_left(&mut self) {
        self.placement = WorkspacePanelPlacement::Left;
    }

    pub(crate) fn hide(&mut self) {
        self.placement = WorkspacePanelPlacement::Off;
    }

    pub(crate) fn cycle_placement(&mut self) {
        self.placement = match self.placement {
            WorkspacePanelPlacement::Off => WorkspacePanelPlacement::Left,
            WorkspacePanelPlacement::Left => WorkspacePanelPlacement::Right,
            WorkspacePanelPlacement::Right => WorkspacePanelPlacement::Off,
        };
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
                for (index, workspace) in
                    self.workspaces.iter().enumerate().filter(|(_, workspace)| {
                        !workspace.linked_worktree && group.workspace_ids.contains(&workspace.id)
                    })
                {
                    rows.push(WorkspacePanelRow::Workspace(index));
                    rows.extend(
                        self.child_workspace_indices(&workspace.id)
                            .into_iter()
                            .map(WorkspacePanelRow::Workspace),
                    );
                }
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

        let mut rows = Vec::new();
        for (group_index, group) in self.groups.iter().enumerate() {
            let agents = (0..self.agents.len())
                .filter(|agent| self.group_for_agent(*agent) == Some(group_index))
                .collect::<Vec<_>>();
            if agents.is_empty() {
                continue;
            }
            rows.push(WorkspacePanelRow::Spacer);
            rows.push(WorkspacePanelRow::AgentGroup(group_index));
            if group.expanded {
                append_agent_cards(&mut rows, agents);
            }
        }
        let ungrouped = (0..self.agents.len())
            .filter(|agent| self.group_for_agent(*agent).is_none())
            .collect::<Vec<_>>();
        if !ungrouped.is_empty() && matches!(rows.last(), Some(WorkspacePanelRow::AgentSession(_)))
        {
            rows.push(WorkspacePanelRow::Spacer);
        }
        append_agent_cards(&mut rows, ungrouped);
        rows
    }

    pub(crate) fn poll(&mut self) -> (bool, Option<String>, Option<PathBuf>, bool) {
        if !self.enabled {
            return (false, None, None, false);
        }

        let mut changed = false;
        let mut action_error = None;
        let mut reopen_path = None;
        let mut workspace_focus_succeeded = false;
        while let Ok(completion) = self.receiver.try_recv() {
            changed = true;
            match completion {
                Completion::Snapshot(result) => {
                    self.loading = false;
                    if self.snapshot_loading {
                        continue;
                    }
                    match result {
                        Ok((workspaces, agents)) => {
                            let previous = self.selection_key();
                            self.focus.apply_snapshot(&workspaces);
                            self.workspaces = workspaces;
                            self.apply_agent_snapshot(agents);
                            self.error = None;
                            if self.reconcile_group_workspace_ids()
                                && let Err(error) = self.preset_store.save_groups(&self.groups)
                            {
                                action_error = Some(error);
                            }
                            self.restore_selection(previous);
                        }
                        Err(error) => self.error = Some(error),
                    }
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
                } => match result {
                    Ok(()) => {
                        self.next_refresh = Instant::now();
                        reopen_path = action_reopen_path;
                        action_error = warning;
                    }
                    Err(error) => action_error = Some(error),
                },
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

        if !self.snapshot_loading && !self.loading && Instant::now() >= self.next_refresh {
            self.start_snapshot();
            changed = true;
        }
        changed |= self.poll_spinner(Instant::now());
        (
            changed,
            action_error,
            reopen_path,
            workspace_focus_succeeded,
        )
    }

    fn apply_agent_snapshot(&mut self, agents: Vec<HerdrAgent>) {
        self.apply_agent_snapshot_at(agents, Instant::now());
    }

    fn apply_agent_snapshot_at(&mut self, agents: Vec<HerdrAgent>, now: Instant) {
        self.agent_timings
            .retain(|pane_id, _| agents.iter().any(|agent| agent.pane_id == *pane_id));
        for agent in &agents {
            if let Some(timing) = self.agent_timings.get_mut(&agent.pane_id) {
                timing.observe(agent.status, agent.state_change_seq, now);
            } else if matches!(agent.status, AgentStatus::Working | AgentStatus::Blocked) {
                self.agent_timings.insert(
                    agent.pane_id.clone(),
                    AgentTiming::new(agent.status, agent.state_change_seq, now),
                );
            }
        }

        let previous = &self.agents;
        let mut ranked = agents.into_iter().enumerate().collect::<Vec<_>>();
        ranked.sort_by_key(|(incoming_index, agent)| {
            let previous_index = previous
                .iter()
                .position(|existing| existing.pane_id == agent.pane_id);
            let became_working = agent.status == AgentStatus::Working
                && previous_index
                    .is_none_or(|index| previous[index].status != AgentStatus::Working);
            if became_working {
                (0, *incoming_index)
            } else if agent.status == AgentStatus::Working {
                (1, previous_index.unwrap_or(usize::MAX))
            } else if let Some(previous_index) = previous_index {
                (2, previous_index)
            } else {
                (3, *incoming_index)
            }
        });
        self.agents = ranked.into_iter().map(|(_, agent)| agent).collect();
    }

    pub(crate) fn agent_elapsed(&self, index: usize) -> Option<Duration> {
        self.agent_elapsed_at(index, Instant::now())
    }

    fn agent_elapsed_at(&self, index: usize, now: Instant) -> Option<Duration> {
        let agent = self.agents.get(index)?;
        self.agent_timings
            .get(&agent.pane_id)
            .map(|timing| timing.elapsed_at(now))
    }

    pub(crate) fn spinner_frame(&self) -> usize {
        self.spinner_frame
    }

    fn poll_spinner(&mut self, now: Instant) -> bool {
        let working = self.is_visible()
            && self.layout_available
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
            KeyCode::Char('w') if key.modifiers.is_empty() => WorkspacePanelEffect::Cycle,
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
                WorkspacePanelEffect::None
            }
            KeyCode::End => {
                self.selected = self.visible_selections().last().copied();
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
        let Some(workspace) = self
            .selected
            .and_then(|selected| self.workspaces.get(selected))
        else {
            return WorkspacePanelEffect::Notice("Select a workspace to rename".to_owned());
        };
        let mut input = TextInput::default();
        input.set(workspace.label.clone());
        input.focus();
        input.select_all();
        self.rename_dialog = Some(WorkspaceRenameDialog {
            workspace_id: workspace.id.clone(),
            original_label: workspace.label.clone(),
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
        if label.is_empty() {
            dialog.error = Some("Workspace name is required".to_owned());
            return WorkspacePanelEffect::None;
        }
        if label == dialog.original_label {
            self.rename_dialog = None;
            return WorkspacePanelEffect::None;
        }
        let workspace_id = dialog.workspace_id.clone();
        let label = label.to_owned();
        self.rename_dialog = None;
        WorkspacePanelEffect::RenameWorkspace {
            workspace_id,
            label,
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

    pub(crate) fn close_workspace(&self, workspace_id: &str) {
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

    pub(crate) fn delete_worktree(&self, workspace_id: &str, reopen_path: Option<PathBuf>) {
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
        true
    }

    pub(crate) fn select_agent(&mut self, index: usize) -> bool {
        if index >= self.agents.len() {
            return false;
        }
        self.selected = Some(self.workspaces.len().saturating_add(index));
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
        WorkspacePanelEffect::FocusWorkspace(self.agents[index].workspace_id.clone())
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

    pub(crate) fn group_for_agent(&self, index: usize) -> Option<usize> {
        let workspace_id = &self.agents.get(index)?.workspace_id;
        let workspace = self
            .workspaces
            .iter()
            .position(|workspace| &workspace.id == workspace_id)?;
        self.group_for_workspace(workspace)
    }

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
                    .is_some_and(|(workspace, loaded)| presets::same_path(workspace, loaded))
            }),
            selected: panel_focused && self.selected == Some(index),
        }
    }

    pub(crate) fn agent_entry_state(
        &self,
        index: usize,
        panel_focused: bool,
    ) -> WorkspacePanelEntryState {
        WorkspacePanelEntryState {
            active: self.agents.get(index).is_some_and(|agent| agent.focused),
            loaded: false,
            selected: panel_focused
                && self.selected == Some(self.workspaces.len().saturating_add(index)),
        }
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
        self.workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| workspace.parent_workspace_id.as_deref() == Some(parent_id))
            .map(|(index, _)| index)
            .collect()
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
        let Some(agent) = self.agents.get(agent_index) else {
            return;
        };
        self.start_action(herdr::Action::FocusTab {
            tab_id: agent.tab_id.clone(),
        });
    }

    pub(crate) fn start_workspace_focus(&mut self, workspace_id: String) {
        let request_id = self.focus.begin(workspace_id.clone());
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = herdr::perform(herdr::Action::FocusWorkspace { workspace_id });
            let _ = sender.send(Completion::WorkspaceFocus { request_id, result });
        });
    }

    fn start_action(&self, action: herdr::Action) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = herdr::perform(action);
            let _ = sender.send(Completion::Action {
                result,
                reopen_path: None,
                warning: None,
            });
        });
    }

    fn start_destructive_action(
        &self,
        action: herdr::Action,
        removed_workspace_id: &str,
        reopen_path: Option<PathBuf>,
    ) {
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
            });
        });
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
    }

    pub(crate) fn move_workspace_selection(&mut self, delta: isize) {
        let selections = self
            .workspace_rows()
            .into_iter()
            .filter_map(|row| match row {
                WorkspacePanelRow::Workspace(index) => Some(index),
                _ => None,
            })
            .collect::<Vec<_>>();
        self.move_selection_within(&selections, delta);
    }

    pub(crate) fn move_agent_selection(&mut self, delta: isize) {
        let selections = self
            .agent_rows()
            .into_iter()
            .filter_map(|row| match row {
                WorkspacePanelRow::Agent(index) => {
                    Some(self.workspaces.len().saturating_add(index))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.move_selection_within(&selections, delta);
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
            let result = herdr::session_snapshot().map(|(mut workspaces, agents)| {
                populate_workspace_branches(&mut workspaces);
                (workspaces, agents)
            });
            let _ = sender.send(Completion::Snapshot(result));
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

    pub(crate) fn selected_workspace_visual_row(&self) -> Option<usize> {
        let selected = self.selected?;
        self.workspace_rows().iter().position(
            |row| matches!(row, WorkspacePanelRow::Workspace(index) if *index == selected),
        )
    }

    pub(crate) fn selected_agent_visual_row(&self) -> Option<usize> {
        let selected = self.selected?;
        self.agent_rows().iter().position(|row| {
            matches!(row, WorkspacePanelRow::AgentSession(index)
                if self.workspaces.len().saturating_add(*index) == selected)
        })
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test(value: &Value) -> Self {
        let mut panel = Self::new(true, None, None);
        panel.placement = WorkspacePanelPlacement::Left;
        let (workspaces, agents) = herdr::parse_snapshot(value).unwrap();
        panel.focus.apply_snapshot(&workspaces);
        panel.workspaces = workspaces;
        panel.apply_agent_snapshot(agents);
        panel.restore_selection(None);
        panel
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspacePanelEffect {
    None,
    Unhandled,
    Close,
    Cycle,
    CreateWorkspace,
    CreateWorktree(String),
    RenameWorkspace {
        workspace_id: String,
        label: String,
    },
    CloseWorkspace(String),
    DeleteWorktree {
        workspace_id: String,
        path: Option<PathBuf>,
        parent_path: Option<PathBuf>,
    },
    FocusWorkspace(String),
    OpenWorkspace(PathBuf),
    Notice(String),
}

fn recall_snapshot(
    snapshot: &WorkspaceSnapshot,
    current: &[HerdrWorkspace],
) -> Result<SnapshotRecallResult, String> {
    let matches = presets::matching_indices(snapshot, current);
    let mut used = vec![false; current.len()];
    let mut target_ids = Vec::with_capacity(snapshot.entries.len());
    let mut renames = Vec::new();
    for (entry, matched) in snapshot.entries.iter().zip(matches) {
        if let Some(index) = matched {
            let workspace = &current[index];
            used[index] = true;
            if workspace.label != entry.label {
                renames.push((workspace.id.clone(), entry.label.clone()));
            }
            target_ids.push(workspace.id.clone());
            continue;
        }

        let id = herdr::restore(herdr::RestoreRequest {
            path: entry.path.clone(),
            label: entry.label.clone(),
            linked_worktree: entry.linked_worktree,
        })
        .map_err(|error| {
            format!(
                "Could not load preset '{}' completely: {error}",
                snapshot.name
            )
        })?
        .ok_or_else(|| {
            format!(
                "Could not identify '{}' while loading preset '{}'",
                entry.label, snapshot.name
            )
        })?;
        target_ids.push(id);
    }

    for (workspace_id, label) in renames {
        herdr::perform(herdr::Action::RenameWorkspace {
            workspace_id,
            label,
        })?;
    }

    let focus_index = snapshot
        .entries
        .iter()
        .position(|entry| entry.focused)
        .unwrap_or(0);
    herdr::perform(herdr::Action::FocusWorkspace {
        workspace_id: target_ids[focus_index].clone(),
    })?;

    let mut extras = current
        .iter()
        .enumerate()
        .filter(|(index, _)| !used[*index])
        .map(|(_, workspace)| workspace)
        .collect::<Vec<_>>();
    extras.sort_by_key(|workspace| workspace.focused);
    for workspace in extras {
        herdr::perform(herdr::Action::CloseWorkspace {
            workspace_id: workspace.id.clone(),
        })?;
    }

    let groups = presets::groups_after_recall(snapshot, &target_ids);
    Ok(SnapshotRecallResult { groups })
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
mod tests {
    use super::*;

    fn snapshot() -> Value {
        serde_json::json!({
            "result": {
                "snapshot": {
                    "workspaces": [
                        {
                            "workspace_id": "w1",
                            "label": "HUNKLE",
                            "active_tab_id": "w1:t1",
                            "number": 2,
                            "pane_count": 2,
                            "focused": true,
                            "agent_status": "working"
                        },
                        {
                            "workspace_id": "w2",
                            "label": "docs",
                            "number": 3,
                            "pane_count": 1,
                            "focused": false,
                            "agent_status": "idle"
                        }
                    ],
                    "agents": [{
                        "agent": "opencode",
                        "agent_status": "blocked",
                        "focused": true,
                        "pane_id": "w1:p1",
                        "tab_id": "w1:t1",
                        "workspace_id": "w1"
                    }],
                    "panes": [{
                        "pane_id": "w1:p1",
                        "tab_id": "w1:t1",
                        "workspace_id": "w1",
                        "cwd": "/home/spoon/code/gitui",
                        "foreground_cwd": "/home/spoon/code/gitui"
                    }],
                    "layouts": [{
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "focused_pane_id": "w1:p1"
                    }]
                }
            }
        })
    }

    fn agent(name: &str, status: AgentStatus) -> HerdrAgent {
        HerdrAgent {
            name: name.to_owned(),
            session_name: None,
            workspace_id: "workspace".to_owned(),
            tab_id: format!("tab-{name}"),
            pane_id: format!("pane-{name}"),
            focused: false,
            status,
            state_change_seq: 0,
        }
    }

    fn agent_at_sequence(name: &str, status: AgentStatus, state_change_seq: u64) -> HerdrAgent {
        HerdrAgent {
            state_change_seq,
            ..agent(name, status)
        }
    }

    #[test]
    fn tracks_active_time_for_the_latest_agent_request() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.agents.clear();
        panel.agent_timings.clear();
        let started = Instant::now();

        panel.apply_agent_snapshot_at(
            vec![agent_at_sequence("alpha", AgentStatus::Idle, 1)],
            started,
        );
        assert_eq!(panel.agent_elapsed_at(0, started), None);

        panel.apply_agent_snapshot_at(
            vec![agent_at_sequence("alpha", AgentStatus::Working, 2)],
            started + Duration::from_secs(2),
        );
        assert_eq!(
            panel.agent_elapsed_at(0, started + Duration::from_secs(7)),
            Some(Duration::from_secs(5))
        );

        panel.apply_agent_snapshot_at(
            vec![agent_at_sequence("alpha", AgentStatus::Blocked, 3)],
            started + Duration::from_secs(7),
        );
        assert_eq!(
            panel.agent_elapsed_at(0, started + Duration::from_secs(17)),
            Some(Duration::from_secs(5))
        );

        panel.apply_agent_snapshot_at(
            vec![agent_at_sequence("alpha", AgentStatus::Working, 4)],
            started + Duration::from_secs(17),
        );
        panel.apply_agent_snapshot_at(
            vec![agent_at_sequence("alpha", AgentStatus::Idle, 5)],
            started + Duration::from_secs(20),
        );
        assert_eq!(
            panel.agent_elapsed_at(0, started + Duration::from_secs(40)),
            Some(Duration::from_secs(8))
        );

        panel.apply_agent_snapshot_at(
            vec![agent_at_sequence("alpha", AgentStatus::Working, 6)],
            started + Duration::from_secs(40),
        );
        assert_eq!(
            panel.agent_elapsed_at(0, started + Duration::from_secs(42)),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn promotes_newly_working_agents_without_resorting_the_rest() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.agents = vec![
            agent("alpha", AgentStatus::Idle),
            agent("beta", AgentStatus::Idle),
            agent("gamma", AgentStatus::Idle),
        ];

        panel.apply_agent_snapshot(vec![
            agent("alpha", AgentStatus::Idle),
            agent("beta", AgentStatus::Working),
            agent("gamma", AgentStatus::Idle),
        ]);
        assert_eq!(
            panel
                .agents
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>(),
            ["beta", "alpha", "gamma"]
        );

        panel.apply_agent_snapshot(vec![
            agent("alpha", AgentStatus::Idle),
            agent("beta", AgentStatus::Working),
            agent("gamma", AgentStatus::Working),
        ]);
        assert_eq!(
            panel
                .agents
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>(),
            ["gamma", "beta", "alpha"]
        );

        panel.apply_agent_snapshot(vec![
            agent("alpha", AgentStatus::Idle),
            agent("gamma", AgentStatus::Idle),
            agent("beta", AgentStatus::Working),
        ]);
        assert_eq!(
            panel
                .agents
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>(),
            ["beta", "gamma", "alpha"]
        );

        panel.apply_agent_snapshot(vec![
            agent("alpha", AgentStatus::Idle),
            agent("beta", AgentStatus::Idle),
            agent("gamma", AgentStatus::Idle),
        ]);
        assert_eq!(
            panel
                .agents
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>(),
            ["beta", "gamma", "alpha"]
        );
    }

    #[test]
    fn parses_snapshot_and_tracks_workspace_and_agent_selection() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        assert_eq!(panel.workspaces.len(), 2);
        assert_eq!(panel.agents.len(), 1);
        assert_eq!(panel.workspaces[0].status, AgentStatus::Working);
        assert_eq!(
            panel.workspaces[0].path.as_deref(),
            Some(Path::new("/home/spoon/code/gitui"))
        );
        let mut worktree_snapshot = snapshot();
        worktree_snapshot["result"]["snapshot"]["workspaces"][0]["worktree"] =
            serde_json::json!({ "checkout_path": "/tmp/hunkle-worktree" });
        let (workspaces, _) = herdr::parse_snapshot(&worktree_snapshot).unwrap();
        assert_eq!(
            workspaces[0].path.as_deref(),
            Some(Path::new("/tmp/hunkle-worktree"))
        );
        assert_eq!(panel.agents[0].status, AgentStatus::Blocked);
        assert_eq!(panel.selected, Some(0));
        assert_eq!(panel.selected_visual_row(), Some(1));

        panel.move_selection(2);
        assert_eq!(panel.selected, Some(2));
        assert_eq!(panel.selected_visual_row(), Some(5));
        panel.move_selection(1);
        assert_eq!(panel.selected, Some(2));

        assert_eq!(
            panel.click_workspace(0),
            WorkspacePanelEffect::OpenWorkspace(PathBuf::from("/home/spoon/code/gitui"))
        );
        assert_eq!(panel.selected, Some(0));
    }

    #[test]
    fn a_second_workspace_click_becomes_a_focus_request() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.workspaces[1].path = Some(PathBuf::from("/tmp/work-b"));

        assert!(!panel.register_workspace_click(1));
        assert!(panel.register_workspace_click(1));
    }

    #[test]
    fn every_agent_click_requests_its_workspace() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());

        for _ in 0..2 {
            assert_eq!(
                panel.click_agent(0),
                WorkspacePanelEffect::FocusWorkspace("w1".to_owned())
            );
            assert_eq!(panel.selected, Some(panel.workspaces.len()));
        }
    }

    #[test]
    fn keeps_the_target_workspace_active_until_herdr_confirms_focus() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.next_refresh = Instant::now() + Duration::from_secs(60);
        assert!(panel.select_workspace(1));
        let request_id = panel.focus.begin("w2".to_owned());

        assert!(!panel.workspace_is_active(0));
        assert!(panel.workspace_is_active(1));

        let stale = herdr::parse_snapshot(&snapshot()).unwrap();
        panel.sender.send(Completion::Snapshot(Ok(stale))).unwrap();
        let (changed, error, _, focus_succeeded) = panel.poll();
        assert!(changed);
        assert!(error.is_none());
        assert!(!focus_succeeded);
        assert_eq!(panel.selected, Some(1));
        assert!(!panel.workspace_is_active(0));
        assert!(panel.workspace_is_active(1));
        assert!(panel.focus.pending.is_some());

        let mut confirmed = snapshot();
        confirmed["result"]["snapshot"]["workspaces"][0]["focused"] = false.into();
        confirmed["result"]["snapshot"]["workspaces"][1]["focused"] = true.into();
        panel
            .sender
            .send(Completion::Snapshot(Ok(
                herdr::parse_snapshot(&confirmed).unwrap()
            )))
            .unwrap();
        panel.poll();

        assert!(panel.focus.pending.is_some());
        assert!(!panel.workspace_is_active(0));
        assert!(panel.workspace_is_active(1));

        panel
            .sender
            .send(Completion::WorkspaceFocus {
                request_id,
                result: Ok(()),
            })
            .unwrap();
        let (_, _, _, focus_succeeded) = panel.poll();
        assert!(focus_succeeded);
        assert!(panel.focus.pending.is_none());
        assert!(!panel.workspace_is_active(0));
        assert!(panel.workspace_is_active(1));
    }

    #[test]
    fn herdr_focus_overrides_the_workspace_that_hosts_hunkle() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.focus.set_host(Some("w2".to_owned()));
        panel.restore_selection(None);

        assert!(panel.workspace_is_active(0));
        assert!(!panel.workspace_is_active(1));
        assert_eq!(panel.selected, Some(1));

        let stale = herdr::parse_snapshot(&snapshot()).unwrap();
        panel.workspaces = stale.0;
        assert!(panel.workspace_is_active(0));
        assert!(!panel.workspace_is_active(1));
    }

    #[test]
    fn prepares_the_hidden_process_cursor_after_focusing_away() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.focus.set_host(Some("w1".to_owned()));
        assert!(panel.select_workspace(1));
        panel.loading = true;
        let request_id = panel.focus.begin("w2".to_owned());
        panel
            .sender
            .send(Completion::WorkspaceFocus {
                request_id,
                result: Ok(()),
            })
            .unwrap();

        let (_, _, _, focus_succeeded) = panel.poll();

        assert!(focus_succeeded);
        assert!(panel.focus.pending.is_none());
        assert_eq!(panel.selected, Some(0));
        assert_eq!(panel.selected_workspace_id(), Some("w1"));
        assert!(panel.workspace_is_active(0));
    }

    #[test]
    fn rolls_back_only_the_current_failed_workspace_focus() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.loading = true;
        let old_request = panel.focus.begin("w2".to_owned());
        let current_request = panel.focus.begin("w1".to_owned());
        panel
            .sender
            .send(Completion::WorkspaceFocus {
                request_id: old_request,
                result: Err("old failure".to_owned()),
            })
            .unwrap();
        let (_, error, _, _) = panel.poll();
        assert!(error.is_none());
        assert_eq!(
            panel
                .focus
                .pending
                .as_ref()
                .map(|pending| pending.request_id),
            Some(current_request)
        );
        assert!(panel.workspace_is_active(0));

        panel
            .sender
            .send(Completion::WorkspaceFocus {
                request_id: current_request,
                result: Err("focus failed".to_owned()),
            })
            .unwrap();
        let (_, error, _, _) = panel.poll();
        assert_eq!(error.as_deref(), Some("focus failed"));
        assert!(panel.focus.pending.is_none());
        assert!(panel.workspace_is_active(0));
        assert!(!panel.workspace_is_active(1));
    }

    #[test]
    fn confirms_workspace_close_or_linked_worktree_removal() {
        let mut value = snapshot();
        value["result"]["snapshot"]["workspaces"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "workspace_id": "w3",
                "label": "feature-worktree",
                "pane_count": 1,
                "focused": false,
                "agent_status": "idle",
                "worktree": {
                    "checkout_path": "/tmp/worktrees/feature",
                    "is_linked_worktree": true,
                    "repo_key": "/home/spoon/code/gitui/.git",
                    "repo_root": "/home/spoon/code/gitui"
                }
            }));
        let mut panel = WorkspacePanel::ready_for_test(&value);

        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert_eq!(
            panel.delete_dialog.as_ref().map(|dialog| &dialog.kind),
            Some(&WorkspaceDeleteKind::Workspace { pane_count: 2 })
        );
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert!(panel.delete_dialog.is_none());

        panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::CloseWorkspace("w1".to_owned())
        );
        assert!(panel.delete_dialog.is_none());

        assert!(panel.select_workspace(2));
        panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(
            panel.delete_dialog.as_ref().map(|dialog| &dialog.kind),
            Some(&WorkspaceDeleteKind::Worktree {
                path: Some(PathBuf::from("/tmp/worktrees/feature")),
                parent_path: Some(PathBuf::from("/home/spoon/code/gitui")),
            })
        );
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            WorkspacePanelEffect::DeleteWorktree {
                workspace_id: "w3".to_owned(),
                path: Some(PathBuf::from("/tmp/worktrees/feature")),
                parent_path: Some(PathBuf::from("/home/spoon/code/gitui")),
            }
        );

        assert!(panel.select_agent(0));
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            WorkspacePanelEffect::Notice("Select a workspace to close".to_owned())
        );
    }

    #[test]
    fn renames_only_the_selected_workspace() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());

        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        let dialog = panel.rename_dialog.as_ref().unwrap();
        assert_eq!(dialog.workspace_id, "w1");
        assert_eq!(dialog.original_label, "HUNKLE");
        assert_eq!(dialog.input.selection(), Some((0, "HUNKLE".len())));

        panel.paste("renamed");
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::RenameWorkspace {
                workspace_id: "w1".to_owned(),
                label: "renamed".to_owned(),
            }
        );
        assert!(panel.rename_dialog.is_none());

        panel.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        panel.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert_eq!(
            panel
                .rename_dialog
                .as_ref()
                .and_then(|dialog| dialog.error.as_deref()),
            Some("Workspace name is required")
        );
        panel.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(panel.rename_dialog.is_none());

        assert!(panel.select_agent(0));
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            WorkspacePanelEffect::Notice("Select a workspace to rename".to_owned())
        );
        assert!(panel.rename_dialog.is_none());
    }

    #[test]
    fn preserves_herdr_focus_when_removing_another_workspace() {
        let panel = WorkspacePanel::ready_for_test(&snapshot());

        assert_eq!(
            panel.focus_to_restore_after_removing("w2").as_deref(),
            Some("w1")
        );
        assert_eq!(panel.focus_to_restore_after_removing("w1"), None);
    }

    #[test]
    fn reopens_the_parent_only_after_successful_worktree_removal() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.next_refresh = Instant::now() + Duration::from_secs(60);
        panel.loading = true;
        let parent = PathBuf::from("/home/spoon/code/gitui");

        panel
            .sender
            .send(Completion::Action {
                result: Ok(()),
                reopen_path: Some(parent.clone()),
                warning: None,
            })
            .unwrap();
        let (changed, error, reopen_path, _) = panel.poll();
        assert!(changed);
        assert_eq!(error, None);
        assert_eq!(reopen_path, Some(parent.clone()));

        panel.next_refresh = Instant::now() + Duration::from_secs(60);
        panel
            .sender
            .send(Completion::Action {
                result: Err("worktree has uncommitted changes".to_owned()),
                reopen_path: Some(parent),
                warning: None,
            })
            .unwrap();
        let (changed, error, reopen_path, _) = panel.poll();
        assert!(changed);
        assert_eq!(error.as_deref(), Some("worktree has uncommitted changes"));
        assert_eq!(reopen_path, None);
    }

    #[test]
    fn create_menu_requires_a_selected_workspace_for_worktrees() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.toggle_create_menu();
        assert!(panel.create_menu_open);
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert_eq!(panel.create_menu_choice, 1);
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::CreateWorktree("w1".to_owned())
        );
        assert!(!panel.create_menu_open);

        assert!(panel.select_agent(0));
        panel.toggle_create_menu();
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert_eq!(panel.create_menu_choice, 0);
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::CreateWorkspace
        );
    }

    #[test]
    fn reads_branches_from_repositories_and_linked_worktrees() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repository");
        let nested = repository.join("src/nested");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            repository.join(".git/HEAD"),
            "ref: refs/heads/feature/panel\n",
        )
        .unwrap();
        assert_eq!(workspace_branch(&nested).as_deref(), Some("feature/panel"));

        let worktree = directory.path().join("worktree");
        let git_dir = directory.path().join("git-data");
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(worktree.join(".git"), "gitdir: ../git-data\n").unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/topic/worktree\n").unwrap();
        assert_eq!(
            workspace_branch(&worktree).as_deref(),
            Some("topic/worktree")
        );
    }

    #[test]
    fn nests_linked_worktrees_under_their_parent_and_prevents_dragging_them() {
        let mut value = snapshot();
        let workspaces = value["result"]["snapshot"]["workspaces"]
            .as_array_mut()
            .unwrap();
        workspaces.push(serde_json::json!({
            "workspace_id": "w3",
            "label": "feature-worktree",
            "pane_count": 1,
            "focused": false,
            "agent_status": "idle",
            "worktree": {
                "checkout_path": "/tmp/worktrees/feature",
                "is_linked_worktree": true,
                "repo_key": "/home/spoon/code/gitui/.git",
                "repo_root": "/home/spoon/code/gitui"
            }
        }));
        let mut panel = WorkspacePanel::ready_for_test(&value);

        assert_eq!(
            panel.workspaces[2].parent_workspace_id.as_deref(),
            Some("w1")
        );
        assert_eq!(
            &panel.rows()[..4],
            &[
                WorkspacePanelRow::Header,
                WorkspacePanelRow::Workspace(0),
                WorkspacePanelRow::Workspace(2),
                WorkspacePanelRow::Workspace(1),
            ]
        );
        assert_eq!(panel.workspace_indent(2), " ");
        assert!(!panel.begin_workspace_drag(2));

        panel.groups = vec![
            WorkspaceGroup {
                name: "Project".to_owned(),
                expanded: true,
                workspace_ids: vec!["w1".to_owned()],
            },
            WorkspaceGroup {
                name: "Old worktree group".to_owned(),
                expanded: true,
                workspace_ids: vec!["w3".to_owned()],
            },
        ];
        assert!(panel.reconcile_group_workspace_ids());
        assert!(panel.groups[1].workspace_ids.is_empty());
        assert_eq!(panel.group_for_workspace(2), Some(0));
        assert_eq!(panel.workspace_indent(2), "  ");
        assert_eq!(
            &panel.rows()[..5],
            &[
                WorkspacePanelRow::Header,
                WorkspacePanelRow::Spacer,
                WorkspacePanelRow::Group(0),
                WorkspacePanelRow::Workspace(0),
                WorkspacePanelRow::Workspace(2),
            ]
        );
        assert_eq!(panel.rows()[5], WorkspacePanelRow::Spacer);
        assert_eq!(panel.rows()[6], WorkspacePanelRow::Group(1));
    }

    #[test]
    fn persists_groups_and_moves_workspaces_between_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace-groups.json");
        let mut panel = WorkspacePanel::new(true, Some(path.clone()), None);
        assert_eq!(panel.placement, WorkspacePanelPlacement::Left);
        panel.cycle_placement();
        assert_eq!(panel.placement, WorkspacePanelPlacement::Right);
        panel.cycle_placement();
        assert_eq!(panel.placement, WorkspacePanelPlacement::Off);
        panel.cycle_placement();
        assert_eq!(panel.placement, WorkspacePanelPlacement::Left);
        let (workspaces, agents) = herdr::parse_snapshot(&snapshot()).unwrap();
        panel.workspaces = workspaces;
        panel.agents = agents;
        panel.restore_selection(None);

        panel.begin_group();
        panel.paste("Zulu work");
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert!(panel.begin_workspace_drag(0));
        panel.update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
        assert_eq!(panel.finish_workspace_drag(), WorkspacePanelEffect::None);
        assert_eq!(panel.group_for_workspace(0), Some(0));
        assert_eq!(
            panel.agent_rows(),
            [
                WorkspacePanelRow::Spacer,
                WorkspacePanelRow::AgentGroup(0),
                WorkspacePanelRow::Agent(0),
                WorkspacePanelRow::AgentSession(0),
            ]
        );
        assert!(path.exists());

        assert!(panel.begin_workspace_drag(1));
        panel.update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
        assert_eq!(panel.finish_workspace_drag(), WorkspacePanelEffect::None);
        assert_eq!(panel.group_for_workspace(1), Some(0));

        panel.begin_group();
        panel.paste("alpha work");
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert_eq!(panel.groups[0].name, "alpha work");
        assert_eq!(panel.groups[1].name, "Zulu work");
        assert!(panel.begin_workspace_drag(0));
        panel.update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
        assert_eq!(panel.finish_workspace_drag(), WorkspacePanelEffect::None);
        assert_eq!(panel.group_for_workspace(0), Some(0));
        assert_eq!(panel.groups[1].workspace_ids, ["w2"]);
        assert_eq!(
            panel.agent_rows(),
            [
                WorkspacePanelRow::Spacer,
                WorkspacePanelRow::AgentGroup(0),
                WorkspacePanelRow::Agent(0),
                WorkspacePanelRow::AgentSession(0),
            ]
        );

        panel.toggle_group(0);
        assert_eq!(
            panel.agent_rows(),
            [WorkspacePanelRow::Spacer, WorkspacePanelRow::AgentGroup(0)]
        );
        panel.toggle_group(0);

        panel.toggle_group(1);
        assert!(!panel.groups[1].expanded);
        assert!(!panel.rows().contains(&WorkspacePanelRow::Workspace(1)));

        let restored = WorkspacePanel::new(true, Some(path), None);
        assert_eq!(restored.groups[0].name, "alpha work");
        assert_eq!(restored.groups[0].workspace_ids, ["w1"]);
        assert_eq!(restored.groups[1].name, "Zulu work");
        assert!(!restored.groups[1].expanded);
        assert_eq!(restored.groups[1].workspace_ids, ["w2"]);
    }

    #[test]
    fn saves_loads_and_deletes_named_workspace_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace-snapshots.json");
        let mut panel = WorkspacePanel::new(true, None, Some(path.clone()));
        let (mut workspaces, agents) = herdr::parse_snapshot(&snapshot()).unwrap();
        workspaces[1].path = Some(PathBuf::from("/home/spoon/docs"));
        panel.workspaces = workspaces;
        panel.agents = agents;
        panel.groups = vec![
            WorkspaceGroup {
                name: "Active work".to_owned(),
                expanded: false,
                workspace_ids: vec!["w1".to_owned()],
            },
            WorkspaceGroup {
                name: "Empty later".to_owned(),
                expanded: true,
                workspace_ids: Vec::new(),
            },
        ];

        panel.toggle_snapshot_menu();
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert!(panel.snapshot_editing);
        panel.paste("Daily setup");
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::Notice("Preset saved: Daily setup".to_owned())
        );
        assert_eq!(panel.snapshots.len(), 1);
        assert_eq!(panel.snapshots[0].workspace_count(), 2);

        panel.groups[0].expanded = true;
        panel.toggle_snapshot_menu();
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        panel.paste("Daily setup");
        assert_eq!(
            panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WorkspacePanelEffect::Notice("Preset updated: Daily setup".to_owned())
        );
        assert_eq!(panel.snapshots.len(), 1);

        let mut restored = WorkspacePanel::new(true, None, Some(path.clone()));
        assert_eq!(restored.snapshots.len(), 1);
        assert_eq!(restored.snapshots[0].name, "Daily setup");
        assert_eq!(restored.snapshots[0].entries[0].label, "HUNKLE");
        assert!(restored.snapshots[0].entries[0].focused);
        assert_eq!(
            restored.snapshots[0].entries[0].group.as_deref(),
            Some("Active work")
        );
        assert_eq!(restored.snapshots[0].groups.len(), 2);
        assert!(restored.snapshots[0].groups[0].expanded);
        assert_eq!(restored.snapshots[0].groups[1].name, "Empty later");
        assert_eq!(
            restored.snapshots[0].entries[1].path,
            PathBuf::from("/home/spoon/docs")
        );

        restored.workspaces = panel.workspaces.clone();
        restored.toggle_snapshot_menu();
        assert_eq!(
            restored.activate_snapshot_choice(1),
            WorkspacePanelEffect::None
        );
        let dialog = restored.snapshot_load_dialog.as_ref().unwrap();
        assert_eq!(dialog.open_count, 0);
        assert_eq!(dialog.close_count, 0);
        assert_eq!(dialog.group_count, 2);
        assert_eq!(
            restored.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert!(restored.snapshot_load_dialog.is_none());

        restored.toggle_snapshot_menu();
        restored.snapshot_menu_choice = 1;
        assert_eq!(
            restored.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            WorkspacePanelEffect::None
        );
        assert!(restored.snapshots.is_empty());
        assert!(
            WorkspacePanel::new(true, None, Some(path))
                .snapshots
                .is_empty()
        );
    }

    #[test]
    fn migrates_legacy_snapshot_groups_by_path_before_workspace_ids_change() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.workspaces[1].path = Some(PathBuf::from("/home/spoon/docs"));
        panel.groups = vec![
            WorkspaceGroup {
                name: "Code".to_owned(),
                expanded: false,
                workspace_ids: vec!["w1".to_owned()],
            },
            WorkspaceGroup {
                name: "Notes".to_owned(),
                expanded: true,
                workspace_ids: vec!["w2".to_owned()],
            },
        ];
        panel.snapshots = vec![WorkspaceSnapshot {
            name: "legacy".to_owned(),
            entries: vec![
                WorkspaceSnapshotEntry {
                    label: "HUNKLE".to_owned(),
                    path: PathBuf::from("/home/spoon/code/gitui"),
                    focused: true,
                    linked_worktree: false,
                    group: None,
                },
                WorkspaceSnapshotEntry {
                    label: "docs".to_owned(),
                    path: PathBuf::from("/home/spoon/docs"),
                    focused: false,
                    linked_worktree: false,
                    group: None,
                },
            ],
            groups: Vec::new(),
            groups_captured: false,
        }];

        assert_eq!(
            panel.activate_snapshot_choice(1),
            WorkspacePanelEffect::None
        );
        let migrated = &panel.snapshot_load_dialog.as_ref().unwrap().snapshot;
        assert!(migrated.groups_captured);
        assert!(panel.snapshots[0].groups_captured);
        assert_eq!(migrated.entries[0].group.as_deref(), Some("Code"));
        assert_eq!(migrated.entries[1].group.as_deref(), Some("Notes"));

        let groups = presets::groups_after_recall(
            migrated,
            &["new-code".to_owned(), "new-notes".to_owned()],
        );
        assert_eq!(groups[0].workspace_ids, ["new-code"]);
        assert_eq!(groups[1].workspace_ids, ["new-notes"]);
        assert!(!groups[0].expanded);
        assert!(groups[1].expanded);
    }

    #[test]
    fn animates_the_status_marker_while_an_agent_is_working() {
        let mut panel = WorkspacePanel::ready_for_test(&snapshot());
        panel.set_layout_available(true);
        let now = Instant::now();
        panel.next_spinner = now;

        assert!(panel.poll_spinner(now));
        assert_eq!(panel.spinner_frame, 1);
        assert!(!panel.poll_spinner(now));
        assert!(panel.poll_spinner(now + SPINNER_INTERVAL));
        assert_eq!(panel.spinner_frame, 2);

        panel.workspaces[0].status = AgentStatus::Idle;
        assert!(!panel.poll_spinner(now + SPINNER_INTERVAL * 2));
        assert_eq!(panel.spinner_frame, 0);
    }
}
