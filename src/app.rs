mod actions;
mod author_filter;
mod changes;
mod commit_message;
mod commit_summary;
mod explorer;
mod file_search;
mod files;
mod fuzzy;
mod herdr_prompt;
mod mouse;
mod repository_browser;
mod settings;
mod text_input;
mod workspace_panel;
mod worktree_manager;

pub(crate) use actions::{ACTION_ITEMS, ActionsState, CommandRecord, CommandStatus};
pub(crate) use author_filter::{AuthorFilter, AuthorFilterEffect};
pub(crate) use changes::{ChangesHitTarget, SqliteFocus, SqlitePage};
pub use changes::{ChangesState, LeftPane};
pub(crate) use commit_message::CommitMessageGenerator;
pub(crate) use commit_summary::CommitSummaryCache;
pub use explorer::{Explorer, PickerAction, PickerEntry};
pub(crate) use explorer::{ExplorerHitTarget, SurroundingEntry};
pub(crate) use file_search::FileSearch;
pub(crate) use files::{FileDialog, FileDialogKind, FileDrag, FileNameAction};
pub(crate) use herdr_prompt::HerdrPrompt;
pub(crate) use repository_browser::{
    BranchDeleteDialog, BrowserTab, PullRequest, RemoteItems, RepositoryBrowser,
    RepositoryBrowserEffect,
};
pub use settings::Settings;
pub(crate) use settings::SettingsStore;
pub(crate) use workspace_panel::{
    AgentStatus, DEFAULT_WIDTH as DEFAULT_WORKSPACE_PANEL_WIDTH,
    MINIMUM_WIDTH as MINIMUM_WORKSPACE_PANEL_WIDTH, SPINNER_FRAMES, SnapshotLoadDialog,
    WorkspaceDeleteDialog, WorkspaceDeleteKind, WorkspaceDropTarget, WorkspacePanel,
    WorkspacePanelEffect, WorkspacePanelPlacement, WorkspacePanelRow, WorkspaceRenameDialog,
};
pub(crate) use worktree_manager::{
    WorktreeCandidate, WorktreeCreateDialog, WorktreeCreateField, WorktreeManager,
    WorktreeManagerEffect, WorktreeManagerRow, WorktreeRemoveDialog, short_head, worktree_label,
};

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const WORKSPACE_FETCH_FRESHNESS: Duration = Duration::from_secs(5 * 60);

struct CommitDraftResult {
    root: PathBuf,
    result: Result<(PathBuf, Option<String>), String>,
}

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Position, Rect},
    widgets::TableState,
};

use crate::{
    diagnostics,
    filesystem::{atomic_write, same_path},
    formatter,
    git::{self, RefreshScope, RepositoryData},
    repo_path::RepoPath,
    repository_session::{LoadKind, Mutation, RepositorySession, WorkerOutcome},
    selection::SelectionState,
};

use actions::{ActionId, action_command, display_git_command, parse_command_args, parse_git_args};
use explorer::PickerCommand;
pub(crate) use text_input::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Changes,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Commit,
    FileSearch,
    Explorer,
    Settings,
    Help,
    RepositoryBrowser,
    WorktreeManager,
    AuthorFilter,
    ActionMenu,
    Command,
    HerdrPrompt,
    Editor,
    Files,
    WorkspacePanel,
    WorkspacePresets,
}

#[derive(Debug, Clone, Copy)]
pub struct DiffHunkRegion {
    pub rect: Rect,
    pub index: usize,
    pub continues_above: bool,
    pub continues_below: bool,
    pub scroll_start: usize,
    pub scroll_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HitTarget {
    Changes(ChangesHitTarget),
    CommitMessageGenerate,
    MarkdownPreviewToggle,
    Graph(GraphHitTarget),
    Explorer(ExplorerHitTarget),
    RepositoryBrowser(RepositoryBrowserHitTarget),
    WorktreeManager(WorktreeManagerHitTarget),
    WorkspacePanel(WorkspacePanelHitTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeManagerHitTarget {
    Overlay,
    List,
    Item { generation: u64, row: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspacePanelHitTarget {
    Focus,
    Collapse,
    CreateMenu,
    CreateWorkspace,
    CreateWorktree,
    SnapshotMenu,
    PresetOverlay,
    SaveSnapshot,
    Snapshot(usize),
    Group(usize),
    Workspace(usize),
    Agent(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphHitTarget {
    AuthorHeader,
    FilterOverlay,
    FilterItem(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryBrowserHitTarget {
    Overlay,
    List,
    Tab(BrowserTab),
    Item(usize),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HitRegion {
    target: HitTarget,
    rect: Rect,
}

#[derive(Debug, Default, Clone)]
pub struct Regions {
    pub screen: Option<Rect>,
    pub changes: Option<Rect>,
    pub graph: Option<Rect>,
    pub left_pane_toggle: Option<Rect>,
    pub explorer: Option<Rect>,
    pub repository_browser: Option<Rect>,
    pub worktree_manager: Option<Rect>,
    pub settings: Option<Rect>,
    pub help: Option<Rect>,
    pub workspace_panel: Option<Rect>,
    pub workspace_panel_workspaces: Option<Rect>,
    pub workspace_panel_agents: Option<Rect>,
    pub workspace_panel_splitter: Option<Rect>,
    pub workspace_panel_bounds: Option<Rect>,
    pub workspace_presets_overlay: Option<Rect>,
    pub actions: Option<Rect>,
    pub worktree: Option<Rect>,
    pub worktree_list: Option<Rect>,
    pub explorer_list: Option<Rect>,
    pub history_list: Option<Rect>,
    pub history_splitter: Option<Rect>,
    pub history_bounds: Option<Rect>,
    pub diff: Option<Rect>,
    pub diff_scrollbar: Option<Rect>,
    pub diff_scroll_thumb: Option<Rect>,
    pub diff_scroll_max: usize,
    pub sqlite_objects: Option<Rect>,
    pub sqlite_rows: Option<Rect>,
    pub splitter: Option<Rect>,
    pub split_bounds: Option<Rect>,
    pub commit: Option<Rect>,
    pub commit_scroll: usize,
    pub commit_scroll_max: usize,
    pub graph_table: Option<Rect>,
    pub settings_overlay: Option<Rect>,
    pub action_menu: Option<Rect>,
    pub action_list: Option<Rect>,
    pub command_overlay: Option<Rect>,
    pub command_output: Option<Rect>,
    pub herdr_prompt_overlay: Option<Rect>,
    pub editor_overlay: Option<Rect>,
    pub file_search_overlay: Option<Rect>,
    pub file_search_list: Option<Rect>,
    pub files_add: Option<Rect>,
    pub files_root: Option<Rect>,
    pub file_dialog_overlay: Option<Rect>,
    pub file_dialog_primary: Option<Rect>,
    pub file_dialog_secondary: Option<Rect>,
    pub editor_setting: Option<Rect>,
    pub media_preview_setting: Option<Rect>,
    pub auto_fetch: Option<Rect>,
    pub workspace_panel_setting: Option<Rect>,
    pub agent_harness_setting: Option<Rect>,
    pub fetch_interval: Option<Rect>,
    pub fetch_interval_down: Option<Rect>,
    pub fetch_interval_up: Option<Rect>,
    pub diff_hunks: Vec<DiffHunkRegion>,
    hit_regions: Vec<HitRegion>,
}

impl Regions {
    pub(crate) fn register_hit_target(&mut self, target: HitTarget, rect: Rect) {
        self.hit_regions.push(HitRegion { target, rect });
    }

    pub(crate) fn hit_target_at(&self, point: Position) -> Option<HitTarget> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| region.rect.contains(point))
            .map(|region| region.target)
    }

    pub(crate) fn hit_target_rect(&self, target: HitTarget) -> Option<Rect> {
        self.hit_regions
            .iter()
            .find(|region| region.target == target)
            .map(|region| region.rect)
    }

    pub(crate) fn clear_hit_targets_in(&mut self, rect: Rect) {
        self.hit_regions
            .retain(|region| region.rect.intersection(rect).is_empty());
    }
}

pub struct App {
    pub(crate) session: RepositorySession,
    pub view: View,
    pub(crate) graph_commit_open: bool,
    pub mode: Mode,
    pub changes: ChangesState,
    pub graph_state: TableState,
    pub(crate) graph_scroll_to_selection: bool,
    pub(crate) author_filter: AuthorFilter,
    pub(crate) commit_summaries: CommitSummaryCache,
    pub(crate) commit_input: TextInput,
    pub(crate) commit_scroll: Option<usize>,
    pub(crate) commit_message_generator: CommitMessageGenerator,
    commit_draft_path: Option<PathBuf>,
    commit_draft_due: Option<Instant>,
    commit_draft_rx: Option<Receiver<CommitDraftResult>>,
    pub dragging_splitter: bool,
    pub dragging_workspace_panel_splitter: bool,
    pub dragging_history: bool,
    pub dragging_diff_scrollbar: bool,
    diff_scroll_drag_offset: u16,
    pub workspace_explorer: Explorer,
    pub(crate) file_search: FileSearch,
    pub(crate) actions: ActionsState,
    pub(crate) herdr_prompt: HerdrPrompt,
    pub(crate) repository_browser: RepositoryBrowser,
    pub(crate) worktree_manager: WorktreeManager,
    pub(crate) workspace_panel: WorkspacePanel,
    pub(crate) hovered_hit_target: Option<HitTarget>,
    pub settings: Settings,
    pub settings_selection: usize,
    pub notice: Option<String>,
    pub regions: Regions,
    pub(crate) selection: SelectionState,
    copy_request: Option<String>,
    pub should_quit: bool,
    pub(crate) settings_store: SettingsStore,
    pending_reload: Option<(changes::ChangesSelection, Option<String>)>,
    reload_queued: Option<RefreshScope>,
    pub(crate) editor_input: String,
    pub(crate) editor_error: Option<String>,
    pub(crate) editor_configure_only: bool,
    editor_request: Option<EditorRequest>,
    pub(crate) file_dialog: Option<FileDialog>,
    file_drag: Option<FileDrag>,
    last_worktree_file_click: Option<(RepoPath, bool, Instant)>,
    pending_file_selection: Option<RepoPath>,
    workspace_focus_restore_path: Option<PathBuf>,
    pending_workspace_restore: Option<PathBuf>,
    recent_fetches: HashMap<PathBuf, Instant>,
    workspace_fetch_pending: bool,
}

pub(crate) struct EditorRequest {
    pub(crate) command: Vec<String>,
    pub(crate) file: PathBuf,
    pub(crate) repository: PathBuf,
}

impl App {
    #[cfg(test)]
    pub fn new(path: PathBuf) -> Self {
        Self::build(path, false)
    }

    pub fn opening(path: PathBuf) -> Self {
        Self::build(path, true)
    }

    fn build(path: PathBuf, open_in_background: bool) -> Self {
        let (settings_store, settings) = SettingsStore::discover();
        let workspace_config_dir = settings_store.config_dir();
        let workspace_groups_path =
            workspace_config_dir.map(|path| path.join("workspace-groups.json"));
        let workspace_snapshots_path =
            workspace_config_dir.map(|path| path.join("workspace-snapshots.json"));
        #[cfg(not(test))]
        let known_repositories_path =
            workspace_config_dir.map(|path| path.join("known-repositories.json"));
        #[cfg(test)]
        let known_repositories_path = None;
        let interval = settings.fetch_interval();
        let session = if open_in_background {
            RepositorySession::opening(path.clone(), interval)
        } else {
            RepositorySession::new(&path, interval)
        };
        let mode = if session.data().is_some() {
            Mode::Normal
        } else {
            Mode::Explorer
        };
        let start = session
            .data()
            .and_then(|repo| repo.root.parent().map(Path::to_path_buf))
            .unwrap_or(path);

        let changes = ChangesState::new(session.data());
        let file_search = FileSearch::new(
            session.data().map_or(&[], |repo| repo.files.as_slice()),
            session.data().map(|repo| repo.files_fingerprint),
        );
        let mut graph_state = TableState::default();
        graph_state.select(
            session
                .data()
                .is_some_and(|repo| !repo.commits.is_empty())
                .then_some(0),
        );
        let mut repository_browser = RepositoryBrowser::default();
        if let Some(repo) = session.data().filter(|repo| repo.github_remote) {
            repository_browser.prefetch(&repo.root);
        }
        let mut worktree_manager = WorktreeManager::new(known_repositories_path);
        if let Some(common_dir) = session.data().and_then(|repo| repo.common_dir.as_deref()) {
            let _ = worktree_manager.remember(common_dir);
        }
        let mut author_filter = AuthorFilter::default();
        if let Some(repo) = session.data() {
            author_filter.sync(&repo.root, &repo.commits);
        }
        let mut app = Self {
            session,
            view: View::Changes,
            graph_commit_open: false,
            mode,
            changes,
            graph_state,
            graph_scroll_to_selection: true,
            author_filter,
            commit_summaries: CommitSummaryCache::default(),
            commit_input: TextInput::default(),
            commit_scroll: None,
            commit_message_generator: CommitMessageGenerator::detect(),
            commit_draft_path: None,
            commit_draft_due: None,
            commit_draft_rx: None,
            dragging_splitter: false,
            dragging_workspace_panel_splitter: false,
            dragging_history: false,
            dragging_diff_scrollbar: false,
            diff_scroll_drag_offset: 0,
            workspace_explorer: Explorer::new(start),
            file_search,
            actions: ActionsState::default(),
            herdr_prompt: HerdrPrompt::default(),
            repository_browser,
            worktree_manager,
            workspace_panel: WorkspacePanel::detect(
                workspace_groups_path,
                workspace_snapshots_path,
            ),
            hovered_hit_target: None,
            settings,
            settings_selection: 0,
            notice: open_in_background.then(|| "Opening workspace…".to_owned()),
            regions: Regions::default(),
            selection: SelectionState::default(),
            copy_request: None,
            should_quit: false,
            settings_store,
            pending_reload: None,
            reload_queued: None,
            editor_input: String::new(),
            editor_error: None,
            editor_configure_only: false,
            editor_request: None,
            file_dialog: None,
            file_drag: None,
            last_worktree_file_click: None,
            pending_file_selection: None,
            workspace_focus_restore_path: None,
            pending_workspace_restore: None,
            recent_fetches: HashMap::new(),
            workspace_fetch_pending: false,
        };
        app.restore_commit_draft();
        app.show_graph_if_diff_empty();
        app
    }

    pub(crate) fn repository(&self) -> Option<&RepositoryData> {
        self.session.data()
    }

    pub(crate) fn diagnostic_context(&self) -> String {
        self.repository().map_or_else(
            || format!("mode={:?} workspace=none", self.mode),
            |repository| {
                format!(
                    "mode={:?} workspace={} kind={:?} files={} directories={} changes={}",
                    self.mode,
                    repository.root.display(),
                    repository.kind,
                    repository.files.len(),
                    repository.directories.len(),
                    repository.changes.len()
                )
            },
        )
    }

    pub(crate) fn visible_graph_indices(&self) -> &[usize] {
        if self.repository().is_some() {
            self.author_filter.visible_indices()
        } else {
            &[]
        }
    }

    pub(crate) fn selected_graph_commit(&self) -> Option<&git::Commit> {
        let selected = self.graph_state.selected()?;
        let index = *self.author_filter.visible_indices().get(selected)?;
        self.repository()?.commits.get(index)
    }

    fn visible_graph_len(&self) -> usize {
        self.repository()
            .map_or(0, |_| self.author_filter.visible_indices().len())
    }

    fn reconcile_graph_selection(&mut self) {
        let len = self.visible_graph_len();
        let selected = self
            .graph_state
            .selected()
            .map(|index| index.min(len.saturating_sub(1)))
            .or_else(|| (len > 0).then_some(0));
        self.graph_state
            .select((len > 0).then_some(selected.unwrap_or(0)));
        *self.graph_state.offset_mut() = self.graph_state.offset().min(len.saturating_sub(1));
        self.graph_scroll_to_selection = true;
        if len == 0 {
            self.graph_commit_open = false;
        }
    }

    fn git_repository(&self) -> Option<&RepositoryData> {
        self.repository().filter(|repo| !repo.is_local())
    }

    fn require_git_repository(&mut self) -> bool {
        if self.git_repository().is_some() {
            return true;
        }
        self.notice = Some(
            if self.repository().is_some() {
                "Not a Git repository"
            } else {
                "Open a repository first"
            }
            .to_owned(),
        );
        false
    }

    pub(crate) fn commit_running(&self) -> bool {
        self.session.commit_running()
    }

    pub(crate) fn commit_message_available(&self) -> bool {
        self.commit_message_generator.is_available()
    }

    pub(crate) fn commit_message_running(&self) -> bool {
        self.commit_message_generator.is_running()
    }

    pub(crate) fn fetch_running(&self) -> bool {
        self.session.fetch_running()
    }

    pub(crate) fn workspace_panel_enabled(&self) -> bool {
        self.settings.workspace_panel_enabled && self.workspace_panel.is_enabled()
    }

    pub(crate) fn workspace_panel_available(&self) -> bool {
        self.settings.workspace_panel_enabled && self.workspace_panel.is_available()
    }

    pub(crate) fn format_running(&self) -> bool {
        self.session.format_running()
    }

    pub(crate) fn can_restart(&self) -> bool {
        self.session.can_restart()
            && !self.commit_message_running()
            && !self.worktree_removal_running()
    }

    pub(crate) fn shutdown(&mut self) {
        self.changes.shutdown();
        self.commit_summaries.shutdown();
        self.workspace_explorer.shutdown();
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.selection.has_selection() {
            self.selection.clear();
            if key.code == KeyCode::Esc {
                return;
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::F(3) && self.mode == Mode::Normal {
            self.open_file_search();
            return;
        }
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Commit => self.handle_commit_input(key),
            Mode::FileSearch => self.handle_file_search(key),
            Mode::Explorer => self.handle_explorer(key),
            Mode::Settings => self.handle_settings(key),
            Mode::RepositoryBrowser => self.handle_repository_browser(key),
            Mode::WorktreeManager => self.handle_worktree_manager(key),
            Mode::AuthorFilter => self.handle_author_filter(key),
            Mode::ActionMenu => self.handle_action_menu(key),
            Mode::Command => self.handle_command(key),
            Mode::HerdrPrompt => self.handle_herdr_prompt(key),
            Mode::Editor => self.handle_editor(key),
            Mode::Files => self.handle_file_dialog(key),
            Mode::WorkspacePanel => self.handle_workspace_panel(key),
            Mode::WorkspacePresets => self.handle_workspace_presets(key),
            Mode::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                    self.mode = Mode::Normal;
                }
            }
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.mode == Mode::Normal && self.paste_clipboard_files(text) {
            return;
        }
        match self.mode {
            Mode::Commit => {
                self.commit_input.insert(text);
                self.schedule_commit_draft();
            }
            Mode::FileSearch => {
                if let Some(repo) = self.session.data() {
                    self.file_search.paste(text, &repo.files);
                }
            }
            Mode::Explorer => self.workspace_explorer.paste(text),
            Mode::Command if self.actions.status != CommandStatus::Running => {
                self.actions.input.push_str(text);
                if self.actions.status == CommandStatus::Input {
                    self.actions.stderr.clear();
                }
            }
            Mode::HerdrPrompt if !self.herdr_prompt.sending => {
                self.herdr_prompt.input.insert(text);
                self.herdr_prompt.error = None;
            }
            Mode::Editor => {
                self.editor_input.push_str(text);
                self.editor_error = None;
            }
            Mode::Files => {
                if let Some(dialog) = &mut self.file_dialog
                    && matches!(dialog.kind, FileDialogKind::Name { .. })
                {
                    dialog.input.insert(text);
                    dialog.error = None;
                }
            }
            Mode::RepositoryBrowser => self.repository_browser.paste(text),
            Mode::WorktreeManager => self.worktree_manager.paste(text),
            Mode::WorkspacePanel => self.workspace_panel.paste(text),
            Mode::WorkspacePresets => self.workspace_panel.paste(text),
            _ => {}
        }
    }

    pub fn take_copy_request(&mut self) -> Option<String> {
        self.copy_request.take()
    }

    pub fn poll_worker(&mut self) -> bool {
        let mut changed = self.mode == Mode::Explorer && self.workspace_explorer.poll_index();
        if self.workspace_panel_enabled()
            || (self.mode == Mode::WorktreeManager && self.workspace_panel.is_enabled())
            || self.workspace_panel.destructive_action_running()
        {
            let panel_poll = self.workspace_panel.poll();
            changed |= panel_poll.changed;
            if let Some(error) = panel_poll.notice {
                self.notice = Some(error);
            }
            if let Some(path) = panel_poll.reopen_path {
                diagnostics::event(format!(
                    "opening parent workspace after worktree removal path={}",
                    path.display()
                ));
                self.queue_workspace_restore(path);
            }
            if panel_poll.workspace_focus_succeeded
                && let Some(path) = self.workspace_focus_restore_path.take()
            {
                self.queue_workspace_restore(path);
                changed = true;
            }
            if self.mode == Mode::WorktreeManager {
                let candidates_changed = self.worktree_manager.update_herdr_inventory(
                    self.workspace_panel.worktree_candidates(),
                    self.workspace_panel.linked_herdr_worktrees(),
                    self.workspace_panel.worktree_inventory_verified(),
                );
                if candidates_changed {
                    self.worktree_manager.start_refresh();
                }
            }
        }
        changed |= self.repository_browser.poll();
        let worktree_poll = self.worktree_manager.poll();
        changed |= worktree_poll.changed;
        if let Some(notice) = worktree_poll.notice {
            self.notice = Some(notice);
        }
        if let Some(path) = worktree_poll.open_path {
            self.mode = Mode::Normal;
            self.queue_workspace_restore(path);
        }
        self.prefetch_commit_summaries();
        changed |= self.commit_summaries.poll();
        changed |= self.commit_input.poll_blink(self.mode == Mode::Commit);
        if let Some(done) = self
            .commit_draft_rx
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.commit_draft_rx = None;
            if self
                .repository()
                .is_some_and(|repo| same_path(&repo.root, &done.root))
            {
                changed = true;
                match done.result {
                    Ok((path, message)) => {
                        self.commit_draft_path = Some(path);
                        if self.commit_draft_due.is_none()
                            && self.commit_input.is_empty()
                            && let Some(message) = message
                        {
                            self.commit_input.set(message);
                        }
                    }
                    Err(error) => {
                        self.commit_draft_due = None;
                        self.notice = Some(format!("Could not load commit draft: {error}"));
                    }
                }
            }
        }
        changed |= self
            .herdr_prompt
            .input
            .poll_blink(self.mode == Mode::HerdrPrompt && !self.herdr_prompt.sending);
        if let Some(completion) = self.herdr_prompt.poll() {
            changed = true;
            match completion {
                Ok(pane_id) => {
                    if self.mode == Mode::HerdrPrompt {
                        self.mode = Mode::Normal;
                    }
                    self.notice = Some(format!("Sent to Herdr pane {pane_id}"));
                }
                Err(error) if self.mode == Mode::HerdrPrompt => {
                    self.herdr_prompt.error = Some(error);
                }
                Err(error) => self.notice = Some(error),
            }
        }
        changed |= self.workspace_panel.snapshot_input.poll_blink(
            self.mode == Mode::WorkspacePresets && self.workspace_panel.snapshot_editing,
        );
        if let Some(dialog) = &mut self.workspace_panel.rename_dialog {
            changed |= dialog.input.poll_blink(self.mode == Mode::WorkspacePanel);
        }
        if let Some(completion) = self.commit_message_generator.poll() {
            changed = true;
            if !self
                .repository()
                .is_some_and(|repo| same_path(&repo.root, &completion.root))
            {
                self.notice = Some(
                    "Generated commit message ignored because the workspace changed".to_owned(),
                );
            } else if self.commit_input.text() != completion.baseline {
                self.notice = Some(
                    "Generated commit message ignored because the message was edited".to_owned(),
                );
            } else {
                match completion.result {
                    Ok(message) => {
                        self.commit_input.set(message);
                        self.commit_scroll = None;
                        self.commit_input.focus();
                        self.mode = Mode::Commit;
                        self.schedule_commit_draft();
                        self.notice = Some("Commit message generated with OpenCode".to_owned());
                    }
                    Err(error) => self.notice = Some(error),
                }
            }
        }
        changed |= self.flush_commit_draft_if_due();
        if let Some(dialog) = &mut self.file_dialog {
            changed |= dialog.input.poll_blink(
                self.mode == Mode::Files && matches!(dialog.kind, FileDialogKind::Name { .. }),
            );
        }
        let interval = self.settings.fetch_interval();
        self.session
            .maybe_start_fetch(self.settings.auto_fetch, interval);
        self.session.maybe_start_status_check();
        while let Some(done) = self.session.next_worker_completion(interval) {
            changed = true;
            let invalidation = done.invalidation();
            if let Some(scope) = invalidation {
                self.reload(scope);
            }
            match done.outcome {
                WorkerOutcome::Commit(result) => match result {
                    Ok(output) if output.success => {
                        self.commit_input.clear();
                        self.commit_scroll = None;
                        self.schedule_commit_draft();
                        self.flush_commit_draft();
                        self.notice = Some("Commit created".to_owned());
                    }
                    Ok(output) => {
                        self.notice = Some(first_error(&output.stderr, "Commit failed"));
                    }
                    Err(error) => self.notice = Some(error),
                },
                WorkerOutcome::Fetch(result) => match result {
                    Ok(output) if output.success => {
                        if let Some(root) = self.session.data().map(|repo| repo.root.clone()) {
                            self.recent_fetches.insert(root, Instant::now());
                        }
                        self.notice = Some("Fetched remotes".to_owned());
                    }
                    Ok(output) => {
                        self.notice = Some(first_error(&output.stderr, "Fetch failed"));
                    }
                    Err(error) => self.notice = Some(error),
                },
                WorkerOutcome::Command(done) => match done.result {
                    Ok(output) => {
                        let success = output.success;
                        let error = first_error(&output.stderr, "Git command failed");
                        self.actions.complete(output);
                        self.notice = Some(if success {
                            format!("{} complete", done.label)
                        } else {
                            error
                        });
                    }
                    Err(error) => {
                        self.actions.fail(error.clone());
                        self.notice = Some(error);
                    }
                },
                WorkerOutcome::Mutation(result) => match result {
                    Ok(()) => {}
                    Err(error) => {
                        self.changes.cancel_pending_hunk_stage();
                        self.notice = Some(error);
                    }
                },
                WorkerOutcome::FileOperation(done) => match done.result {
                    Ok(selection) => {
                        self.pending_file_selection = selection;
                        self.notice = Some(done.message);
                    }
                    Err(error) => self.notice = Some(error),
                },
                WorkerOutcome::DiscardUnstaged(done) => {
                    self.notice = Some(match done.result {
                        Ok(()) => format!("Discarded unstaged changes to {}", done.path),
                        Err(error) => error,
                    });
                }
                WorkerOutcome::Format(done) => match done.result {
                    Ok(output) if output.success => {
                        self.notice =
                            Some(format!("Formatted {} with {}", done.path, done.formatter));
                    }
                    Ok(output) => {
                        let fallback = format!("{} could not format {}", done.formatter, done.path);
                        self.notice = Some(if output.stderr.trim().is_empty() {
                            first_error(&output.stdout, &fallback)
                        } else {
                            first_error(&output.stderr, &fallback)
                        });
                    }
                    Err(error) => self.notice = Some(error),
                },
                WorkerOutcome::BranchDelete(done) => {
                    self.notice = Some(match done.result {
                        Ok(()) => done.remote.map_or_else(
                            || {
                                format!(
                                    "{} local branch {}",
                                    if done.force {
                                        "Force deleted"
                                    } else {
                                        "Deleted"
                                    },
                                    done.branch
                                )
                            },
                            |(remote, remote_branch)| {
                                format!(
                                    "{} {} locally and {remote}/{remote_branch}",
                                    if done.force {
                                        "Force deleted"
                                    } else {
                                        "Deleted"
                                    },
                                    done.branch
                                )
                            },
                        ),
                        Err(error) => error,
                    });
                }
            }
        }
        while let Some(scope) = self.session.next_worktree_change() {
            changed = true;
            self.reload(scope);
            self.notice = None;
        }
        if self.session.open_running() {
            self.notice = Some("Opening workspace…".to_owned());
        }
        while let Some(done) = self.session.next_load_completion() {
            changed = true;
            let prepared_file_tree = done.prepared_file_tree;
            match (done.kind, done.result) {
                (LoadKind::Open, Ok(())) => {
                    let _activity =
                        diagnostics::activity("apply-workspace", self.diagnostic_context());
                    diagnostics::event(format!("workspace opened {}", self.diagnostic_context()));
                    self.pending_reload = None;
                    self.reload_queued = None;
                    if self.mode != Mode::WorkspacePanel {
                        self.mode = Mode::Normal;
                    }
                    self.actions = ActionsState::default();
                    self.notice = Some(
                        if self.session.data().is_some_and(RepositoryData::is_local) {
                            "Workspace opened"
                        } else {
                            "Repository opened"
                        }
                        .to_owned(),
                    );
                    self.graph_state = TableState::default();
                    self.graph_scroll_to_selection = true;
                    if let Some(repo) = self.session.data() {
                        self.author_filter.sync(&repo.root, &repo.commits);
                    }
                    self.changes
                        .reset_repository(self.session.data(), prepared_file_tree);
                    if let Some(path) = self.pending_file_selection.take()
                        && let Some(repo) = self.session.data()
                    {
                        self.view = View::Changes;
                        self.changes.set_pane(LeftPane::Files, Some(repo));
                        let viewport = self
                            .regions
                            .explorer_list
                            .map_or(0, |rect| usize::from(rect.height));
                        self.changes.select_explorer_path(repo, &path, viewport);
                    }
                    self.file_search.invalidate();
                    self.graph_state.select(
                        self.session
                            .data()
                            .is_some_and(|repo| !repo.commits.is_empty())
                            .then_some(0),
                    );
                    self.restore_commit_draft();
                    self.show_graph_if_diff_empty();
                    self.prefetch_repository_browser();
                    if let Some(common_dir) = self
                        .session
                        .data()
                        .and_then(|repository| repository.common_dir.as_deref())
                        && let Err(error) = self.worktree_manager.remember(common_dir)
                    {
                        self.notice = Some(error);
                    }
                }
                (LoadKind::Open, Err(error)) => {
                    diagnostics::event(format!("workspace open failed error={error}"));
                    self.workspace_fetch_pending = false;
                    self.pending_file_selection = None;
                    let message = format!("Could not open workspace: {error}");
                    self.notice = Some(message.clone());
                    self.workspace_explorer.error = Some(message);
                }
                (LoadKind::Reload, Ok(())) => {
                    if let Some((selection, selected_oid)) = self.pending_reload.take() {
                        let repo = self.session.data().expect("reloaded repository");
                        self.author_filter.sync(&repo.root, &repo.commits);
                        let visible = self.author_filter.visible_indices();
                        let commit_index = selected_oid.and_then(|oid| {
                            visible
                                .iter()
                                .position(|index| repo.commits[*index].oid == oid)
                        });
                        self.graph_state
                            .select(commit_index.or_else(|| repo.commits.first().map(|_| 0)));
                        self.graph_scroll_to_selection = true;
                        self.changes.restore_selection(repo, selection);
                        if let Some(path) = self.pending_file_selection.take() {
                            let viewport = self
                                .regions
                                .explorer_list
                                .map_or(0, |rect| usize::from(rect.height));
                            self.changes.select_explorer_path(repo, &path, viewport);
                        }
                    }
                    if let Some(repo) = self.session.data() {
                        if self.mode == Mode::FileSearch {
                            self.file_search
                                .reindex(&repo.files, Some(repo.files_fingerprint));
                        } else {
                            self.file_search.invalidate();
                        }
                        if self.mode == Mode::RepositoryBrowser {
                            self.repository_browser.sync_branches(&repo.branches);
                        }
                    }
                    self.show_graph_if_diff_empty();
                    self.prefetch_repository_browser();
                    if self.notice.as_deref() == Some("Refreshing…")
                        || self
                            .notice
                            .as_deref()
                            .is_some_and(|notice| notice.ends_with(" (retrying queued refresh…)"))
                    {
                        self.notice = Some("Refreshed".to_owned());
                    }
                    if let Some(scope) = self.reload_queued.take() {
                        self.reload(scope);
                    }
                }
                (LoadKind::Reload, Err(error)) => {
                    self.pending_reload = None;
                    let queued = self.reload_queued.take();
                    if let Some(scope) = queued {
                        self.reload(scope);
                        self.notice = Some(format!("{error} (retrying queued refresh…)"));
                    } else {
                        self.notice = Some(error);
                    }
                }
            }
        }
        self.try_start_workspace_restore();
        self.maybe_start_workspace_fetch();
        changed |= self.changes.poll_directories(self.session.data());
        changed |= self
            .changes
            .poll_preview(self.session.data().map(|repo| repo.root.as_path()));
        changed |= self.changes.preview_presentation.poll_media();
        changed
    }

    pub(crate) fn reset_media_presentation(&mut self) {
        self.changes.preview_presentation.hide_media();
    }

    pub(crate) fn take_media_terminal_cleanup(&mut self) -> Vec<u8> {
        self.changes.preview_presentation.take_terminal_cleanup()
    }

    pub(crate) fn take_media_terminal_output(&mut self) -> crate::ui::preview::MediaTerminalOutput {
        self.changes.preview_presentation.take_terminal_output()
    }

    pub(crate) fn media_terminal_restarted(&mut self) {
        self.changes.preview_presentation.terminal_restarted();
    }

    pub(crate) fn configure_media_picker(
        &mut self,
        picker: ratatui_image::picker::Picker,
        allow_auto_kitty: bool,
    ) {
        self.changes
            .preview_presentation
            .configure_media_picker(picker, allow_auto_kitty);
    }

    fn prefetch_commit_summaries(&mut self) {
        let Some(repo) = self.session.data().filter(|repo| !repo.is_local()) else {
            return;
        };
        let mut oids = Vec::new();
        if self.view == View::Graph {
            let viewport = self
                .regions
                .graph_table
                .map_or(40, |region| usize::from(region.height));
            let visible = self.author_filter.visible_indices();
            oids.extend(
                visible
                    .iter()
                    .skip(self.graph_state.offset())
                    .take(viewport)
                    .map(|index| repo.commits[*index].oid.clone()),
            );
        }
        if self.changes.history_focused
            && let Some(commit) = self
                .changes
                .history_state
                .selected()
                .and_then(|index| repo.history.get(index))
        {
            oids.push(commit.oid.clone());
        }
        if oids.is_empty() {
            return;
        }
        let root = repo.root.clone();
        self.commit_summaries
            .request(&root, oids.iter().map(String::as_str));
    }

    pub fn requires_render_before_next_event(&self) -> bool {
        self.editor_request.is_some()
            || self.changes.hunk_selection.is_some()
            || self
                .regions
                .screen
                .is_some_and(|area| self.selection.needs_capture(area))
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Tab)
            && self.view == View::Graph
            && self.graph_commit_open
        {
            self.graph_commit_open = false;
            return;
        }
        if key.code == KeyCode::Char('f') && self.view == View::Graph {
            self.toggle_changes_files();
            return;
        }
        if key.code == KeyCode::Delete
            && key.modifiers.is_empty()
            && self.changes.pane == LeftPane::Worktree
        {
            self.open_discard_unstaged_dialog();
            return;
        }
        if let Some(index) = self.changes.hunk_selection {
            match key.code {
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => {
                    self.changes.leave_hunk_selection();
                }
                KeyCode::Down | KeyCode::Char('j') => self.scroll_or_move_hunk(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll_or_move_hunk(-1),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                    self.stage_hunk(index, true);
                }
                _ => {}
            }
            return;
        }
        if self.view == View::Changes
            && self.changes.pane == LeftPane::Files
            && self.changes.sqlite_active()
        {
            self.handle_sqlite_browser_key(key);
            return;
        }
        match key.code {
            KeyCode::F(1) => self.open_herdr_prompt(),
            KeyCode::F(3) => self.open_file_search(),
            KeyCode::Char('s')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.changes.pane == LeftPane::Files =>
            {
                self.format_selected_file();
            }
            KeyCode::Char('q') if self.format_running() => {
                self.notice = Some("A formatter is still running".to_owned())
            }
            KeyCode::Char('q') if self.commit_running() || self.session.command_running() => {
                self.notice = Some("A Git operation is still running".to_owned())
            }
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('w')
                if key.modifiers == KeyModifiers::NONE && self.workspace_panel_enabled() =>
            {
                self.cycle_workspace_panel();
            }
            KeyCode::Char('p')
                if key.modifiers == KeyModifiers::NONE && self.workspace_panel_enabled() =>
            {
                self.open_workspace_presets();
            }
            KeyCode::Char('1') => {
                self.view = View::Changes;
                self.graph_commit_open = false;
                self.show_graph_if_diff_empty();
            }
            KeyCode::Char('2') => {
                self.view = View::Graph;
                self.graph_commit_open = false;
            }
            KeyCode::Tab => {
                self.view = match self.view {
                    View::Changes => View::Graph,
                    View::Graph => View::Changes,
                };
                self.graph_commit_open = false;
                self.show_graph_if_diff_empty();
            }
            KeyCode::Char('r') => self.reload(RefreshScope::ALL),
            KeyCode::Char('o') => self.open_explorer(),
            KeyCode::Char('W') => self.open_worktree_manager(),
            KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => self.mode = Mode::Settings,
            KeyCode::Char('b') => self.open_repository_browser(),
            KeyCode::Char('x') => self.open_actions(),
            KeyCode::Char('g') => self.open_git_command(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('w')
                if key.modifiers == KeyModifiers::ALT
                    && (self.view == View::Changes || self.graph_commit_open) =>
            {
                let wrapped = self.changes.toggle_wrap();
                let subject = if self.view == View::Changes && self.changes.pane == LeftPane::Files
                {
                    "Preview"
                } else {
                    "Diff"
                };
                self.notice = Some(if wrapped {
                    format!("{subject} wrap enabled")
                } else {
                    format!("{subject} wrap disabled")
                });
            }
            KeyCode::Char('m') => self.toggle_markdown_preview(),
            KeyCode::F(2) if self.changes.pane == LeftPane::Files => {
                self.open_rename_dialog();
            }
            KeyCode::Delete
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.changes.pane == LeftPane::Files =>
            {
                self.open_delete_dialog();
            }
            KeyCode::Char('e') => self.open_selected_file(false),
            KeyCode::Char('E') => self.open_selected_file(true),
            KeyCode::Char('f') if self.view == View::Changes => self.toggle_changes_files(),
            KeyCode::Char('c') => {
                self.set_left_pane(LeftPane::Worktree);
                self.focus_commit();
            }
            KeyCode::Char('a') if self.changes.pane == LeftPane::Worktree => {
                self.stage_all();
            }
            KeyCode::Char('u') if self.changes.pane == LeftPane::Worktree => {
                self.unstage_all();
            }
            KeyCode::Char(' ')
                if self.changes.pane == LeftPane::Worktree && !self.changes.history_focused =>
            {
                self.toggle_stage()
            }
            KeyCode::Enter if self.view == View::Graph && !self.graph_commit_open => {
                self.open_selected_graph_commit();
            }
            KeyCode::Enter
                if self.view == View::Changes
                    && self.changes.pane == LeftPane::Files
                    && self.changes.activate_sqlite() => {}
            KeyCode::Enter
                if self.view == View::Changes && self.changes.pane == LeftPane::Files =>
            {
                let repo = self.session.data();
                self.changes.toggle_selected_explorer_directory(repo);
            }
            KeyCode::Enter
                if self.view == View::Changes
                    && self.changes.pane == LeftPane::Worktree
                    && !self.changes.history_focused =>
            {
                let repo = self.session.data();
                self.changes.toggle_selected_directory(repo);
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.view == View::Changes && self.changes.pane == LeftPane::Files =>
            {
                let repo = self.session.data();
                self.changes.expand_or_descend_explorer(repo);
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.view == View::Changes
                    && self.changes.pane == LeftPane::Worktree
                    && !self.changes.history_focused =>
            {
                let invalid_path = self.session.data().and_then(|repo| {
                    let index = self.changes.selected_change_index(repo)?;
                    (!repo.changes.get(index)?.path.is_utf8()).then_some(())
                });
                if invalid_path.is_some() {
                    self.notice = Some(
                        "Hunk actions are unavailable for paths that are not valid UTF-8"
                            .to_owned(),
                    );
                } else {
                    let repo = self.session.data();
                    if !repo.is_some_and(|repo| self.changes.enter_hunk_selection(repo)) {
                        self.changes.expand_or_descend_worktree(repo);
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h')
                if self.view == View::Changes && self.changes.pane == LeftPane::Files =>
            {
                let repo = self.session.data();
                self.changes.collapse_or_ascend_explorer(repo);
            }
            KeyCode::Left | KeyCode::Char('h')
                if self.view == View::Changes
                    && self.changes.pane == LeftPane::Worktree
                    && !self.changes.history_focused =>
            {
                let repo = self.session.data();
                self.changes.collapse_or_ascend_worktree(repo);
            }
            KeyCode::PageDown if self.view == View::Changes || self.graph_commit_open => {
                self.scroll_diff_by(10)
            }
            KeyCode::PageUp if self.view == View::Changes || self.graph_commit_open => {
                self.scroll_diff_by(-10)
            }
            KeyCode::Down | KeyCode::Char('j') if self.graph_commit_open => self.scroll_diff_by(1),
            KeyCode::Up | KeyCode::Char('k') if self.graph_commit_open => self.scroll_diff_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Home => self.select_first(),
            KeyCode::End | KeyCode::Char('G') => self.select_last(),
            _ => {}
        }
    }

    fn handle_sqlite_browser_key(&mut self, key: KeyEvent) {
        let object_viewport = self
            .regions
            .sqlite_objects
            .map_or(0, |area| usize::from(area.height));
        let row_viewport = self
            .regions
            .sqlite_rows
            .map_or(0, |area| usize::from(area.height));
        let focus = self
            .changes
            .sqlite_browser
            .as_ref()
            .map(|browser| browser.focus);
        match key.code {
            KeyCode::Esc => self.changes.deactivate_sqlite(),
            KeyCode::Tab | KeyCode::BackTab => self.changes.toggle_sqlite_focus(),
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(repo) = self.session.data() {
                    self.changes
                        .move_sqlite_selection(repo, 1, object_viewport, row_viewport);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(repo) = self.session.data() {
                    self.changes
                        .move_sqlite_selection(repo, -1, object_viewport, row_viewport);
                }
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => {
                if focus == Some(SqliteFocus::Objects) {
                    self.changes.focus_sqlite_rows();
                } else {
                    self.changes.shift_sqlite_columns(1);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if focus == Some(SqliteFocus::Rows) {
                    self.changes.shift_sqlite_columns(-1);
                } else {
                    self.changes.focus_sqlite_objects();
                }
            }
            KeyCode::PageDown => {
                if let Some(repo) = self.session.data() {
                    self.changes.page_sqlite(repo, 1);
                }
            }
            KeyCode::PageUp => {
                if let Some(repo) = self.session.data() {
                    self.changes.page_sqlite(repo, -1);
                }
            }
            KeyCode::Home => {
                if let Some(repo) = self.session.data() {
                    self.changes
                        .select_sqlite_boundary(repo, false, object_viewport, row_viewport);
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if let Some(repo) = self.session.data() {
                    self.changes
                        .select_sqlite_boundary(repo, true, object_viewport, row_viewport);
                }
            }
            _ => {}
        }
    }

    fn scroll_or_move_hunk(&mut self, delta: isize) {
        let Some(selected) = self.changes.hunk_selection else {
            return;
        };
        let Some(region) = self
            .regions
            .diff_hunks
            .iter()
            .find(|region| region.index == selected)
        else {
            self.changes.move_hunk_selection(delta);
            return;
        };
        let can_scroll = if delta > 0 {
            region.continues_below
        } else {
            region.continues_above
        };
        if can_scroll {
            if delta > 0 {
                self.changes.diff_scroll = self
                    .changes
                    .diff_scroll
                    .saturating_add(10)
                    .min(region.scroll_end);
            } else {
                self.changes.diff_scroll = self
                    .changes
                    .diff_scroll
                    .saturating_sub(10)
                    .max(region.scroll_start);
            }
        } else {
            self.changes.move_hunk_selection(delta);
        }
    }

    fn handle_commit_input(&mut self, key: KeyEvent) {
        self.commit_input.focus();
        self.commit_scroll = None;
        let previous = self.commit_input.text().to_owned();
        let input_width = self
            .regions
            .commit
            .map_or(1, |area| usize::from(area.width.saturating_sub(2)).max(1));
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.flush_commit_draft();
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_commit();
            }
            KeyCode::Char('j' | 'm') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_commit();
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.commit_input.select_all();
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.commit_input.delete_word();
            }
            KeyCode::Left => self.commit_input.move_left(),
            KeyCode::Right => self.commit_input.move_right(),
            KeyCode::Up => self.commit_input.move_up(input_width),
            KeyCode::Down => self.commit_input.move_down(input_width),
            KeyCode::Home => self.commit_input.move_home(),
            KeyCode::End => self.commit_input.move_end(),
            KeyCode::Delete => self.commit_input.delete(),
            KeyCode::Enter => self.commit_input.insert_char('\n'),
            KeyCode::Backspace => self.commit_input.backspace(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.commit_input.insert_char(character);
            }
            _ => {}
        }
        if self.commit_input.text() != previous {
            self.schedule_commit_draft();
        }
    }

    fn handle_explorer(&mut self, key: KeyEvent) {
        let command = self
            .workspace_explorer
            .handle_key(key, self.repository().is_some());
        self.apply_explorer_command(command);
    }

    fn handle_file_search(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::F(3) => self.mode = Mode::Normal,
            KeyCode::Enter => self.activate_file_search_result(),
            KeyCode::Down | KeyCode::Tab => self.file_search.move_selection(1),
            KeyCode::Up | KeyCode::BackTab => self.file_search.move_selection(-1),
            KeyCode::Backspace => {
                if let Some(repo) = self.session.data() {
                    self.file_search.backspace(&repo.files);
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.clear(&repo.files);
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(repo) = self.session.data() {
                    self.file_search.push(character, &repo.files);
                }
            }
            _ => {}
        }
    }

    fn handle_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('s') => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.settings_selection = (self.settings_selection + 1) % 6;
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.settings_selection = (self.settings_selection + 5) % 6;
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 0 => {
                self.toggle_auto_fetch();
            }
            KeyCode::Left | KeyCode::Char('-') if self.settings_selection == 1 => {
                self.change_fetch_interval(-1);
            }
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=')
                if self.settings_selection == 1 =>
            {
                self.change_fetch_interval(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 2 => {
                self.toggle_workspace_panel_enabled();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 3 => {
                self.toggle_agent_harness();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 4 => {
                self.toggle_media_preview_protocol();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 5 => {
                self.open_editor_setting();
            }
            _ => {}
        }
    }

    fn handle_action_menu(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('x') => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.actions.move_selection(1);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.actions.move_selection(-1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_action(),
            _ => {}
        }
    }

    fn handle_command(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.mode = Mode::Normal;
            return;
        }
        if self.actions.status != CommandStatus::Running {
            match key.code {
                KeyCode::Enter => {
                    let input = if self.actions.input.trim().is_empty()
                        && matches!(self.actions.status, CommandStatus::Complete { .. })
                    {
                        self.actions.command.clone()
                    } else {
                        self.actions.input.clone()
                    };
                    match parse_git_args(&input) {
                        Ok(args) => self.start_git_command("Git command".to_owned(), args),
                        Err(error) => {
                            self.actions.status = CommandStatus::Input;
                            self.actions.stderr = error;
                        }
                    }
                }
                KeyCode::Down if matches!(self.actions.status, CommandStatus::Complete { .. }) => {
                    self.actions.scroll_by(1);
                }
                KeyCode::Up if matches!(self.actions.status, CommandStatus::Complete { .. }) => {
                    self.actions.scroll_by(-1);
                }
                KeyCode::PageDown
                    if matches!(self.actions.status, CommandStatus::Complete { .. }) =>
                {
                    self.actions.scroll_by(10);
                }
                KeyCode::PageUp
                    if matches!(self.actions.status, CommandStatus::Complete { .. }) =>
                {
                    self.actions.scroll_by(-10);
                }
                KeyCode::Backspace => {
                    self.actions.input.pop();
                    if self.actions.status == CommandStatus::Input {
                        self.actions.stderr.clear();
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.clear();
                    if self.actions.status == CommandStatus::Input {
                        self.actions.stderr.clear();
                    }
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.push(character);
                    if self.actions.status == CommandStatus::Input {
                        self.actions.stderr.clear();
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.actions.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.actions.scroll_by(-1),
            KeyCode::PageDown => self.actions.scroll_by(10),
            KeyCode::PageUp => self.actions.scroll_by(-10),
            KeyCode::Home | KeyCode::Char('g') => self.actions.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.actions.scroll = self.actions.scroll_max,
            _ => {}
        }
    }

    fn handle_herdr_prompt(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::F(1)) {
            self.mode = Mode::Normal;
            return;
        }
        if self.herdr_prompt.sending {
            return;
        }

        match key.code {
            KeyCode::Enter => self.herdr_prompt.submit(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.herdr_prompt.input.select_all();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.herdr_prompt.input.clear();
                self.herdr_prompt.error = None;
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.herdr_prompt.input.delete_word();
                self.herdr_prompt.error = None;
            }
            KeyCode::Left => self.herdr_prompt.input.move_left(),
            KeyCode::Right => self.herdr_prompt.input.move_right(),
            KeyCode::Home => self.herdr_prompt.input.move_home(),
            KeyCode::End => self.herdr_prompt.input.move_end(),
            KeyCode::Delete => {
                self.herdr_prompt.input.delete();
                self.herdr_prompt.error = None;
            }
            KeyCode::Backspace => {
                self.herdr_prompt.input.backspace();
                self.herdr_prompt.error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.herdr_prompt.input.insert_char(character);
                self.herdr_prompt.error = None;
            }
            _ => {}
        }
    }

    fn handle_editor(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.editor_error = None;
                self.mode = if self.editor_configure_only {
                    Mode::Settings
                } else {
                    Mode::Normal
                };
            }
            KeyCode::Enter => self.queue_editor(),
            KeyCode::Backspace => {
                self.editor_input.pop();
                self.editor_error = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.editor_input.clear();
                self.editor_error = None;
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.editor_input.push(character);
                self.editor_error = None;
            }
            _ => {}
        }
    }

    fn open_selected_file(&mut self, configure: bool) {
        let Some((repository, file)) = self.selected_file_to_edit() else {
            self.notice = Some("Select a file to edit".to_owned());
            return;
        };
        let configured = self.settings.editor_command.clone();
        if configure || configured.is_none() {
            self.editor_configure_only = false;
            self.editor_input = configured
                .or_else(|| std::env::var("VISUAL").ok())
                .or_else(|| std::env::var("EDITOR").ok())
                .unwrap_or_default();
            self.editor_error = None;
            self.mode = Mode::Editor;
            return;
        }
        self.prepare_editor_request(configured.expect("checked above"), repository, file);
    }

    fn open_selected_graph_commit(&mut self) {
        let Some(commit) = self.selected_graph_commit().cloned() else {
            return;
        };
        let Some(repo) = self.session.data() else {
            return;
        };
        self.changes.clear_history_selection();
        self.changes.preview_commit(repo, &commit);
        self.graph_commit_open = true;
    }

    fn open_browser_branch(&mut self, oid: &str) {
        let Some((author, actual_index)) = self.repository().and_then(|repo| {
            repo.commits
                .iter()
                .position(|commit| commit.oid.starts_with(oid))
                .map(|index| (repo.commits[index].author.clone(), index))
        }) else {
            self.mode = Mode::Normal;
            self.notice = Some("Branch tip is outside the loaded graph".to_owned());
            return;
        };
        self.author_filter.ensure_enabled(&author);
        let Some(index) = self
            .visible_graph_indices()
            .iter()
            .position(|index| *index == actual_index)
        else {
            return;
        };
        self.graph_state.select(Some(index));
        *self.graph_state.offset_mut() = index.saturating_sub(5);
        self.graph_scroll_to_selection = false;
        self.graph_commit_open = false;
        self.view = View::Graph;
        self.mode = Mode::Normal;
    }

    fn apply_repository_browser_effect_option(&mut self, effect: Option<RepositoryBrowserEffect>) {
        if let Some(effect) = effect {
            self.apply_repository_browser_effect(effect);
        }
    }

    fn apply_repository_browser_effect(&mut self, effect: RepositoryBrowserEffect) {
        match effect {
            RepositoryBrowserEffect::Close => self.mode = Mode::Normal,
            RepositoryBrowserEffect::OpenBranch(oid) => self.open_browser_branch(&oid),
            RepositoryBrowserEffect::DeleteBranch {
                branch,
                remote,
                force,
            } => {
                if self.session.start_branch_delete(branch, remote, force) {
                    self.notice = Some(if force {
                        "Force deleting branch…".to_owned()
                    } else {
                        "Deleting branch…".to_owned()
                    });
                } else {
                    self.notice = Some("Another repository operation is running".to_owned());
                }
            }
            RepositoryBrowserEffect::Notice(notice) => self.notice = Some(notice),
        }
    }

    fn queue_editor(&mut self) {
        if self.editor_configure_only {
            let command = self.editor_input.trim().to_owned();
            match parse_command_args(&command) {
                Ok(_) => {
                    self.settings.editor_command = Some(command);
                    self.persist_settings();
                    self.editor_error = None;
                    self.mode = Mode::Settings;
                }
                Err(error) => self.editor_error = Some(error),
            }
            return;
        }
        let Some((repository, file)) = self.selected_file_to_edit() else {
            self.mode = Mode::Normal;
            self.notice = Some("Select a file to edit".to_owned());
            return;
        };
        self.prepare_editor_request(self.editor_input.trim().to_owned(), repository, file);
    }

    fn open_editor_setting(&mut self) {
        self.editor_input = self
            .settings
            .editor_command
            .clone()
            .or_else(|| std::env::var("VISUAL").ok())
            .or_else(|| std::env::var("EDITOR").ok())
            .unwrap_or_default();
        self.editor_error = None;
        self.editor_configure_only = true;
        self.mode = Mode::Editor;
    }

    fn prepare_editor_request(&mut self, command: String, repository: PathBuf, file: PathBuf) {
        match parse_command_args(&command) {
            Ok(command_args) => {
                self.settings.editor_command = Some(command);
                self.persist_settings();
                self.editor_error = None;
                self.editor_request = Some(EditorRequest {
                    command: command_args,
                    file: repository.join(file),
                    repository,
                });
                self.mode = Mode::Normal;
            }
            Err(error) => {
                self.editor_error = Some(error);
                self.mode = Mode::Editor;
            }
        }
    }

    fn selected_file_to_edit(&self) -> Option<(PathBuf, PathBuf)> {
        if self.changes.history_focused {
            return None;
        }
        let repo = self.repository()?;
        let path = match self.changes.pane {
            LeftPane::Worktree => {
                let index = self.changes.selected_change_index(repo)?;
                repo.changes.get(index)?.path.as_path()
            }
            LeftPane::Files => self.changes.selected_explorer_file_path(repo)?.as_path(),
        };
        Some((repo.root.clone(), PathBuf::from(path)))
    }

    pub(crate) fn can_edit_selected_file(&self) -> bool {
        self.mode == Mode::Normal
            && self.view == View::Changes
            && self.changes.hunk_selection.is_none()
            && self.selected_file_to_edit().is_some()
    }

    fn format_selected_file(&mut self) {
        let Some(repo) = self.repository() else {
            self.notice = Some("Open a workspace first".to_owned());
            return;
        };
        let Some(path) = self.changes.selected_explorer_file_path(repo).cloned() else {
            self.notice = Some("Select a file to format".to_owned());
            return;
        };
        let root = repo.root.clone();
        let command = match formatter::detect(&root, path.as_path()) {
            Ok(command) => command,
            Err(error) => {
                self.notice = Some(error.to_string());
                return;
            }
        };
        let label = command.label;
        if self.session.start_format(path.clone(), command) {
            self.notice = Some(format!("Formatting {path} with {label}…"));
        } else {
            self.notice = Some("Another repository operation is still running".to_owned());
        }
    }

    pub(crate) fn take_editor_request(&mut self) -> Option<EditorRequest> {
        self.editor_request.take()
    }

    pub(crate) fn editor_finished(&mut self, result: Result<(), String>) {
        let error = result.err();
        self.reload(RefreshScope::WORKTREE);
        if let Some(error) = error {
            self.notice = Some(error);
        }
    }

    fn open_actions(&mut self) {
        if self.require_git_repository() {
            self.mode = Mode::ActionMenu;
        }
    }

    fn open_herdr_prompt(&mut self) {
        if !self.workspace_panel.is_enabled() {
            self.notice = Some("Herdr command prompt is only available inside Herdr".to_owned());
            return;
        }
        self.herdr_prompt.open();
        self.mode = Mode::HerdrPrompt;
    }

    fn open_repository_browser(&mut self) {
        let Some(repo) = self.git_repository() else {
            self.require_git_repository();
            return;
        };
        let root = repo.root.clone();
        let branches = repo.branches.clone();
        let prefetch = repo.github_remote;
        self.repository_browser.open(&root, &branches, prefetch);
        self.mode = Mode::RepositoryBrowser;
    }

    fn prefetch_repository_browser(&mut self) {
        let root = self
            .git_repository()
            .filter(|repo| repo.github_remote)
            .map(|repo| repo.root.clone());
        if let Some(root) = root {
            self.repository_browser.prefetch(&root);
        }
    }

    fn handle_repository_browser(&mut self, key: KeyEvent) {
        let effect = self.repository_browser.handle_key(key);
        self.apply_repository_browser_effect_option(effect);
    }

    fn open_worktree_manager(&mut self) {
        self.workspace_panel.refresh_worktree_inventory();
        let current_path = self.repository().map(|repository| repository.root.clone());
        let mut candidates = self.workspace_panel.worktree_candidates();
        if let Some(path) = current_path.as_ref()
            && !candidates
                .iter()
                .any(|candidate| same_path(&candidate.path, path))
        {
            candidates.push(WorktreeCandidate {
                path: path.clone(),
                group: None,
            });
        }
        let warning = self.worktree_manager.open(
            candidates,
            self.workspace_panel.linked_herdr_worktrees(),
            current_path,
            self.workspace_panel.is_enabled(),
            self.workspace_panel.worktree_inventory_verified(),
        );
        self.mode = Mode::WorktreeManager;
        if let Some(warning) = warning {
            self.notice = Some(warning);
        }
    }

    fn handle_worktree_manager(&mut self, key: KeyEvent) {
        let effect = self.worktree_manager.handle_key(key);
        self.apply_worktree_manager_effect_option(effect);
    }

    fn apply_worktree_manager_effect_option(&mut self, effect: Option<WorktreeManagerEffect>) {
        if let Some(effect) = effect {
            self.apply_worktree_manager_effect(effect);
        }
    }

    fn apply_worktree_manager_effect(&mut self, effect: WorktreeManagerEffect) {
        match effect {
            WorktreeManagerEffect::Close => self.mode = Mode::Normal,
            WorktreeManagerEffect::Open(path) => {
                if self
                    .repository()
                    .is_some_and(|repository| same_path(&repository.root, &path))
                {
                    self.mode = Mode::Normal;
                    self.open_repository_with_fetch(path);
                    return;
                }
                if !self.session.can_start_open() {
                    self.notice = Some("Another workspace operation is still running".to_owned());
                    return;
                }
                if self.start_repository_open(path, true) {
                    self.mode = Mode::Normal;
                } else if let Some(error) = self.workspace_explorer.error.clone() {
                    self.notice = Some(error);
                }
            }
            WorktreeManagerEffect::Refresh => {
                self.workspace_panel.refresh_worktree_inventory();
                let _ = self.worktree_manager.update_herdr_inventory(
                    self.workspace_panel.worktree_candidates(),
                    self.workspace_panel.linked_herdr_worktrees(),
                    self.workspace_panel.worktree_inventory_verified(),
                );
                self.worktree_manager.start_refresh();
            }
            WorktreeManagerEffect::CreateHerdr {
                cwd,
                path,
                branch,
                start_point,
            } => {
                if !self.session.can_start_mutation() || self.worktree_removal_running() {
                    self.notice = Some(
                        "Wait for the current workspace operation to finish before creating a worktree"
                            .to_owned(),
                    );
                    return;
                }
                if !self
                    .worktree_manager
                    .start_create(cwd, path.clone(), branch, start_point)
                {
                    self.notice = Some("A worktree operation is already running".to_owned());
                    return;
                }
                self.notice = Some(format!("Creating worktree {}...", path.display()));
            }
            WorktreeManagerEffect::RemoveNative { common_dir, path } => {
                if self
                    .repository()
                    .is_some_and(|repository| same_path(&repository.root, &path))
                    || self.session.open_running()
                    || self.worktree_removal_running()
                {
                    self.notice = Some(
                        "Cannot remove a worktree while it is active or another workspace operation is running"
                            .to_owned(),
                    );
                    return;
                }
                if !self.worktree_manager.start_remove(common_dir, path) {
                    self.notice = Some("A worktree removal is already running".to_owned());
                }
            }
            WorktreeManagerEffect::RemoveHerdr { workspace_id, path } => {
                if self
                    .repository()
                    .is_some_and(|repository| same_path(&repository.root, &path))
                    || self.session.open_running()
                    || self.worktree_removal_running()
                {
                    self.notice = Some(
                        "Cannot remove a worktree while it is active or another workspace operation is running"
                            .to_owned(),
                    );
                    return;
                }
                self.workspace_panel.delete_worktree(&workspace_id, None);
                self.notice = Some(format!("Removing worktree {}…", path.display()));
                self.mode = Mode::Normal;
            }
            WorktreeManagerEffect::Notice(notice) => self.notice = Some(notice),
        }
    }

    fn open_workspace_panel(&mut self) {
        if !self.workspace_panel_available() {
            if self.workspace_panel_enabled() {
                self.notice = Some("Workspaces need a wider terminal".to_owned());
            }
            return;
        }
        if !self.workspace_panel.is_visible() {
            self.workspace_panel.show_left();
        }
        self.mode = Mode::WorkspacePanel;
    }

    fn cycle_workspace_panel(&mut self) {
        if !self.workspace_panel_available() {
            self.open_workspace_panel();
        } else {
            self.workspace_panel.cycle_placement();
            self.mode = if self.workspace_panel.is_visible() {
                Mode::WorkspacePanel
            } else {
                Mode::Normal
            };
        }
    }

    fn handle_workspace_panel(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('p')
            && key.modifiers.is_empty()
            && self.workspace_panel.rename_dialog.is_none()
        {
            self.open_workspace_presets();
            return;
        }
        let effect = self.workspace_panel.handle_key(key);
        if effect == WorkspacePanelEffect::Unhandled {
            if is_workspace_passthrough_shortcut(key) {
                self.mode = Mode::Normal;
                self.handle_normal(key);
            }
        } else {
            self.apply_workspace_panel_effect(effect);
        }
    }

    fn open_workspace_presets(&mut self) {
        if !self.workspace_panel_enabled() {
            return;
        }
        self.workspace_panel.open_workspace_presets();
        self.mode = Mode::WorkspacePresets;
    }

    fn handle_workspace_presets(&mut self, key: KeyEvent) {
        let effect = self.workspace_panel.handle_workspace_presets(key);
        self.apply_workspace_panel_effect(effect);
    }

    pub(crate) fn apply_workspace_panel_effect(&mut self, effect: WorkspacePanelEffect) {
        match effect {
            WorkspacePanelEffect::None | WorkspacePanelEffect::Unhandled => {}
            WorkspacePanelEffect::Close => self.mode = Mode::Normal,
            WorkspacePanelEffect::Cycle => self.cycle_workspace_panel(),
            WorkspacePanelEffect::CreateWorkspace => {
                let path = self
                    .session
                    .data()
                    .map(|repository| repository.root.clone())
                    .or_else(|| std::env::current_dir().ok());
                diagnostics::event(format!(
                    "Herdr workspace create requested path={}",
                    path.as_deref()
                        .map_or_else(|| "<default>".to_owned(), |path| path.display().to_string())
                ));
                self.workspace_panel.create_workspace(path.as_deref());
            }
            WorkspacePanelEffect::CreateWorktree(workspace_id) => {
                diagnostics::event(format!(
                    "Herdr worktree create requested workspace={workspace_id}"
                ));
                self.workspace_panel.create_worktree(&workspace_id);
            }
            WorkspacePanelEffect::RenameWorkspace {
                workspace_id,
                label,
            } => {
                diagnostics::event(format!(
                    "Herdr workspace rename requested workspace={workspace_id}"
                ));
                self.workspace_panel.rename_workspace(workspace_id, label);
            }
            WorkspacePanelEffect::CloseWorkspace(workspace_id) => {
                if self.session.open_running() || self.worktree_removal_running() {
                    self.notice = Some(
                        "Wait for the current workspace operation to finish before closing it"
                            .to_owned(),
                    );
                    return;
                }
                diagnostics::event(format!(
                    "Herdr workspace close requested workspace={workspace_id}"
                ));
                self.workspace_panel.close_workspace(&workspace_id);
            }
            WorkspacePanelEffect::DeleteWorktree {
                workspace_id,
                path,
                parent_path,
            } => {
                let removing_current = path.as_deref().is_some_and(|worktree_path| {
                    self.session
                        .data()
                        .is_some_and(|repository| same_path(&repository.root, worktree_path))
                });
                if self.session.open_running()
                    || self.worktree_removal_running()
                    || (removing_current && !self.session.can_start_mutation())
                {
                    self.notice = Some(
                        "Wait for the current workspace operation to finish before removing it"
                            .to_owned(),
                    );
                    return;
                }
                diagnostics::event(format!(
                    "Herdr worktree remove requested workspace={workspace_id}"
                ));
                let reopen_path = parent_path.filter(|_| removing_current);
                self.workspace_panel
                    .delete_worktree(&workspace_id, reopen_path);
            }
            WorkspacePanelEffect::FocusAgent(pane_id) => {
                diagnostics::event(format!("Herdr agent focus requested pane={pane_id}"));
                self.workspace_panel.focus_agent(pane_id);
            }
            WorkspacePanelEffect::OpenWorkspace(path) => {
                if self.workspace_focus_restore_path.is_none() {
                    self.workspace_focus_restore_path =
                        self.repository().map(|repository| repository.root.clone());
                }
                diagnostics::event(format!(
                    "opening selected workspace in current hunkle path={}",
                    path.display()
                ));
                self.open_repository_with_fetch(path);
            }
            WorkspacePanelEffect::Notice(notice) => self.notice = Some(notice),
        }
    }

    fn open_author_filter(&mut self) {
        let Some(repo) = self.session.data().filter(|repo| !repo.is_local()) else {
            return;
        };
        self.author_filter.open(&repo.root, &repo.commits);
        self.mode = Mode::AuthorFilter;
    }

    fn handle_author_filter(&mut self, key: KeyEvent) {
        match self.author_filter.handle_key(key) {
            Some(AuthorFilterEffect::Close) => self.mode = Mode::Normal,
            Some(AuthorFilterEffect::Changed) => self.reconcile_graph_selection(),
            None => {}
        }
    }

    fn open_git_command(&mut self) {
        if self.require_git_repository() {
            self.actions.begin_input();
            self.mode = Mode::Command;
        }
    }

    fn activate_action(&mut self) {
        let action = self.actions.selected();
        if action == ActionId::Commit {
            self.view = View::Changes;
            self.set_left_pane(LeftPane::Worktree);
            if self.commit_input.text().trim().is_empty() {
                self.focus_commit();
            } else {
                self.start_commit();
            }
            return;
        }
        if action == ActionId::Custom {
            self.open_git_command();
            return;
        }
        if let Some((label, args)) = action_command(action) {
            self.start_git_command(label.to_owned(), args);
        }
    }

    fn start_git_command(&mut self, label: String, args: Vec<String>) {
        if !self.require_git_repository() {
            self.mode = Mode::Normal;
            return;
        }
        let display = display_git_command(&args);
        if self.session.start_command(label, args) {
            self.actions.begin_command(display);
            self.mode = Mode::Command;
            self.notice = None;
        } else {
            self.mode = Mode::Normal;
            self.notice = Some("Another Git operation is already running".to_owned());
        }
    }

    fn apply_explorer_command(&mut self, command: PickerCommand) {
        match command {
            PickerCommand::None => {}
            PickerCommand::Close => self.mode = Mode::Normal,
            PickerCommand::Quit => self.should_quit = true,
            PickerCommand::Open(path) => self.open_repository(path),
            PickerCommand::OpenFile(path) => self.open_workspace_file(path),
        }
    }

    fn open_workspace_file(&mut self, path: PathBuf) {
        let Some(parent) = path.parent() else {
            return;
        };
        let Some(name) = path.file_name() else {
            return;
        };
        self.pending_file_selection = Some(RepoPath::from(PathBuf::from(name)));
        if !self.start_repository_open(parent.to_path_buf(), false) {
            self.pending_file_selection = None;
        }
    }

    fn open_repository(&mut self, path: PathBuf) {
        self.start_repository_open(path, false);
    }

    fn open_repository_with_fetch(&mut self, path: PathBuf) {
        if self
            .repository()
            .is_some_and(|repository| repository.root == path)
        {
            self.workspace_fetch_pending = true;
            self.maybe_start_workspace_fetch();
            return;
        }
        self.start_repository_open(path, true);
    }

    fn queue_workspace_restore(&mut self, path: PathBuf) {
        self.pending_workspace_restore = Some(path);
        self.try_start_workspace_restore();
    }

    fn try_start_workspace_restore(&mut self) {
        let Some(path) = self.pending_workspace_restore.as_ref() else {
            return;
        };
        // The loaded repository still names the source while a speculative open is in flight.
        // Wait for that open before deciding whether the restore is already satisfied.
        if self.session.open_running() || !self.session.can_start_open() {
            return;
        }
        if self
            .repository()
            .is_some_and(|repository| same_path(&repository.root, path))
        {
            self.pending_workspace_restore = None;
            return;
        }
        let path = self
            .pending_workspace_restore
            .as_ref()
            .expect("checked pending workspace restore")
            .clone();
        if self.start_repository_open(path, false) {
            self.pending_workspace_restore = None;
        }
    }

    fn start_repository_open(&mut self, path: PathBuf, fetch_if_stale: bool) -> bool {
        diagnostics::event(format!(
            "workspace open requested path={} fetch_if_stale={fetch_if_stale}",
            path.display()
        ));
        if self.worktree_removal_running() {
            self.notice = Some("Wait for the worktree removal to finish".to_owned());
            return false;
        }
        self.flush_commit_draft();
        if self.commit_draft_due.is_some() {
            self.workspace_explorer.error =
                Some("Could not open workspace until the commit draft is saved".to_owned());
            return false;
        }
        if self
            .session
            .start_open(path, self.settings.fetch_interval())
        {
            self.workspace_fetch_pending = fetch_if_stale;
            self.workspace_explorer.error = None;
            self.notice = Some("Opening workspace…".to_owned());
            true
        } else if self.session.open_running() {
            self.notice = Some("A workspace is already opening".to_owned());
            false
        } else {
            self.workspace_explorer.error =
                Some("Another workspace operation is running".to_owned());
            false
        }
    }

    fn worktree_removal_running(&self) -> bool {
        self.worktree_manager.operation_running()
            || self.workspace_panel.destructive_action_running()
    }

    fn maybe_start_workspace_fetch(&mut self) {
        if !self.workspace_fetch_pending {
            return;
        }
        let Some(repository) = self.session.data() else {
            return;
        };
        if repository.is_local() {
            self.workspace_fetch_pending = false;
            return;
        }
        let root = repository.root.clone();
        if fetch_is_fresh(self.recent_fetches.get(&root), Instant::now()) {
            self.workspace_fetch_pending = false;
            return;
        }
        if self.session.start_fetch(self.settings.fetch_interval()) {
            self.workspace_fetch_pending = false;
        }
    }

    fn open_explorer(&mut self) {
        let start = self
            .repository()
            .map(|repo| repo.root.clone())
            .unwrap_or_else(|| self.workspace_explorer.directory.clone());
        if self.workspace_explorer.directory == start {
            let _ = self.workspace_explorer.poll_index();
        } else {
            self.workspace_explorer.navigate(start);
        }
        self.workspace_explorer.editing_path = false;
        self.mode = Mode::Explorer;
    }

    fn open_file_search(&mut self) {
        let Some(repository) = self.session.data() else {
            return;
        };
        self.file_search
            .reindex(&repository.files, Some(repository.files_fingerprint));
        self.file_search.open();
        self.mode = Mode::FileSearch;
    }

    fn activate_file_search_result(&mut self) {
        let Some(file_index) = self.file_search.selected_file_index() else {
            return;
        };
        let viewport = self
            .regions
            .explorer_list
            .map_or(0, |rect| usize::from(rect.height));
        let Some(repo) = self.session.data() else {
            return;
        };
        self.changes.set_pane(LeftPane::Files, Some(repo));
        if self
            .changes
            .select_explorer_file(repo, file_index, viewport)
        {
            self.view = View::Changes;
            self.graph_commit_open = false;
            self.mode = Mode::Normal;
        }
    }

    fn toggle_auto_fetch(&mut self) {
        self.settings.auto_fetch = !self.settings.auto_fetch;
        self.settings_changed();
    }

    fn toggle_workspace_panel_enabled(&mut self) {
        self.settings.workspace_panel_enabled = !self.settings.workspace_panel_enabled;
        self.dragging_workspace_panel_splitter = false;
        self.settings_changed();
    }

    fn toggle_agent_harness(&mut self) {
        self.settings.show_agent_harness = !self.settings.show_agent_harness;
        self.settings_changed();
    }

    fn toggle_media_preview_protocol(&mut self) {
        self.settings.media_preview_protocol = self.settings.media_preview_protocol.next();
        self.reset_media_presentation();
        self.settings_changed();
    }

    fn change_fetch_interval(&mut self, delta: i16) {
        self.settings.fetch_interval_minutes =
            (self.settings.fetch_interval_minutes as i16 + delta).clamp(1, 1440) as u16;
        self.settings_changed();
    }

    fn settings_changed(&mut self) {
        self.session
            .reset_fetch_deadline(self.settings.fetch_interval());
        self.persist_settings();
    }

    fn persist_settings(&mut self) {
        if let Err(error) = self.settings_store.save(&self.settings) {
            self.notice = Some(format!("Could not save settings: {error}"));
        }
    }

    fn restore_commit_draft(&mut self) {
        self.commit_input.clear();
        self.commit_scroll = None;
        self.commit_draft_due = None;
        self.commit_draft_path = None;
        self.commit_draft_rx = None;
        let Some(root) = self
            .repository()
            .filter(|repo| !repo.is_local())
            .map(|repo| repo.root.clone())
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.commit_draft_rx = Some(receiver);
        thread::spawn(move || {
            let result = git::commit_draft_path(&root)
                .map_err(|error| error.to_string())
                .and_then(|path| match fs::read_to_string(&path) {
                    Ok(message) => Ok((path, Some(message))),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((path, None)),
                    Err(error) => Err(error.to_string()),
                });
            let _ = sender.send(CommitDraftResult { root, result });
        });
    }

    fn schedule_commit_draft(&mut self) {
        self.commit_draft_due = Some(Instant::now() + Duration::from_millis(300));
    }

    fn flush_commit_draft_if_due(&mut self) -> bool {
        if self
            .commit_draft_due
            .is_some_and(|due| Instant::now() >= due)
        {
            return self.flush_commit_draft();
        }
        false
    }

    pub(crate) fn flush_commit_draft(&mut self) -> bool {
        if self.commit_draft_due.is_none() {
            return false;
        }
        let Some(path) = &self.commit_draft_path else {
            if self.commit_draft_rx.is_some() {
                return false;
            }
            self.commit_draft_due = None;
            return false;
        };
        let result = if self.commit_input.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        } else {
            atomic_write(path, self.commit_input.text().as_bytes())
        };
        if let Err(error) = result {
            self.commit_draft_due = Some(Instant::now() + Duration::from_secs(1));
            self.notice = Some(format!("Could not save commit draft: {error}"));
            return true;
        }
        self.commit_draft_due = None;
        false
    }

    pub(crate) fn commit_draft_pending(&self) -> bool {
        self.commit_draft_due.is_some()
    }

    fn move_selection(&mut self, delta: isize) {
        match self.view {
            View::Changes => {
                let worktree_viewport = self
                    .regions
                    .worktree_list
                    .map_or(0, |rect| usize::from(rect.height));
                let explorer_viewport = self
                    .regions
                    .explorer_list
                    .map_or(0, |rect| usize::from(rect.height));
                self.changes.move_selection(
                    self.session.data(),
                    delta,
                    worktree_viewport,
                    explorer_viewport,
                );
            }
            View::Graph => {
                let len = self.visible_graph_len();
                move_table(&mut self.graph_state, len, delta);
                self.graph_scroll_to_selection = true;
            }
        }
    }

    fn select_first(&mut self) {
        match self.view {
            View::Changes => {
                let worktree_viewport = self
                    .regions
                    .worktree_list
                    .map_or(0, |rect| usize::from(rect.height));
                let explorer_viewport = self
                    .regions
                    .explorer_list
                    .map_or(0, |rect| usize::from(rect.height));
                self.changes.select_first(
                    self.session.data(),
                    worktree_viewport,
                    explorer_viewport,
                );
            }
            View::Graph => {
                self.graph_state
                    .select((self.visible_graph_len() > 0).then_some(0));
                self.graph_scroll_to_selection = true;
            }
        }
    }

    fn select_last(&mut self) {
        match self.view {
            View::Changes => {
                let worktree_viewport = self
                    .regions
                    .worktree_list
                    .map_or(0, |rect| usize::from(rect.height));
                let explorer_viewport = self
                    .regions
                    .explorer_list
                    .map_or(0, |rect| usize::from(rect.height));
                self.changes
                    .select_last(self.session.data(), worktree_viewport, explorer_viewport);
            }
            View::Graph => {
                self.graph_state
                    .select(self.visible_graph_len().checked_sub(1));
                self.graph_scroll_to_selection = true;
            }
        }
    }

    fn toggle_stage(&mut self) {
        let Some(repo) = self.repository() else {
            return;
        };
        let Some(index) = self.changes.selected_change_index(repo) else {
            return;
        };
        let Some(change) = repo.changes.get(index).cloned() else {
            return;
        };
        let mutation = if change.staged {
            Mutation::Unstage(change)
        } else {
            Mutation::Stage(change)
        };
        let _ = self.session.start_mutation(mutation);
    }

    fn stage_hunk(&mut self, index: usize, preserve_selection: bool) {
        let path_is_invalid = self.repository().is_some_and(|repo| {
            self.changes
                .selected_change_index(repo)
                .and_then(|index| repo.changes.get(index))
                .is_some_and(|change| !change.path.is_utf8())
        });
        if path_is_invalid {
            self.notice =
                Some("Hunk actions are unavailable for paths that are not valid UTF-8".to_owned());
            return;
        }
        let patch = self.changes.diff.clone();
        let path = preserve_selection
            .then(|| {
                let repo = self.repository()?;
                let index = self.changes.selected_change_index(repo)?;
                Some(repo.changes.get(index)?.path.clone())
            })
            .flatten();
        let started = self
            .session
            .start_mutation(Mutation::StageHunk { patch, index });
        if started && let Some(path) = path {
            self.changes
                .preserve_hunk_selection_after_stage(path, index);
        }
    }

    fn stage_all(&mut self) {
        if self.require_git_repository() {
            let _ = self.session.start_mutation(Mutation::StageAll);
        }
    }

    fn toggle_all_staging(&mut self) {
        let all_staged = self.repository().is_some_and(|repo| {
            !repo.changes.is_empty() && repo.changes.iter().all(|change| change.staged)
        });
        if all_staged {
            self.unstage_all();
        } else {
            self.stage_all();
        }
    }

    fn unstage_all(&mut self) {
        if self.require_git_repository() {
            let _ = self.session.start_mutation(Mutation::UnstageAll);
        }
    }

    fn reload(&mut self, scope: RefreshScope) {
        let Some(repo) = self.repository() else {
            return;
        };
        let selection = self.changes.capture_selection(repo);
        let selected_oid = self
            .selected_graph_commit()
            .map(|commit| commit.oid.clone());

        if self
            .session
            .start_reload(scope, self.settings.fetch_interval())
        {
            self.pending_reload = Some((selection, selected_oid));
            self.notice = Some("Refreshing…".to_owned());
        } else {
            self.pending_reload = Some((selection, selected_oid));
            self.reload_queued = Some(
                self.reload_queued
                    .map_or(scope, |queued| queued.union(scope)),
            );
        }
    }

    pub(crate) fn selected_explorer_file_path(&self) -> Option<&RepoPath> {
        self.changes
            .selected_explorer_file_path(self.session.data()?)
    }

    pub(crate) fn markdown_preview_available(&self) -> bool {
        self.view == View::Changes
            && self.changes.pane == LeftPane::Files
            && self.changes.preview_image.is_none()
            && self.changes.sqlite_browser.is_none()
            && self
                .selected_explorer_file_path()
                .is_some_and(is_markdown_path)
    }

    pub(crate) fn markdown_preview_rendered(&self) -> bool {
        self.markdown_preview_available() && self.changes.markdown_rendered
    }

    fn toggle_markdown_preview(&mut self) {
        if !self.markdown_preview_available() {
            return;
        }
        self.changes.toggle_markdown_rendered();
    }

    fn set_left_pane(&mut self, pane: LeftPane) {
        if self.changes.set_pane(pane, self.session.data()) {
            self.mode = Mode::Normal;
        }
    }

    fn toggle_left_pane(&mut self) {
        self.set_left_pane(match self.changes.pane {
            LeftPane::Worktree => LeftPane::Files,
            LeftPane::Files => LeftPane::Worktree,
        });
        self.show_graph_if_diff_empty();
    }

    fn toggle_changes_files(&mut self) {
        if self.view == View::Graph {
            self.view = View::Changes;
            self.graph_commit_open = false;
            self.set_left_pane(LeftPane::Files);
            self.show_graph_if_diff_empty();
        } else {
            self.toggle_left_pane();
        }
    }

    fn show_graph_if_diff_empty(&mut self) {
        let should_show_graph = self.view == View::Changes
            && self.mode != Mode::Commit
            && self.repository().is_some_and(|repo| {
                !repo.is_local()
                    && !repo.commits.is_empty()
                    && !self.changes.has_preview_target(repo)
            });
        if should_show_graph {
            self.view = View::Graph;
            self.graph_commit_open = false;
        }
    }

    fn start_commit(&mut self) {
        if !self.require_git_repository() {
            self.mode = Mode::Normal;
            return;
        }
        if self.session.commit_running() {
            self.notice = Some("A commit is already running".to_owned());
            return;
        }
        if self.session.command_running() {
            self.notice = Some("Another Git operation is already running".to_owned());
            return;
        }
        let message = self.commit_input.text().trim().to_owned();
        if message.is_empty() {
            self.notice = Some("Commit message cannot be empty".to_owned());
            return;
        }
        self.flush_commit_draft();
        if self.session.start_commit(message) {
            self.mode = Mode::Normal;
        }
    }

    fn generate_commit_message(&mut self) {
        let Some(repo) = self.git_repository() else {
            self.notice = Some("Open a Git repository first".to_owned());
            return;
        };
        if repo.changes.is_empty() {
            self.notice = Some("No changes to describe".to_owned());
            return;
        }
        let root = repo.root.clone();
        let baseline = self.commit_input.text().to_owned();
        match self.commit_message_generator.start(root, baseline) {
            Ok(()) => {
                self.mode = Mode::Commit;
                self.commit_input.focus();
                self.notice = Some("Generating commit message with OpenCode…".to_owned());
            }
            Err(error) => self.notice = Some(error),
        }
    }

    fn focus_commit(&mut self) {
        if !self.require_git_repository() {
            return;
        }
        self.mode = Mode::Commit;
        self.commit_scroll = None;
        self.commit_input.focus();
    }
}

fn fetch_is_fresh(fetched_at: Option<&Instant>, now: Instant) -> bool {
    fetched_at.is_some_and(|fetched_at| {
        now.saturating_duration_since(*fetched_at) < WORKSPACE_FETCH_FRESHNESS
    })
}

fn first_error(stderr: &str, fallback: &str) -> String {
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn is_markdown_path(path: &RepoPath) -> bool {
    path.as_path()
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["md", "markdown", "mdown", "mkd", "mkdn"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn is_workspace_passthrough_shortcut(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::F(1)
        | KeyCode::F(2)
        | KeyCode::F(3)
        | KeyCode::Tab
        | KeyCode::PageUp
        | KeyCode::PageDown => true,
        KeyCode::Delete => key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Char('w') => key.modifiers.contains(KeyModifiers::ALT),
        KeyCode::Char(
            'q' | '1' | '2' | 'o' | 's' | 'b' | 'x' | '?' | 'e' | 'E' | 'f' | 'm' | 'c' | 'a' | 'u'
            | ' ',
        ) => true,
        _ => false,
    }
}

fn move_table(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0);
    let next = (current as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
    state.select(Some(next));
}

fn scroll_table(state: &mut TableState, len: usize, viewport: usize, delta: isize) {
    let maximum = len.saturating_sub(viewport);
    *state.offset_mut() = state.offset().saturating_add_signed(delta).min(maximum);
}

#[cfg(test)]
mod tests {
    use std::{process::Command, thread};

    use crate::media::MediaPreviewProtocol;

    use super::*;

    #[test]
    fn clearing_hit_targets_removes_overlaps_but_keeps_adjacent_targets() {
        let mut regions = Regions::default();
        regions.register_hit_target(HitTarget::CommitMessageGenerate, Rect::new(0, 0, 4, 1));
        regions.register_hit_target(HitTarget::MarkdownPreviewToggle, Rect::new(3, 0, 4, 1));
        regions.register_hit_target(
            HitTarget::Graph(GraphHitTarget::AuthorHeader),
            Rect::new(7, 0, 2, 1),
        );

        regions.clear_hit_targets_in(Rect::new(4, 0, 3, 1));

        assert!(
            regions
                .hit_target_rect(HitTarget::CommitMessageGenerate)
                .is_some()
        );
        assert!(
            regions
                .hit_target_rect(HitTarget::MarkdownPreviewToggle)
                .is_none()
        );
        assert!(
            regions
                .hit_target_rect(HitTarget::Graph(GraphHitTarget::AuthorHeader))
                .is_some()
        );
    }

    #[test]
    fn graph_scrolling_moves_the_viewport_without_moving_selection() {
        let mut state = TableState::default().with_selected(4);

        scroll_table(&mut state, 20, 5, 3);
        assert_eq!(state.selected(), Some(4));
        assert_eq!(state.offset(), 3);

        scroll_table(&mut state, 20, 5, 30);
        assert_eq!(state.selected(), Some(4));
        assert_eq!(state.offset(), 15);
    }

    #[test]
    fn opens_a_nested_directory_as_a_local_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        let nested = root.join("nested/config");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("settings.toml"), "theme = 'test'\n").unwrap();

        let app = App::new(nested.clone());

        let repo = app.session.data().unwrap();
        assert!(repo.is_local());
        assert_eq!(repo.root, fs::canonicalize(&nested).unwrap());
        assert_eq!(repo.branch, "local");
        assert_eq!(repo.files, ["settings.toml"]);
        assert_eq!(repo.change_counts, (0, 0));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.changes.pane, LeftPane::Files);
    }

    #[test]
    fn opening_a_file_from_the_explorer_selects_it_in_the_new_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("auth.json"), "{}\n").unwrap();
        let mut app = App::new(root.to_path_buf());

        app.apply_explorer_command(PickerCommand::OpenFile(nested.join("auth.json")));
        wait_for_state(&mut app, |app| !app.session.open_running());

        let repo = app.repository().unwrap().clone();
        assert_eq!(repo.root, fs::canonicalize(&nested).unwrap());
        assert!(repo.is_local());
        assert_eq!(app.view, View::Changes);
        assert_eq!(app.changes.pane, LeftPane::Files);
        assert_eq!(
            app.changes.selected_explorer_file_path(&repo),
            Some(&RepoPath::from("auth.json"))
        );
        assert!(app.pending_file_selection.is_none());
    }

    #[test]
    fn repeated_open_keeps_the_first_workspace_request_active() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("file.txt"), "content\n").unwrap();
        let mut app = App::new(root.to_path_buf());

        app.open_repository(root.to_path_buf());
        assert!(app.session.open_running());
        app.open_repository(root.to_path_buf());

        assert_eq!(
            app.notice.as_deref(),
            Some("A workspace is already opening")
        );
        wait_for_state(&mut app, |app| !app.session.open_running());
        assert_eq!(
            app.repository().unwrap().root,
            fs::canonicalize(root).unwrap()
        );
        assert_eq!(app.notice.as_deref(), Some("Workspace opened"));
    }

    #[test]
    fn opening_a_workspace_keeps_the_workspace_panel_focused() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.txt"), "first\n").unwrap();
        fs::write(second.path().join("second.txt"), "second\n").unwrap();
        let mut app = App::new(first.path().to_path_buf());
        app.mode = Mode::WorkspacePanel;

        app.open_repository_with_fetch(second.path().to_path_buf());
        wait_for_state(&mut app, |app| !app.session.open_running());

        assert_eq!(app.mode, Mode::WorkspacePanel);
        assert_eq!(
            app.repository().unwrap().root,
            fs::canonicalize(second.path()).unwrap()
        );
    }

    #[test]
    fn speculative_workspace_open_restores_after_focus_succeeds() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::write(first.path().join("first.txt"), "first\n").unwrap();
        fs::write(second.path().join("second.txt"), "second\n").unwrap();
        let first_path = fs::canonicalize(first.path()).unwrap();
        let second_path = fs::canonicalize(second.path()).unwrap();
        let mut app = App::new(first.path().to_path_buf());
        app.mode = Mode::WorkspacePanel;

        app.apply_workspace_panel_effect(WorkspacePanelEffect::OpenWorkspace(
            second.path().to_path_buf(),
        ));
        assert!(app.session.open_running());
        wait_for_state(&mut app, |app| {
            !app.session.open_running()
                && app
                    .repository()
                    .is_some_and(|repository| repository.root == second_path)
        });
        assert_eq!(
            app.workspace_focus_restore_path.as_deref(),
            Some(first_path.as_path())
        );

        let restore_path = app.workspace_focus_restore_path.take().unwrap();
        app.queue_workspace_restore(restore_path);
        wait_for_state(&mut app, |app| {
            !app.session.open_running()
                && app.pending_workspace_restore.is_none()
                && app
                    .repository()
                    .is_some_and(|repository| repository.root == first_path)
        });

        app.apply_workspace_panel_effect(WorkspacePanelEffect::OpenWorkspace(
            second.path().to_path_buf(),
        ));
        let restore_path = app.workspace_focus_restore_path.take().unwrap();
        app.queue_workspace_restore(restore_path);
        assert_eq!(
            app.pending_workspace_restore.as_deref(),
            Some(first_path.as_path())
        );
        wait_for_state(&mut app, |app| {
            !app.session.open_running()
                && app.pending_workspace_restore.is_none()
                && app
                    .repository()
                    .is_some_and(|repository| repository.root == first_path)
        });

        assert_eq!(app.mode, Mode::WorkspacePanel);
    }

    #[test]
    fn workspace_open_errors_remain_visible_after_explorer_closes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("file.txt"), "content\n").unwrap();
        let mut app = App::new(root.to_path_buf());

        app.open_repository(root.join("missing"));
        app.mode = Mode::Normal;
        wait_for_state(&mut app, |app| !app.session.open_running());

        let notice = app.notice.as_deref().unwrap();
        assert!(notice.starts_with("Could not open workspace:"));
        assert_eq!(app.workspace_explorer.error.as_deref(), Some(notice));
        assert_eq!(
            app.repository().unwrap().root,
            fs::canonicalize(root).unwrap()
        );
    }

    #[test]
    fn local_workspaces_reload_files_and_reject_git_actions() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("one.txt"), "one\n").unwrap();
        let mut app = App::new(root.to_path_buf());
        assert!(app.repository().unwrap().is_local());

        for key in ['x', 'g', 'c', 'a', 'u', 'b'] {
            app.mode = Mode::Normal;
            app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
            assert_eq!(app.mode, Mode::Normal, "{key}");
            assert_eq!(app.notice.as_deref(), Some("Not a Git repository"), "{key}");
        }

        fs::write(root.join("two.txt"), "two\n").unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        for _ in 0..100 {
            let _ = app.poll_worker();
            if app.repository().unwrap().files == ["one.txt", "two.txt"] {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.repository().unwrap().files, ["one.txt", "two.txt"]);
    }

    #[test]
    fn creates_renames_drags_and_deletes_files_from_the_files_pane() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let renamed = " renamed.txt";
        fs::write(root.join("old.txt"), "content\n").unwrap();
        fs::create_dir(root.join("destination")).unwrap();
        let mut app = App::new(root.to_path_buf());
        app.view = View::Changes;
        app.changes.pane = LeftPane::Files;

        app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Files);
        app.handle_paste(renamed);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        wait_for_state(&mut app, |app| {
            app.repository()
                .is_some_and(|repo| repo.files.iter().any(|path| path == renamed))
                && app
                    .selected_explorer_file_path()
                    .is_some_and(|path| path == renamed)
        });
        assert!(root.join(renamed).is_file());
        assert_eq!(
            app.selected_explorer_file_path().map(RepoPath::display),
            Some(renamed.to_owned())
        );

        app.open_add_dialog();
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_paste("created");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        wait_for_state(&mut app, |app| {
            app.repository()
                .is_some_and(|repo| repo.directories.iter().any(|path| path == "created"))
                && app.changes.explorer_rows().iter().any(|row| {
                    row.directory_path
                        .as_ref()
                        .is_some_and(|path| path == "created")
                })
        });
        assert!(root.join("created").is_dir());

        let repo = app.session.data().unwrap();
        assert!(
            app.changes
                .select_explorer_path(repo, &RepoPath::from(renamed), 20)
        );
        app.regions.explorer_list = Some(Rect::new(0, 10, 30, 20));
        let source = app
            .changes
            .explorer_rows()
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|path| path == renamed))
            .unwrap();
        let target = app
            .changes
            .explorer_rows()
            .iter()
            .position(|row| {
                row.directory_path
                    .as_ref()
                    .is_some_and(|path| path == "created")
            })
            .unwrap();
        assert!(app.begin_file_drag(Position::new(1, 10 + source as u16)));
        app.update_file_drag(Position::new(1, 10 + target as u16));
        app.finish_file_drag(Position::new(1, 10 + target as u16));
        wait_for_state(&mut app, |app| {
            app.repository()
                .is_some_and(|repo| repo.files.iter().any(|path| path == "created/ renamed.txt"))
                && app
                    .selected_explorer_file_path()
                    .is_some_and(|path| path == "created/ renamed.txt")
        });
        assert!(root.join("created/ renamed.txt").is_file());

        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL));
        assert!(matches!(
            app.file_dialog.as_ref().map(|dialog| &dialog.kind),
            Some(FileDialogKind::Delete { .. })
        ));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(root.join("created/ renamed.txt").is_file());
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        wait_for_state(&mut app, |_| !root.join("created/ renamed.txt").exists());
    }

    #[test]
    fn confirms_and_discards_only_the_selected_unstaged_change() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        fs::write(root.join("tracked.txt"), "staged\n").unwrap();
        run_git(root, &["add", "tracked.txt"]);
        fs::write(root.join("tracked.txt"), "unstaged\n").unwrap();
        fs::write(root.join("other.txt"), "other unstaged\n").unwrap();
        let mut app = App::new(root.to_path_buf());
        let change_index = app
            .repository()
            .unwrap()
            .changes
            .iter()
            .position(|change| change.path == "tracked.txt" && !change.staged)
            .unwrap();
        let row = app
            .changes
            .worktree_rows(app.repository().unwrap())
            .iter()
            .position(|row| row.change_index == Some(change_index))
            .unwrap();
        let repo = app.repository().unwrap().clone();
        assert!(app.changes.select_worktree_row(&repo, row));

        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(matches!(
            app.file_dialog.as_ref().map(|dialog| &dialog.kind),
            Some(FileDialogKind::DiscardUnstaged { change })
                if change.path == "tracked.txt" && !change.staged
        ));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "unstaged\n"
        );

        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        wait_for_state(&mut app, |app| {
            app.repository().is_some_and(|repo| {
                repo.changes
                    .iter()
                    .filter(|change| change.path == "tracked.txt")
                    .all(|change| change.staged)
                    && repo
                        .changes
                        .iter()
                        .filter(|change| change.path == "tracked.txt")
                        .count()
                        == 1
            })
        });

        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "staged\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("other.txt")).unwrap(),
            "other unstaged\n"
        );
        assert_eq!(app.changes.pane, LeftPane::Worktree);
        assert_eq!(
            app.repository()
                .and_then(|repo| app.changes.selected_change_index(repo))
                .and_then(|index| app.repository()?.changes.get(index))
                .map(|change| (change.path.display(), change.staged)),
            Some(("tracked.txt".to_owned(), true))
        );
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(app.file_dialog.is_none());
        assert_eq!(
            app.notice.as_deref(),
            Some("Select an unstaged change to discard")
        );
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "staged\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_hunk_actions_for_non_utf8_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        let name = OsString::from_vec(b"invalid-\x80.txt".to_vec());
        fs::write(root.join(&name), "original\n").unwrap();
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "add invalid path"]);
        fs::write(root.join(&name), "changed\n").unwrap();

        let mut app = App::new(root.to_path_buf());
        let change_index = app
            .repository()
            .unwrap()
            .changes
            .iter()
            .position(|change| change.path.as_path() == Path::new(&name) && !change.staged)
            .unwrap();
        let row = app
            .changes
            .worktree_rows(app.repository().unwrap())
            .iter()
            .position(|row| row.change_index == Some(change_index))
            .unwrap();
        let repo = app.repository().unwrap().clone();
        assert!(app.changes.select_worktree_row(&repo, row));

        app.stage_hunk(0, false);

        assert_eq!(
            app.notice.as_deref(),
            Some("Hunk actions are unavailable for paths that are not valid UTF-8")
        );
        assert!(
            app.repository()
                .unwrap()
                .changes
                .iter()
                .any(|change| change.path.as_path() == Path::new(&name) && !change.staged)
        );
    }

    #[test]
    fn empty_diff_falls_back_to_the_graph() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);

        let mut app = App::new(root.to_path_buf());
        assert_eq!(app.view, View::Graph);

        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(app.view, View::Graph);

        app.changes.pane = LeftPane::Files;
        app.changes.explorer_state.select(None);
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(app.view, View::Graph);

        fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        let mut dirty_app = App::new(root.to_path_buf());
        assert_eq!(dirty_app.view, View::Changes);

        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        dirty_app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        wait_for_state(&mut dirty_app, |app| {
            app.repository().is_some_and(|repo| repo.changes.is_empty())
        });
        assert_eq!(dirty_app.view, View::Graph);
    }

    #[test]
    fn stages_worktree_changes_while_the_graph_is_visible() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        fs::write(root.join("tracked.txt"), "edited\n").unwrap();

        let mut app = App::new(root.to_path_buf());
        app.view = View::Graph;
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        wait_for_state(&mut app, |app| {
            app.repository()
                .is_some_and(|repo| repo.changes.iter().all(|change| change.staged))
        });
        assert_eq!(app.view, View::Graph);

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        wait_for_state(&mut app, |app| {
            app.repository()
                .is_some_and(|repo| repo.changes.iter().all(|change| !change.staged))
        });
        assert_eq!(app.view, View::Graph);

        let selected = app.changes.worktree_state.selected().unwrap() as u16;
        app.regions.worktree_list = Some(Rect::new(0, 0, 20, selected + 1));
        app.regions.register_hit_target(
            HitTarget::Changes(app.changes.worktree_stage_target(selected as usize)),
            Rect::new(18, selected, 2, 1),
        );
        app.handle_left_click(Position::new(19, selected));
        wait_for_state(&mut app, |app| {
            app.repository()
                .is_some_and(|repo| repo.changes.iter().all(|change| change.staged))
        });
        assert_eq!(app.view, View::Graph);
    }

    #[test]
    fn switches_views_with_tab_and_edits_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config");
        let mut app = App::new(directory.path().join("missing"));
        app.mode = Mode::Normal;
        app.settings = Settings::default();
        app.settings_store = SettingsStore::at(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.view, View::Graph);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.view, View::Changes);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.graph_commit_open = true;
        app.changes.pane = LeftPane::Files;
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(app.changes.pane, LeftPane::Worktree);
        assert_eq!(app.view, View::Graph);
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.view, View::Changes);
        assert_eq!(app.changes.pane, LeftPane::Files);
        assert!(!app.graph_commit_open);
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.changes.pane, LeftPane::Worktree);
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.changes.pane, LeftPane::Files);

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Settings);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(
            app.settings,
            Settings {
                auto_fetch: true,
                fetch_interval_minutes: 6,
                worktree_width: 38,
                workspace_panel_enabled: true,
                show_agent_harness: false,
                workspace_panel_width: DEFAULT_WORKSPACE_PANEL_WIDTH,
                history_height: 7,
                editor_command: None,
                media_preview_protocol: MediaPreviewProtocol::Auto,
            }
        );
        assert_eq!(app.settings_store.load(), app.settings);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.settings.workspace_panel_enabled);
        assert_eq!(app.settings_store.load(), app.settings);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.settings.show_agent_harness);
        assert_eq!(app.settings_store.load(), app.settings);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.settings.media_preview_protocol,
            MediaPreviewProtocol::Halfblocks
        );
        assert_eq!(app.settings_store.load(), app.settings);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Editor);
        assert!(app.editor_configure_only);
        app.editor_input.clear();
        app.handle_paste("nvim");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Settings);
        assert_eq!(app.settings.editor_command.as_deref(), Some("nvim"));
        assert_eq!(app.settings_store.load(), app.settings);

        app.mode = Mode::Normal;
        app.changes.diff_scroll = 37;
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
        assert!(app.changes.diff_wrap);
        assert_eq!(app.changes.diff_scroll, 37);
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
        assert!(!app.changes.diff_wrap);
        assert_eq!(app.changes.diff_scroll, 37);
    }

    #[test]
    fn auto_fetch_runs_without_blocking_the_app() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.name", "Fetch Test"][..],
            &["config", "user.email", "fetch@example.com"][..],
        ] {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
        for args in [&["add", "."][..], &["commit", "-m", "initial"][..]] {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
        }

        let mut app = App::new(root.to_path_buf());
        app.settings.auto_fetch = true;
        app.session.schedule_fetch_now();
        let _ = app.poll_worker();
        assert!(app.fetch_running());

        for _ in 0..100 {
            thread::sleep(Duration::from_millis(10));
            let _ = app.poll_worker();
            if !app.fetch_running() {
                break;
            }
        }
        assert!(!app.fetch_running());
        assert_eq!(app.notice.as_deref(), Some("Fetched remotes"));
    }

    #[test]
    fn workspace_fetches_expire_after_five_minutes() {
        let now = Instant::now();
        assert!(!fetch_is_fresh(None, now));
        assert!(fetch_is_fresh(
            Some(&(now - Duration::from_secs(5 * 60 - 1))),
            now
        ));
        assert!(!fetch_is_fresh(
            Some(&(now - Duration::from_secs(5 * 60))),
            now
        ));
    }

    #[test]
    fn control_j_commits_in_terminals_that_encode_control_enter_as_line_feed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        fs::write(root.join("next.txt"), "next\n").unwrap();
        run_git(root, &["add", "next.txt"]);

        let mut app = App::new(root.to_path_buf());
        app.mode = Mode::Commit;
        app.commit_input.set("commit from control enter");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert!(app.commit_running());
        assert_eq!(app.commit_input.text(), "commit from control enter");

        for _ in 0..100 {
            thread::sleep(Duration::from_millis(10));
            let _ = app.poll_worker();
            if app.repository().unwrap().commits.len() == 2 {
                break;
            }
        }
        assert!(!app.commit_running());
        assert!(app.commit_input.is_empty());
        assert_eq!(app.repository().unwrap().commits.len(), 2);
    }

    #[test]
    fn restores_commit_drafts_and_removes_them_after_commit() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        fs::write(root.join("next.txt"), "next\n").unwrap();
        run_git(root, &["add", "next.txt"]);
        let draft_path = git::commit_draft_path(root).unwrap();

        let mut app = App::new(root.to_path_buf());
        wait_for_state(&mut app, |app| app.commit_draft_path.is_some());
        app.mode = Mode::Commit;
        app.handle_paste("persisted subject\npersisted body");
        assert!(!draft_path.exists());
        assert!(app.commit_draft_due.is_some());
        app.flush_commit_draft();
        assert_eq!(
            fs::read_to_string(&draft_path).unwrap(),
            "persisted subject\npersisted body"
        );
        drop(app);

        let mut restored = App::new(root.to_path_buf());
        wait_for_state(&mut restored, |app| !app.commit_input.is_empty());
        assert_eq!(
            restored.commit_input.text(),
            "persisted subject\npersisted body"
        );
        restored.mode = Mode::Commit;
        restored.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        wait_for_state(&mut restored, |app| {
            app.repository().is_some_and(|repo| repo.commits.len() == 2)
        });
        assert!(restored.commit_input.is_empty());
        assert!(!draft_path.exists());
    }

    #[test]
    fn keeps_edits_pending_until_async_draft_discovery_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        let mut app = App::new(root.to_path_buf());
        let draft_path = root.join("draft");
        let (sender, receiver) = mpsc::channel();
        app.commit_draft_rx = Some(receiver);
        app.commit_draft_path = None;
        app.commit_input.set("new message");
        app.commit_draft_due = Some(Instant::now());

        assert!(!app.flush_commit_draft());
        assert!(app.commit_draft_due.is_some());
        sender
            .send(CommitDraftResult {
                root: root.to_path_buf(),
                result: Ok((draft_path.clone(), Some("old message".to_owned()))),
            })
            .unwrap();
        app.poll_worker();
        app.flush_commit_draft();

        assert_eq!(app.commit_input.text(), "new message");
        assert_eq!(fs::read_to_string(draft_path).unwrap(), "new message");
    }

    #[test]
    fn failed_draft_save_blocks_workspace_switch_and_keeps_retry_pending() {
        let current = tempfile::tempdir().unwrap();
        initialize_repository(current.path());
        let next = tempfile::tempdir().unwrap();
        initialize_repository(next.path());
        let mut app = App::new(current.path().to_path_buf());
        app.commit_input.set("unsaved draft");
        app.commit_draft_path = Some(current.path().join("missing/draft"));
        app.commit_draft_due = Some(Instant::now());
        app.pending_workspace_restore = Some(next.path().to_path_buf());

        app.try_start_workspace_restore();

        assert_eq!(app.commit_input.text(), "unsaved draft");
        assert!(app.commit_draft_due.is_some());
        assert_eq!(
            app.pending_workspace_restore,
            Some(next.path().to_path_buf())
        );
        assert!(!app.session.open_running());
    }

    #[test]
    fn commit_action_submits_an_existing_message() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        fs::write(root.join("next.txt"), "next\n").unwrap();
        run_git(root, &["add", "next.txt"]);

        let mut app = App::new(root.to_path_buf());
        app.commit_input.set("commit from actions");
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(app.commit_running());
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(10));
            let _ = app.poll_worker();
            if app.repository().unwrap().commits.len() == 2 {
                break;
            }
        }
        assert!(!app.commit_running());
        assert!(app.commit_input.is_empty());
        assert_eq!(app.repository().unwrap().commits.len(), 2);
    }

    #[test]
    fn keeps_hunk_mode_on_the_next_hunk_after_staging() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        let baseline = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>();
        fs::write(
            root.join("tracked.txt"),
            format!("{}\n", baseline.join("\n")),
        )
        .unwrap();
        run_git(root, &["add", "tracked.txt"]);
        run_git(root, &["commit", "-m", "expand fixture"]);

        let mut edited = baseline;
        edited[1] = "changed first".to_owned();
        edited[18] = "changed second".to_owned();
        fs::write(root.join("tracked.txt"), format!("{}\n", edited.join("\n"))).unwrap();
        let mut app = App::new(root.to_path_buf());
        for _ in 0..100 {
            let _ = app.poll_worker();
            if app.changes.diff.matches("@@").count() == 2 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.changes.hunk_selection, Some(0));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.changes.hunk_selection, Some(0));

        for _ in 0..200 {
            let _ = app.poll_worker();
            let repo = app.repository().unwrap();
            let split_change = repo
                .changes
                .iter()
                .any(|change| change.path == "tracked.txt" && change.staged)
                && repo
                    .changes
                    .iter()
                    .any(|change| change.path == "tracked.txt" && !change.staged);
            if split_change
                && app.changes.hunk_selection == Some(0)
                && app.changes.diff.contains("changed second")
                && !app.changes.diff.contains("changed first")
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(app.changes.hunk_selection, Some(0));
        assert!(app.changes.diff.contains("changed second"));
        assert!(!app.changes.diff.contains("changed first"));
    }

    #[test]
    fn configures_and_requests_an_interactive_editor() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        let settings_path = root.join(".git/hunkle-editor-test-config");
        let mut app = App::new(root.to_path_buf());
        app.settings.editor_command = None;
        app.settings_store = SettingsStore::at(settings_path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Editor);
        app.editor_input.clear();
        app.handle_paste("code --wait");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let request = app.take_editor_request().unwrap();
        assert_eq!(request.command, ["code", "--wait"]);
        assert_eq!(
            request.file,
            fs::canonicalize(root.join("tracked.txt")).unwrap()
        );
        assert_eq!(request.repository, fs::canonicalize(root).unwrap());
        assert_eq!(app.settings.editor_command.as_deref(), Some("code --wait"));
        assert!(
            fs::read_to_string(settings_path)
                .unwrap()
                .contains("editor_command=code --wait")
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Editor);
        assert_eq!(app.editor_input, "code --wait");

        app.mode = Mode::Normal;
        let repo = app.repository().unwrap().clone();
        app.changes.set_pane(LeftPane::Files, Some(&repo));
        assert!(
            app.changes
                .select_explorer_path(&repo, &RepoPath::from("tracked.txt"), 20)
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(
            app.take_editor_request().unwrap().file,
            fs::canonicalize(root.join("tracked.txt")).unwrap()
        );

        run_git(root, &["add", "tracked.txt"]);
        let mut app = App::new(root.to_path_buf());
        app.settings.editor_command = Some("code --wait".to_owned());
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(
            app.take_editor_request().unwrap().file,
            fs::canonicalize(root.join("tracked.txt")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn control_s_formats_the_selected_file_with_a_project_formatter() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        fs::write(root.join("config.jsonc"), "{\"value\":true}\n").unwrap();
        let formatter = root.join("node_modules/.bin/prettier");
        fs::write(
            &formatter,
            "#!/bin/sh\nprintf '{\\n  \"formatted\": true\\n}\\n' > \"$2\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&formatter).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&formatter, permissions).unwrap();

        let mut app = App::new(root.to_path_buf());
        let repo = app.repository().unwrap().clone();
        app.changes.set_pane(LeftPane::Files, Some(&repo));
        assert!(
            app.changes
                .select_explorer_path(&repo, &RepoPath::from("config.jsonc"), 20)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.format_running());
        wait_for_state(&mut app, |app| !app.format_running());
        assert_eq!(
            fs::read_to_string(root.join("config.jsonc")).unwrap(),
            "{\n  \"formatted\": true\n}\n"
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("Formatted config.jsonc with Prettier")
        );
    }

    #[test]
    fn control_s_reports_files_without_a_known_formatter() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("notes.txt"), "notes\n").unwrap();
        let mut app = App::new(root.to_path_buf());
        let repo = app.repository().unwrap().clone();
        app.changes.set_pane(LeftPane::Files, Some(&repo));
        assert!(
            app.changes
                .select_explorer_path(&repo, &RepoPath::from("notes.txt"), 20)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(
            app.notice.as_deref(),
            Some("No known formatter for .txt files")
        );
    }

    #[test]
    fn runs_a_custom_git_command_and_keeps_its_output() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        let mut app = App::new(root.to_path_buf());

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Commit);
        assert_eq!(app.view, View::Changes);
        assert_eq!(app.changes.pane, LeftPane::Worktree);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.actions.status, CommandStatus::Input);

        app.handle_paste("rev-parse --abbrev-ref HEAD");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.actions.status, CommandStatus::Running);
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(10));
            let _ = app.poll_worker();
            if !app.session.command_running() {
                break;
            }
        }

        assert_eq!(
            app.actions.status,
            CommandStatus::Complete {
                success: true,
                exit_code: Some(0),
            }
        );
        assert_eq!(app.actions.stdout.trim(), "main");
        assert_eq!(app.actions.command, "git rev-parse --abbrev-ref HEAD");
        assert!(app.actions.input.is_empty());
        assert_eq!(app.actions.transcript.len(), 1);
        assert_eq!(app.actions.transcript[0].stdout.trim(), "main");

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.handle_paste("tatus --short");
        assert_eq!(app.actions.input, "status --short");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.actions.status, CommandStatus::Running);
        assert_eq!(app.actions.transcript.len(), 1);
        assert_eq!(app.actions.transcript[0].stdout.trim(), "main");
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(10));
            let _ = app.poll_worker();
            if !app.session.command_running() {
                break;
            }
        }
        assert_eq!(
            app.actions.status,
            CommandStatus::Complete {
                success: true,
                exit_code: Some(0),
            }
        );
        assert_eq!(app.actions.command, "git status --short");
        assert!(app.actions.input.is_empty());
        assert_eq!(app.actions.transcript.len(), 2);
        assert_eq!(
            app.actions.transcript[0].command,
            "git rev-parse --abbrev-ref HEAD"
        );
        assert_eq!(app.actions.transcript[0].stdout.trim(), "main");
        assert_eq!(app.actions.transcript[1].command, "git status --short");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn refreshes_an_already_dirty_file_when_its_contents_change_again() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        initialize_repository(root);
        let tracked = root.join("tracked.txt");
        fs::write(&tracked, "first\n").unwrap();
        let mut app = App::new(root.to_path_buf());
        wait_for_state(&mut app, |app| app.changes.diff.contains("first"));

        fs::write(&tracked, "later content\n").unwrap();
        app.session.schedule_status_check_now();
        wait_for_state(&mut app, |app| app.changes.diff.contains("later"));
    }

    fn initialize_repository(root: &Path) {
        for args in [
            &["init", "-b", "main"][..],
            &["config", "core.autocrlf", "false"][..],
            &["config", "user.name", "App Test"][..],
            &["config", "user.email", "app@example.com"][..],
        ] {
            run_git(root, args);
        }
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        run_git(root, &["add", "tracked.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn wait_for_state(app: &mut App, predicate: impl Fn(&App) -> bool) {
        for _ in 0..1_000 {
            let _ = app.poll_worker();
            if predicate(app) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("application state did not update");
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
