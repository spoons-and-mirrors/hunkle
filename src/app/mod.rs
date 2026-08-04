mod actions;
mod author_filter;
mod changes;
mod commit_message;
mod commit_summary;
mod explorer;
mod file_editor;
mod file_search;
mod files;
mod fuzzy;
mod graph_search;
mod header_picker;
mod herdr_prompt;
mod herdr_session;
mod issues;
mod linked_worktrees;
mod mouse;
mod settings;
mod shortcuts;
mod text_input;

pub(crate) use actions::{ACTION_ITEMS, ActionsState, CommandRecord, CommandStatus};
pub(crate) use author_filter::{AuthorFilter, AuthorFilterEffect};
pub(crate) use changes::{ChangesHitTarget, PreviewOrigin, SqliteFocus, SqlitePage};
pub use changes::{ChangesState, LeftPane};
pub(crate) use commit_message::{CommitMessageCompletion, CommitMessageGenerator};
pub(crate) use commit_summary::CommitSummaryCache;
pub use explorer::{Explorer, PickerAction, PickerEntry};
pub(crate) use explorer::{ExplorerHitTarget, SurroundingEntry};
pub(crate) use file_editor::{FileEditor, TAB_WIDTH};
pub(crate) use file_search::{FileSearch, FileSearchRow, SearchDestination, SearchScope};
pub(crate) use files::{FileDialog, FileDialogKind, FileDrag, FileNameAction};
pub(crate) use graph_search::GraphSearch;
pub(crate) use header_picker::{
    BranchPickerStep, CloneField, HeaderPicker, HeaderPickerItem, HeaderPickerKind,
    RepositoryPickerStep, WorktreePickerStep,
};
pub(crate) use herdr_prompt::{HerdrPrompt, HerdrPromptPoll};
pub(crate) use herdr_session::{
    AgentActivityPreview, AgentEntryState, AgentKey, AgentRequestPartPreview, AgentRequestPreview,
    AgentStatus, AgentUserMessage, HerdrPaneLayout, HerdrSession,
};
#[cfg(test)]
pub(crate) use herdr_session::{HerdrPaneRect, StashedAgent};
pub(crate) use issues::{IssueCatalog, IssueScope};
pub(crate) use linked_worktrees::{
    AgentDestinationMetadata, LinkedWorktreeCandidate, LinkedWorktreeCatalog,
    LinkedWorktreeObservation, RepositoryPickerItem,
};
pub use settings::{OpenCodeReasoning, Settings};
pub(crate) use settings::{SettingsStore, valid_opencode_model};
pub(crate) use shortcuts::{KeyChord, ShortcutAction, Shortcuts};

pub(super) use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const WORKSPACE_FETCH_FRESHNESS: Duration = Duration::from_secs(5 * 60);
const STANDALONE_SETTINGS: &[usize] = &[0, 1, 2, 7, 8];
const ALL_SETTINGS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8];
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug)]
struct AgentPreviewTranscriptScroll {
    agent: AgentKey,
    message: usize,
    offset: usize,
}

#[derive(Debug)]
struct AgentPreviewMessageSelection {
    agent: AgentKey,
    message: usize,
}

#[derive(Debug)]
struct AgentPreviewExpandedRequests {
    agent: AgentKey,
    message: usize,
    requests: Vec<usize>,
}

pub(super) use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub(super) use ratatui::{
    layout::{Position, Rect},
    widgets::TableState,
};

pub(super) use crate::{
    diagnostics,
    filesystem::{atomic_write, same_path},
    formatter,
    git::{self, Branch, RefreshScope, RepositoryData},
    repo_path::RepoPath,
    repository_session::{LoadKind, Mutation, RefreshRequest, RepositorySession, WorkerOutcome},
    selection::SelectionState,
    workspace_state::WorkspaceState,
};

use actions::{ActionId, action_command, display_git_command, parse_command_args, parse_git_args};
use explorer::PickerCommand;
pub(crate) use text_input::{EditOutcome, TextInput};

mod types;
pub(crate) use types::*;

mod keys;
#[cfg(test)]
mod tests;

pub struct App {
    pub(crate) session: RepositorySession,
    pub view: View,
    search_return_view: View,
    pub(crate) graph_commit_open: bool,
    graph_hidden: bool,
    single_panel_detail_open: bool,
    pub mode: Mode,
    pub changes: ChangesState,
    pub graph_state: TableState,
    pub(crate) graph_scroll_to_selection: bool,
    pub(crate) author_filter: AuthorFilter,
    pub(crate) graph_search: GraphSearch,
    pub(crate) graph_search_focused: bool,
    pub(crate) commit_summaries: CommitSummaryCache,
    pub(crate) issues: IssueCatalog,
    pub(crate) commit_input: TextInput,
    pub(crate) commit_scroll: Option<usize>,
    pub(crate) commit_message_generator: CommitMessageGenerator,
    commit_draft_path: Option<PathBuf>,
    commit_draft_due: Option<Instant>,
    commit_draft_rx: Option<Receiver<CommitDraftResult>>,
    pub dragging_splitter: bool,
    pub dragging_agents: bool,
    pub(crate) agents_height_fit_for: Option<usize>,
    pub dragging_diff_scrollbar: bool,
    pub(crate) dragging_graph_column: Option<GraphColumnDrag>,
    diff_scroll_drag_offset: u16,
    mobile_scroll_drag: Option<MobileScrollDrag>,
    pub workspace_explorer: Explorer,
    pub(crate) file_search: FileSearch,
    pub(crate) actions: ActionsState,
    pub(crate) herdr_prompt: HerdrPrompt,
    pub(crate) header_picker: HeaderPicker,
    pub(crate) linked_worktrees: LinkedWorktreeCatalog,
    pub(crate) herdr: HerdrSession,
    pub(crate) agents_visible: bool,
    pub(crate) agents_pane_pinned: bool,
    agent_preview_selection: Option<AgentKey>,
    agent_preview_transcript_scroll: Option<AgentPreviewTranscriptScroll>,
    agent_preview_message_selection: Option<AgentPreviewMessageSelection>,
    agent_preview_expanded_requests: Option<AgentPreviewExpandedRequests>,
    agent_preview_picker_open: bool,
    pub(crate) hovered_hit_target: Option<HitTarget>,
    agent_preview_button_flash: Option<(bool, Instant)>,
    pub settings: Settings,
    pub settings_selection: usize,
    pub(crate) settings_page: SettingsPage,
    pub(crate) shortcut_selection: usize,
    pub(crate) shortcut_scroll: usize,
    pub(crate) shortcut_capture: bool,
    pub(crate) shortcut_error: Option<String>,
    pub(crate) opencode_selection: usize,
    pub(crate) opencode_model_input: Option<String>,
    pub(crate) opencode_error: Option<String>,
    pub notice: Option<String>,
    pub regions: Regions,
    pub(crate) selection: SelectionState,
    copy_request: Option<String>,
    restart_request: Option<PathBuf>,
    pub should_quit: bool,
    pub(crate) settings_store: SettingsStore,
    pending_reload: Option<(changes::ChangesSelection, Option<String>)>,
    pub(crate) editor_input: String,
    pub(crate) file_editor: Option<FileEditor>,
    pub(crate) file_editor_anchor: Option<Position>,
    file_editor_return: Option<FileEditorReturn>,
    pub(crate) editor_error: Option<String>,
    pub(crate) editor_configure_only: bool,
    editor_request: Option<EditorRequest>,
    pub(crate) file_dialog: Option<FileDialog>,
    file_drag: Option<FileDrag>,
    last_agent_click: Option<(AgentKey, Instant)>,
    pending_fullscreen_agent: Option<AgentKey>,
    last_worktree_file_click: Option<(RepoPath, bool, Instant)>,
    last_explorer_file_click: Option<(RepoPath, Instant)>,
    last_file_editor_click: Option<(Position, Instant)>,
    last_file_search_click: Option<(SearchDestination, Instant)>,
    pub(crate) file_editor_dragging: bool,
    pending_file_selection: Option<RepoPath>,
    pending_workspace_restore: Option<PathBuf>,
    workspace_state: Option<WorkspaceState>,
    initial_pane_pending: bool,
    recent_fetches: HashMap<PathBuf, Instant>,
    workspace_fetch_pending: bool,
    footer_marquee: Option<FooterMarquee>,
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
        #[cfg(not(test))]
        let (settings_store, settings) = SettingsStore::discover();
        #[cfg(test)]
        let (settings_store, settings) = {
            let (_, settings) = SettingsStore::discover();
            (SettingsStore::memory(), settings)
        };
        let workspace_config_dir = settings_store.config_dir();
        #[cfg(not(test))]
        let known_repositories_path =
            workspace_config_dir.map(|path| path.join("known-repositories.json"));
        #[cfg(test)]
        let known_repositories_path = None;
        #[cfg(not(test))]
        let explorer_favorites_path =
            workspace_config_dir.map(|path| path.join("explorer-favorites.json"));
        #[cfg(test)]
        let explorer_favorites_path = None;
        let interval = settings.fetch_interval();
        let session = if open_in_background {
            RepositorySession::opening(path.clone(), interval)
        } else {
            RepositorySession::new(&path, interval)
        };
        let mode = if open_in_background || session.data().is_some() {
            Mode::Normal
        } else {
            Mode::Explorer
        };
        let start = session
            .data()
            .and_then(|repo| repo.root.parent().map(Path::to_path_buf))
            .unwrap_or(path);

        let changes = ChangesState::new(session.data());
        let initial_pane_pending = session.data().is_none_or(|repo| !repo.details_ready);
        let file_search = FileSearch::new(
            session.data().map_or(&[], |repo| repo.files.as_slice()),
            session
                .data()
                .map_or(&[], |repo| repo.ignored_files.as_slice()),
            session.data().map(|repo| repo.files_fingerprint),
        );
        let mut graph_state = TableState::default();
        graph_state.select(
            session
                .data()
                .is_some_and(|repo| !repo.commits.is_empty())
                .then_some(0),
        );
        let mut linked_worktrees = LinkedWorktreeCatalog::new(known_repositories_path);
        if let Some(repository) = session.data() {
            let _ = linked_worktrees
                .remember_workspace(repository.common_dir.as_deref(), &repository.root);
        }
        let mut herdr = HerdrSession::detect(workspace_config_dir);
        herdr.set_cross_workspace_agents(settings.cross_workspace_agents);
        linked_worktrees.observe_herdr(herdr.linked_worktree_observation());
        linked_worktrees.refresh();
        let mut author_filter = AuthorFilter::default();
        let mut graph_search = GraphSearch::default();
        if let Some(repo) = session.data() {
            author_filter.sync(&repo.root, &repo.commits);
            graph_search.sync(&repo.root, &repo.commits, author_filter.visible_indices());
        }
        let mut workspace_explorer = Explorer::with_favorites(start, explorer_favorites_path);
        workspace_explorer.left_pane_width = settings.explorer_left_pane_width;
        let mut app = Self {
            session,
            view: View::Changes,
            search_return_view: View::Changes,
            graph_commit_open: false,
            graph_hidden: false,
            single_panel_detail_open: false,
            mode,
            changes,
            graph_state,
            graph_scroll_to_selection: true,
            author_filter,
            graph_search,
            graph_search_focused: false,
            commit_summaries: CommitSummaryCache::default(),
            issues: IssueCatalog::default(),
            commit_input: TextInput::default(),
            commit_scroll: None,
            commit_message_generator: CommitMessageGenerator::detect(),
            commit_draft_path: None,
            commit_draft_due: None,
            commit_draft_rx: None,
            dragging_splitter: false,
            dragging_agents: false,
            agents_height_fit_for: None,
            dragging_diff_scrollbar: false,
            dragging_graph_column: None,
            diff_scroll_drag_offset: 0,
            mobile_scroll_drag: None,
            workspace_explorer,
            file_search,
            actions: ActionsState::default(),
            herdr_prompt: HerdrPrompt::default(),
            header_picker: HeaderPicker::default(),
            linked_worktrees,
            herdr,
            agents_visible: true,
            agents_pane_pinned: false,
            agent_preview_selection: None,
            agent_preview_transcript_scroll: None,
            agent_preview_message_selection: None,
            agent_preview_expanded_requests: None,
            agent_preview_picker_open: false,
            hovered_hit_target: None,
            agent_preview_button_flash: None,
            settings,
            settings_selection: 0,
            settings_page: SettingsPage::General,
            shortcut_selection: 0,
            shortcut_scroll: 0,
            shortcut_capture: false,
            shortcut_error: None,
            opencode_selection: 0,
            opencode_model_input: None,
            opencode_error: None,
            notice: open_in_background.then(|| "Opening workspace…".to_owned()),
            regions: Regions::default(),
            selection: SelectionState::default(),
            copy_request: None,
            restart_request: None,
            should_quit: false,
            settings_store,
            pending_reload: None,
            editor_input: String::new(),
            file_editor: None,
            file_editor_anchor: None,
            file_editor_return: None,
            editor_error: None,
            editor_configure_only: false,
            editor_request: None,
            file_dialog: None,
            file_drag: None,
            last_agent_click: None,
            pending_fullscreen_agent: None,
            last_worktree_file_click: None,
            last_explorer_file_click: None,
            last_file_editor_click: None,
            last_file_search_click: None,
            file_editor_dragging: false,
            pending_file_selection: None,
            pending_workspace_restore: None,
            workspace_state: None,
            initial_pane_pending,
            recent_fetches: HashMap::new(),
            workspace_fetch_pending: false,
            footer_marquee: None,
        };
        app.restore_commit_draft();
        app
    }

    pub(crate) fn repository(&self) -> Option<&RepositoryData> {
        self.session.data()
    }

    pub(crate) fn footer_marquee_elapsed(&mut self, value: &str, width: usize) -> Duration {
        let now = Instant::now();
        if self
            .footer_marquee
            .as_ref()
            .is_none_or(|marquee| marquee.value != value || marquee.width != width)
        {
            self.footer_marquee = Some(FooterMarquee {
                value: value.to_owned(),
                width,
                started: now,
                next_frame: now + FOOTER_MARQUEE_STEP,
            });
        }
        now.duration_since(self.footer_marquee.as_ref().unwrap().started)
    }

    pub(crate) fn clear_footer_marquee(&mut self) {
        self.footer_marquee = None;
    }

    pub(crate) fn set_workspace_state(&mut self, state: Option<WorkspaceState>) {
        self.workspace_state = state;
    }

    pub(crate) fn workspace_loading_initial_state(&self) -> bool {
        self.initial_pane_pending && self.mode == Mode::Normal
    }

    pub(crate) fn visible_view(&self) -> View {
        if self.view == View::RepositorySearch {
            View::RepositorySearch
        } else if self.view == View::Graph
            || self.graph_commit_open
            || (!self.graph_hidden
                && self.view == View::Changes
                && self.changes.preview.pane() == LeftPane::Worktree
                && self.changes.branch_comparison().is_none()
                && self.repository().is_some_and(|repo| {
                    repo.details_ready && !repo.is_local() && repo.changes.is_empty()
                }))
        {
            View::Graph
        } else {
            View::Changes
        }
    }

    pub(crate) fn agents_pane_visible(&self) -> bool {
        self.herdr_available() && self.agents_pane_pinned
    }

    pub(crate) fn herdr_available(&self) -> bool {
        self.herdr.is_enabled()
    }

    pub(crate) fn general_settings(&self) -> &'static [usize] {
        if self.herdr_available() {
            ALL_SETTINGS
        } else {
            STANDALONE_SETTINGS
        }
    }

    pub(crate) fn agents_pane_index(&self) -> Option<usize> {
        if !self.agents_pane_pinned {
            return None;
        }
        self.agent_preview_selection
            .as_ref()
            .and_then(|key| self.herdr.agent_index(key))
            .or_else(|| self.default_agent_preview_index())
    }

    fn default_agent_preview_index(&self) -> Option<usize> {
        (0..self.herdr.agents.len())
            .find(|index| self.herdr.agent_entry_state(*index).selected)
            .or((!self.herdr.agents.is_empty()).then_some(0))
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
            self.graph_search.visible_indices()
        } else {
            &[]
        }
    }

    pub(crate) fn selected_graph_commit(&self) -> Option<&git::Commit> {
        let selected = self.graph_state.selected()?;
        let index = *self.visible_graph_indices().get(selected)?;
        self.repository()?.commits.get(index)
    }

    fn visible_graph_len(&self) -> usize {
        self.repository()
            .map_or(0, |_| self.visible_graph_indices().len())
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
        if let Some(repository) = self.git_repository() {
            if repository.details_ready {
                return true;
            }
            self.notice = Some("Repository details are still loading".to_owned());
            return false;
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

    pub(crate) fn commit_message_spinner(&self) -> &'static str {
        self.commit_message_generator.spinner()
    }

    pub(crate) fn fetch_running(&self) -> bool {
        self.session.fetch_running()
    }

    pub(crate) fn format_running(&self) -> bool {
        self.session.format_running()
    }

    pub(crate) fn can_restart(&self) -> bool {
        self.session.can_restart()
            && !self.commit_message_running()
            && !self.file_editor.as_ref().is_some_and(FileEditor::dirty)
    }

    pub(crate) fn local_build_available(&self) -> bool {
        self.local_build_executable()
            .is_some_and(|path| path.is_file())
    }

    pub(crate) fn request_local_build_restart(&mut self) {
        let Some(executable) = self.local_build_executable().filter(|path| path.is_file()) else {
            self.notice = Some("Local Hunkle build is unavailable".to_owned());
            return;
        };
        self.restart_request = Some(executable);
        self.notice = Some("Reloading local Hunkle build…".to_owned());
    }

    pub(crate) fn take_restart_request(&mut self) -> Option<PathBuf> {
        self.restart_request.take()
    }

    fn local_build_executable(&self) -> Option<PathBuf> {
        Some(
            self.repository()?
                .root
                .join("target")
                .join("hunkle-install")
                .join("bin")
                .join(format!("hunkle{}", std::env::consts::EXE_SUFFIX)),
        )
    }

    pub(crate) fn dirty_file_edit(&self) -> bool {
        self.file_editor.as_ref().is_some_and(FileEditor::dirty)
    }

    pub(crate) fn shutdown(&mut self) {
        self.herdr.shutdown();
        self.file_search.shutdown();
        self.changes.shutdown();
        self.commit_summaries.shutdown();
        self.issues.shutdown();
        self.workspace_explorer.shutdown();
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.mode == Mode::FileEdit {
            self.handle_file_editor(key);
            return;
        }
        if self.header_picker.is_open() {
            self.handle_header_picker(key);
            return;
        }
        if self.herdr_prompt.agent_pane_picker_open() {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                self.should_quit = true;
            } else {
                match key.code {
                    KeyCode::Esc => {
                        self.herdr_prompt.cancel_pending_agent();
                        self.notice = Some("Agent pane selection cancelled".to_owned());
                    }
                    KeyCode::Tab => self.herdr_prompt.cycle_agent_pane_focus(false),
                    KeyCode::BackTab => self.herdr_prompt.cycle_agent_pane_focus(true),
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                        let direction = match key.code {
                            KeyCode::Up => AgentPaneDirection::Up,
                            KeyCode::Down => AgentPaneDirection::Down,
                            KeyCode::Left => AgentPaneDirection::Left,
                            KeyCode::Right => AgentPaneDirection::Right,
                            _ => unreachable!(),
                        };
                        self.herdr_prompt.move_agent_pane_focus(direction);
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        match self.herdr_prompt.activate_agent_pane_focus() {
                            Ok(()) => self.notice = Some("Starting agent in pane".to_owned()),
                            Err(error) => self.notice = Some(error),
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
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
        if self.mode == Mode::Normal && self.view == View::RepositorySearch {
            self.handle_file_search(key);
            return;
        }
        if self.mode == Mode::Normal
            && self.visible_view() == View::Graph
            && self.graph_search_focused
        {
            self.handle_graph_search(key);
            return;
        }
        if self.mode == Mode::Normal
            && self
                .settings
                .shortcuts
                .matches(ShortcutAction::FindFile, key)
        {
            self.open_file_search();
            return;
        }
        if matches!(self.mode, Mode::Normal | Mode::Commit)
            && self.herdr_available()
            && self
                .settings
                .shortcuts
                .matches(ShortcutAction::ToggleFullscreen, key)
        {
            self.toggle_fullscreen();
            return;
        }
        if matches!(self.mode, Mode::Normal | Mode::Commit) && self.handle_main_navigation(key) {
            if self.mode == Mode::Commit {
                self.flush_commit_draft();
                self.mode = Mode::Normal;
            }
            return;
        }
        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Commit => self.handle_commit_input(key),
            Mode::Explorer => self.handle_explorer(key),
            Mode::Settings => self.handle_settings(key),
            Mode::AuthorFilter => self.handle_author_filter(key),
            Mode::ActionMenu => self.handle_action_menu(key),
            Mode::Command => self.handle_command(key),
            Mode::HerdrPrompt => self.handle_herdr_prompt(key),
            Mode::FileEdit => unreachable!("file editor keys are handled first"),
            Mode::Editor => self.handle_editor(key),
            Mode::Files => self.handle_file_dialog(key),
            Mode::Help => {
                if key.code == KeyCode::Esc
                    || self
                        .settings
                        .shortcuts
                        .matches(ShortcutAction::OpenHelp, key)
                {
                    self.mode = Mode::Normal;
                }
            }
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.header_picker.naming_branch() {
            self.header_picker.branch_name.insert_single_line(text);
            self.header_picker.message = None;
            return;
        }
        if self.header_picker.cloning_repository() {
            self.header_picker
                .clone_input_mut()
                .insert_single_line(text);
            self.header_picker.message = None;
            return;
        }
        if self.header_picker.creating_worktree() {
            self.header_picker.worktree_name.insert_single_line(text);
            self.header_picker.message = None;
            return;
        }
        if self.header_picker.filtering() {
            self.header_picker.query.insert_single_line(text);
            self.header_picker.message = None;
            self.header_picker.apply_filter();
            return;
        }
        if self.mode == Mode::Normal && self.view == View::RepositorySearch {
            if let Some(repo) = self.session.data() {
                self.file_search.paste(text, repo);
            }
            return;
        }
        if self.mode == Mode::Normal
            && self.visible_view() == View::Graph
            && self.graph_search_focused
        {
            self.graph_search.input.insert_single_line(text);
            self.graph_search
                .apply(self.author_filter.visible_indices());
            self.reconcile_graph_selection();
            return;
        }
        if self.mode == Mode::Normal && self.paste_clipboard_files(text) {
            return;
        }
        match self.mode {
            Mode::FileEdit => {
                if self.file_editor_viewport_too_small() {
                    self.notice = Some("Resize the terminal before editing".to_owned());
                } else if self.format_running() {
                    self.notice = Some("Wait for the formatter to finish".to_owned());
                } else if let Some(editor) = &mut self.file_editor
                    && let Err(error) = editor.insert(text)
                {
                    self.notice = Some(format!("Could not insert text: {error}"));
                }
            }
            Mode::Commit => {
                self.commit_input.insert(text);
                self.schedule_commit_draft();
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
            Mode::Settings => {
                if let Some(input) = &mut self.opencode_model_input {
                    input.push_str(text);
                    self.opencode_error = None;
                }
            }
            Mode::Files => {
                if let Some(dialog) = &mut self.file_dialog
                    && matches!(dialog.kind, FileDialogKind::Name { .. })
                {
                    dialog.input.insert(text);
                    dialog.error = None;
                }
            }
            _ => {}
        }
    }

    pub fn take_copy_request(&mut self) -> Option<String> {
        self.copy_request.take()
    }

    pub fn poll_worker(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;
        if self
            .agent_preview_button_flash
            .is_some_and(|(_, deadline)| now >= deadline)
        {
            self.agent_preview_button_flash = None;
            changed = true;
        }
        if let Some(marquee) = &mut self.footer_marquee
            && now >= marquee.next_frame
        {
            marquee.next_frame = now + FOOTER_MARQUEE_STEP;
            changed = true;
        }
        changed |= self.mode == Mode::Explorer && self.workspace_explorer.poll_index();
        changed |= self.file_search.poll(self.session.data());
        if self.herdr_available() {
            let herdr_poll = {
                let _activity = diagnostics::activity("poll-herdr-session", "");
                self.herdr.poll()
            };
            changed |= herdr_poll.changed;
            if let Some(error) = herdr_poll.notice {
                self.notice = Some(error);
            }
            if let Some(result) = herdr_poll.fullscreen_result {
                let pending = self.pending_fullscreen_agent.take();
                if result == Ok(false)
                    && let Some(index) =
                        pending.as_ref().and_then(|key| self.herdr.agent_index(key))
                {
                    self.show_agent(index);
                }
            }
            if let Some(path) = herdr_poll.reopen_path {
                diagnostics::event(format!("opening repository path={}", path.display()));
                self.queue_workspace_restore(path);
            }
        }
        if self
            .linked_worktrees
            .observe_herdr(self.herdr.linked_worktree_observation())
        {
            self.linked_worktrees.refresh();
        }
        let catalog_poll = {
            let _activity = diagnostics::activity("poll-worktree-catalog", "");
            self.linked_worktrees.poll()
        };
        if catalog_poll.changed {
            let details = self.repository_picker_details();
            self.header_picker.sync_repository_details(&details);
        }
        changed |= catalog_poll.changed;
        if let Some(notice) = catalog_poll.notice {
            self.notice = Some(notice);
        }
        self.prefetch_commit_summaries();
        changed |= self.commit_summaries.poll();
        if self.issues.poll() {
            if self.header_picker.kind == Some(HeaderPickerKind::Issues) {
                self.refresh_header_issue_items();
            }
            changed = true;
        }
        changed |= self.commit_input.poll_blink(self.mode == Mode::Commit);
        changed |= self
            .file_search
            .query
            .poll_blink(self.mode == Mode::Normal && self.view == View::RepositorySearch);
        let naming_branch = self.header_picker.naming_branch();
        changed |= self.header_picker.branch_name.poll_blink(naming_branch);
        let cloning_repository = self.header_picker.cloning_repository();
        changed |= self.header_picker.clone_directory.poll_blink(
            cloning_repository && self.header_picker.clone_field == CloneField::Directory,
        );
        changed |= self
            .header_picker
            .clone_url
            .poll_blink(cloning_repository && self.header_picker.clone_field == CloneField::Url);
        let creating_worktree = self.header_picker.creating_worktree();
        changed |= self
            .header_picker
            .worktree_name
            .poll_blink(creating_worktree);
        let filtering_header_picker = self.header_picker.filtering();
        changed |= self.header_picker.query.poll_blink(filtering_header_picker);
        changed |= self.header_picker.poll_change_details();
        if let Some(result) = self.header_picker.poll_clone() {
            changed = true;
            match result {
                Ok(path) => {
                    self.notice = Some(format!("Cloned {}; opening workspace…", path.display()));
                    self.queue_workspace_restore(path);
                    self.linked_worktrees.refresh();
                }
                Err(error) => self.notice = Some(format!("Could not clone repository: {error}")),
            }
        }
        if let Some(result) = self.header_picker.poll_worktree_creation() {
            changed = true;
            match result {
                Ok(path) => {
                    self.notice = Some(format!("Created {}; opening workspace…", path.display()));
                    self.queue_workspace_restore(path);
                    self.linked_worktrees.refresh();
                }
                Err(error) => self.notice = Some(format!("Could not create worktree: {error}")),
            }
        }
        if let Some(result) = self.header_picker.poll_worktree_deletion() {
            changed = true;
            match result {
                Ok(path) => {
                    self.notice = Some(format!("Deleted worktree {}", path.display()));
                    self.linked_worktrees.refresh();
                }
                Err(error) => self.notice = Some(format!("Could not delete worktree: {error}")),
            }
        }
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
        let HerdrPromptPoll {
            changed: herdr_changed,
            completion,
        } = self.herdr_prompt.poll();
        changed |= herdr_changed;
        if let Some(completion) = completion {
            match completion {
                Ok(completion) => {
                    if self.mode == Mode::HerdrPrompt {
                        self.mode = Mode::Normal;
                    }
                    if let Some(path) = completion.reopen_path {
                        diagnostics::event(format!(
                            "opening repository for new agent path={}",
                            path.display()
                        ));
                        self.queue_workspace_restore(path);
                    }
                    self.notice = Some(completion.message);
                }
                Err(error) if self.mode == Mode::HerdrPrompt => {
                    self.herdr_prompt.error = Some(error);
                }
                Err(error) => self.notice = Some(error),
            }
        }
        if let Some(completion) = self.commit_message_generator.poll() {
            changed = true;
            self.receive_generated_commit_message(completion);
        }
        changed |= self.commit_message_generator.poll_spinner(Instant::now());
        changed |= {
            let _activity = diagnostics::activity("poll-commit-draft", "");
            self.flush_commit_draft_if_due()
        };
        if let Some(dialog) = &mut self.file_dialog {
            changed |= dialog.input.poll_blink(
                self.mode == Mode::Files && matches!(dialog.kind, FileDialogKind::Name { .. }),
            );
        }
        let interval = self.settings.fetch_interval();
        let session_worker_activity = diagnostics::activity("poll-session-workers", "");
        self.session
            .maybe_start_fetch(self.settings.auto_fetch, interval);
        self.session.maybe_start_status_check();
        while let Some(done) = self.session.next_worker_completion(interval) {
            changed = true;
            if let Some(request) = done.refresh_request() {
                self.track_refresh_request(request, true);
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
                            insert_recent_fetch(&mut self.recent_fetches, root, Instant::now());
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
                WorkerOutcome::Format(done) => {
                    let refresh_error = self
                        .file_editor
                        .as_ref()
                        .is_some_and(|editor| editor.path() == &done.path)
                        .then(|| {
                            self.file_editor
                                .as_mut()
                                .and_then(|editor| editor.refresh_from_disk().err())
                        })
                        .flatten();
                    if let Some(error) = refresh_error {
                        self.notice = Some(format!(
                            "Formatter completed for {}, but the editor could not refresh: {error}",
                            done.path
                        ));
                        continue;
                    }
                    match done.result {
                        Ok(output) if output.success => {
                            self.notice =
                                Some(format!("Formatted {} with {}", done.path, done.formatter));
                        }
                        Ok(output) => {
                            let fallback =
                                format!("{} could not format {}", done.formatter, done.path);
                            self.notice = Some(if output.stderr.trim().is_empty() {
                                first_error(&output.stdout, &fallback)
                            } else {
                                first_error(&output.stderr, &fallback)
                            });
                        }
                        Err(error) => self.notice = Some(error),
                    }
                }
                WorkerOutcome::BranchCheckout(done) => match done.result {
                    Ok(output) if output.success => {
                        self.notice = Some(format!("Checked out {}", done.branch));
                    }
                    Ok(output) => {
                        self.notice = Some(first_error(&output.stderr, "Branch checkout failed"));
                    }
                    Err(error) => self.notice = Some(error),
                },
                WorkerOutcome::BranchCreate(done) => match done.result {
                    Ok(output) if output.success => {
                        self.notice = Some(format!("Created and checked out {}", done.branch));
                    }
                    Ok(output) => {
                        self.notice = Some(first_error(&output.stderr, "Branch creation failed"));
                    }
                    Err(error) => self.notice = Some(error),
                },
                WorkerOutcome::BranchDelete(done) => match done.result {
                    Ok(output) if output.success => {
                        self.notice = Some(format!("Deleted branch {}", done.branch));
                    }
                    Ok(output) => {
                        self.notice = Some(first_error(&output.stderr, "Branch deletion failed"));
                    }
                    Err(error) => self.notice = Some(error),
                },
            }
        }
        while let Some(request) = self.session.next_worktree_change(interval) {
            changed = true;
            self.track_refresh_request(request, true);
            self.notice = None;
        }
        drop(session_worker_activity);
        if self.session.open_running() {
            self.notice = Some("Opening workspace…".to_owned());
        }
        let session_load_activity = diagnostics::activity("poll-session-loads", "");
        while let Some(done) = self.session.next_load_completion() {
            changed = true;
            let prepared_file_tree = done.prepared_file_tree;
            let follow_up_refresh = done.follow_up_refresh;
            let inventory_refresh = done.inventory_refresh;
            match (done.kind, done.result) {
                (LoadKind::Open, Ok(())) => {
                    if let (Some(state), Some(repository)) =
                        (&self.workspace_state, self.session.data())
                        && let Err(error) = state.save(&repository.root)
                    {
                        diagnostics::event(format!(
                            "workspace state save failed path={} error={error}",
                            repository.root.display()
                        ));
                    }
                    let remember_error = self.session.data().and_then(|repository| {
                        self.linked_worktrees
                            .remember_workspace(repository.common_dir.as_deref(), &repository.root)
                            .err()
                    });
                    self.linked_worktrees.refresh();
                    let _activity =
                        diagnostics::activity("apply-workspace", self.diagnostic_context());
                    diagnostics::event(format!("workspace opened {}", self.diagnostic_context()));
                    self.pending_reload = None;
                    self.mode = Mode::Normal;
                    self.actions = ActionsState::default();
                    self.graph_hidden = false;
                    let local = self.session.data().is_some_and(RepositoryData::is_local);
                    self.graph_state = TableState::default();
                    self.graph_scroll_to_selection = true;
                    self.graph_search_focused = false;
                    if let Some(repo) = self.session.data() {
                        self.author_filter.sync(&repo.root, &repo.commits);
                        self.graph_search.sync(
                            &repo.root,
                            &repo.commits,
                            self.author_filter.visible_indices(),
                        );
                    }
                    self.changes
                        .reset_repository(self.session.data(), prepared_file_tree);
                    self.initial_pane_pending = self
                        .session
                        .data()
                        .is_some_and(|repository| !repository.details_ready);
                    if let Some(path) = self.pending_file_selection.take() {
                        self.show_left_pane(LeftPane::Files);
                        let viewport = self
                            .regions
                            .explorer_list
                            .map_or(0, |rect| usize::from(rect.height));
                        if let Some(repo) = self.session.data() {
                            self.changes.select_explorer_path(repo, &path, viewport);
                        }
                    } else {
                        self.show_main_pane();
                    }
                    self.file_search.invalidate();
                    self.graph_state.select(
                        self.session
                            .data()
                            .is_some_and(|repo| !repo.commits.is_empty())
                            .then_some(0),
                    );
                    self.restore_commit_draft();
                    if let Some(request) = follow_up_refresh {
                        self.track_refresh_request(request, false);
                    }
                    self.notice = remember_error.or_else(|| {
                        Some(
                            if local {
                                "Workspace opened; indexing files…"
                            } else {
                                "Repository opened; loading details…"
                            }
                            .to_owned(),
                        )
                    });
                }
                (LoadKind::Open, Err(error)) => {
                    diagnostics::event(format!("workspace open failed error={error}"));
                    self.workspace_fetch_pending = false;
                    self.pending_file_selection = None;
                    let message = format!("Could not open workspace: {error}");
                    if self
                        .session
                        .data()
                        .is_some_and(|repository| !repository.details_ready)
                    {
                        if let Some(request) = follow_up_refresh {
                            self.track_refresh_request(request, false);
                        }
                    } else if self.session.data().is_none() {
                        self.initial_pane_pending = false;
                        self.mode = Mode::Explorer;
                    }
                    self.notice = Some(message.clone());
                    self.workspace_explorer.error = Some(message);
                }
                (LoadKind::Reload, Ok(())) => {
                    if let Some((selection, selected_oid)) = self.pending_reload.take() {
                        let repo = self.session.data().expect("reloaded repository");
                        self.author_filter.sync(&repo.root, &repo.commits);
                        self.graph_search.sync(
                            &repo.root,
                            &repo.commits,
                            self.author_filter.visible_indices(),
                        );
                        let visible = self.graph_search.visible_indices();
                        let commit_index = selected_oid.and_then(|oid| {
                            visible
                                .iter()
                                .position(|index| repo.commits[*index].oid == oid)
                        });
                        self.graph_state
                            .select(commit_index.or_else(|| repo.commits.first().map(|_| 0)));
                        self.graph_scroll_to_selection = true;
                        self.changes
                            .restore_selection(repo, selection, inventory_refresh);
                        if let Some(path) = self.pending_file_selection.take() {
                            let viewport = self
                                .regions
                                .explorer_list
                                .map_or(0, |rect| usize::from(rect.height));
                            self.changes.select_explorer_path(repo, &path, viewport);
                        }
                    }
                    if let Some(repo) = self.session.data() {
                        if self.initial_pane_pending && repo.details_ready {
                            let pane = ChangesState::initial_pane(Some(repo));
                            self.changes.set_pane(pane, Some(repo));
                            self.initial_pane_pending = false;
                        }
                        if self.view == View::RepositorySearch {
                            self.file_search.repository_refreshed(repo);
                        } else {
                            self.file_search.invalidate();
                        }
                    }
                    if self.notice.as_deref() == Some("Refreshing…") {
                        self.notice = Some("Refreshed".to_owned());
                    } else if self
                        .session
                        .data()
                        .is_some_and(|repository| repository.details_ready)
                        && (self.notice.as_deref() == Some("Repository opened; loading details…")
                            || self.notice.as_deref() == Some("Workspace opened; indexing files…")
                            || self.notice.as_deref().is_some_and(|notice| {
                                notice.ends_with(" (retrying queued refresh…)")
                            }))
                    {
                        self.notice = Some(
                            if self.session.data().is_some_and(RepositoryData::is_local) {
                                "Workspace ready"
                            } else {
                                "Repository ready"
                            }
                            .to_owned(),
                        );
                    }
                    if let Some(request) = follow_up_refresh {
                        self.track_refresh_request(request, true);
                    }
                }
                (LoadKind::Reload, Err(error)) => {
                    self.pending_reload = None;
                    if let Some(request) = follow_up_refresh {
                        self.track_refresh_request(request, false);
                        self.notice = Some(format!("{error} (retrying queued refresh…)"));
                    } else {
                        self.initial_pane_pending = false;
                        self.notice = Some(match self.session.data() {
                            Some(repository) if !repository.details_ready => {
                                if repository.is_local() {
                                    format!(
                                        "Could not index workspace files: {error} (press r to retry)"
                                    )
                                } else {
                                    format!(
                                        "Could not load repository details: {error} (press r to retry)"
                                    )
                                }
                            }
                            _ => error,
                        });
                    }
                }
            }
        }
        drop(session_load_activity);
        self.try_start_workspace_restore();
        self.maybe_start_workspace_fetch();
        changed |= self.changes.poll_directories(self.session.data());
        let preview_changed = self
            .changes
            .poll_preview(self.session.data().map(|repo| repo.root.as_path()));
        changed |= preview_changed;
        if preview_changed {
            self.restore_file_editor_scroll(true);
        }
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
        if self.visible_view() == View::Graph {
            let viewport = self
                .regions
                .graph_table
                .map_or(40, |region| usize::from(region.height));
            let visible = self.graph_search.visible_indices();
            oids.extend(
                visible
                    .iter()
                    .skip(self.graph_state.offset())
                    .take(viewport)
                    .map(|index| repo.commits[*index].oid.clone()),
            );
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
        if self.graph_commit_open
            && matches!(key.code, KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc)
        {
            self.show_previous_panel();
            return;
        }
        if self
            .settings
            .shortcuts
            .matches(ShortcutAction::DiscardChanges, key)
            && self.changes.pane == LeftPane::Worktree
        {
            self.open_discard_unstaged_dialog();
            return;
        }
        if let Some(index) = self.changes.hunk_selection {
            if self
                .settings
                .shortcuts
                .matches(ShortcutAction::StageSelection, key)
            {
                self.stage_hunk(index, true);
                return;
            }
            match key.code {
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => {
                    self.changes.leave_hunk_selection();
                }
                KeyCode::Down | KeyCode::Char('j') => self.scroll_or_move_hunk(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll_or_move_hunk(-1),
                KeyCode::Right | KeyCode::Char('l') => {
                    self.stage_hunk(index, true);
                }
                _ => {}
            }
            return;
        }
        if self.handle_normal_shortcut(key) {
            return;
        }
        if self.single_panel_detail_visible()
            && matches!(key.code, KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc)
        {
            self.show_previous_panel();
            return;
        }
        if self.single_panel_detail_visible() && self.agents_pane_visible() {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.scroll_agent_preview_by(1);
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.scroll_agent_preview_by(-1);
                    return;
                }
                _ => {}
            }
        }
        if self.single_panel_layout()
            && self.agents_pane_visible()
            && !self.single_panel_detail_open
        {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_agent_panel_selection(1);
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_agent_panel_selection(-1);
                    return;
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    if let Some(index) = self.agents_pane_index() {
                        self.open_agent_detail(index);
                    }
                    return;
                }
                _ => {}
            }
        }
        if self.view == View::Changes
            && self.changes.pane == LeftPane::Files
            && self.changes.sqlite_active()
        {
            self.handle_sqlite_browser_key(key);
            return;
        }
        if self.single_panel_detail_visible() {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.scroll_diff_by(1);
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.scroll_diff_by(-1);
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Enter if self.visible_view() == View::Graph && !self.graph_commit_open => {
                self.open_selected_graph_commit();
            }
            KeyCode::Enter
                if self.view == View::Changes
                    && self.changes.pane == LeftPane::Files
                    && self.changes.activate_sqlite() =>
            {
                self.show_detail_panel();
            }
            KeyCode::Enter
                if self.view == View::Changes && self.changes.pane == LeftPane::Files =>
            {
                if self.changes.selected_explorer_directory_path().is_some() {
                    let repo = self.session.data();
                    self.changes.toggle_selected_explorer_directory(repo);
                } else if self.selected_explorer_file_path().is_some() {
                    self.show_detail_panel();
                }
            }
            KeyCode::Enter
                if self.view == View::Changes && self.changes.pane == LeftPane::Worktree =>
            {
                let directory_selected = self
                    .session
                    .data()
                    .is_some_and(|repo| self.changes.selected_directory_path(repo).is_some());
                if directory_selected {
                    let repo = self.session.data();
                    self.changes.toggle_selected_directory(repo);
                } else if self.changes.worktree_state.selected().is_some() {
                    self.show_detail_panel();
                }
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.view == View::Changes && self.changes.pane == LeftPane::Files =>
            {
                let repo = self.session.data();
                self.changes.expand_or_descend_explorer(repo);
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.view == View::Changes && self.changes.pane == LeftPane::Worktree =>
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
                if self.view == View::Changes && self.changes.pane == LeftPane::Worktree =>
            {
                let repo = self.session.data();
                self.changes.collapse_or_ascend_worktree(repo);
            }
            KeyCode::PageDown if self.visible_view() == View::Changes || self.graph_commit_open => {
                self.scroll_diff_by(10)
            }
            KeyCode::PageUp if self.visible_view() == View::Changes || self.graph_commit_open => {
                self.scroll_diff_by(-10)
            }
            KeyCode::Down | KeyCode::Char('j') if self.graph_commit_open => self.scroll_diff_by(1),
            KeyCode::Up | KeyCode::Char('k') if self.graph_commit_open => self.scroll_diff_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Home => self.select_first(),
            KeyCode::End => self.select_last(),
            _ => {}
        }
    }

    fn handle_normal_shortcut(&mut self, key: KeyEvent) -> bool {
        let Some(action) = self
            .settings
            .shortcuts
            .main_action(key, self.herdr_available())
        else {
            return false;
        };
        match action {
            ShortcutAction::ToggleFullscreen
            | ShortcutAction::ShowChanges
            | ShortcutAction::ShowFiles
            | ShortcutAction::ShowAgents
            | ShortcutAction::ToggleGraph
            | ShortcutAction::FindFile => {
                return false;
            }
            ShortcutAction::Quit if self.format_running() => {
                self.notice = Some("A formatter is still running".to_owned());
            }
            ShortcutAction::Quit if self.commit_running() || self.session.command_running() => {
                self.notice = Some("A Git operation is still running".to_owned());
            }
            ShortcutAction::Quit => self.should_quit = true,
            ShortcutAction::OpenHerdr => self.open_herdr_prompt(),
            ShortcutAction::StartAgent => self.start_header_agent(),
            ShortcutAction::Refresh => self.reload(RefreshScope::ALL),
            ShortcutAction::OpenExplorer => self.open_explorer(),
            ShortcutAction::OpenSettings => self.open_settings(),
            ShortcutAction::OpenActions => self.open_actions(),
            ShortcutAction::OpenGitCommand => self.open_git_command(),
            ShortcutAction::OpenHelp => self.mode = Mode::Help,
            ShortcutAction::ToggleWrap
                if (self.visible_view() == View::Changes || self.graph_commit_open)
                    && self.changes.preview.wrappable() =>
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
            ShortcutAction::ToggleMarkdown => self.toggle_markdown_preview(),
            ShortcutAction::RenameFile if self.changes.pane == LeftPane::Files => {
                self.open_rename_dialog();
            }
            ShortcutAction::DeleteFile if self.changes.pane == LeftPane::Files => {
                self.open_delete_dialog();
            }
            ShortcutAction::EditFile => self.open_selected_file(false),
            ShortcutAction::ConfigureEditor => self.open_selected_file(true),
            ShortcutAction::FocusCommit => {
                self.show_left_pane(LeftPane::Worktree);
                self.focus_commit();
            }
            ShortcutAction::ToggleAgents => {
                self.dragging_agents = false;
                self.notice = Some(
                    if !self.agents_visible {
                        self.agents_visible = true;
                        self.herdr.show_live_agents();
                        "Agents shown"
                    } else if !self.herdr.showing_stash {
                        self.herdr.toggle_stash();
                        "Agent stash shown"
                    } else {
                        self.agents_visible = false;
                        self.herdr.show_live_agents();
                        "Agents hidden"
                    }
                    .to_owned(),
                );
            }
            ShortcutAction::UnstageAll if self.changes.pane == LeftPane::Worktree => {
                self.unstage_all();
            }
            ShortcutAction::StageSelection if self.changes.pane == LeftPane::Worktree => {
                self.toggle_stage();
            }
            ShortcutAction::SaveOrFormat if self.changes.pane == LeftPane::Files => {
                self.format_selected_file();
            }
            ShortcutAction::DiscardChanges => return false,
            _ => {}
        }
        true
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
        let focus = self.changes.preview.database().map(|browser| browser.focus);
        match key.code {
            KeyCode::Esc => self.changes.deactivate_sqlite(),
            KeyCode::BackTab => self.changes.toggle_sqlite_focus(),
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
            KeyCode::End => {
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
        if self
            .settings
            .shortcuts
            .matches(ShortcutAction::SubmitCommit, key)
        {
            self.start_commit();
            return;
        }
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
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.commit_input.delete_word();
            }
            KeyCode::Up => self.commit_input.move_up(input_width),
            KeyCode::Down => self.commit_input.move_down(input_width),
            KeyCode::Enter => self.commit_input.insert_char('\n'),
            _ => {
                self.commit_input.handle_edit_key(key);
            }
        }
        if self.commit_input.text() != previous {
            self.schedule_commit_draft();
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
        self.changes.preview_commit(repo, &commit);
        self.graph_commit_open = true;
    }

    fn queue_editor(&mut self) {
        if self.editor_configure_only {
            let command = self.editor_input.trim().to_owned();
            match parse_command_args(&command) {
                Ok(_) => {
                    self.settings.editor_command = Some(command);
                    self.persist_settings();
                    self.editor_error = None;
                    self.open_settings();
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
        if self.changes.preview.issue().is_some() {
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

    pub(crate) fn toggle_header_picker(&mut self, kind: HeaderPickerKind) {
        if self.mode != Mode::Normal || self.session.open_running() {
            return;
        }
        if self.header_picker.kind == Some(kind) {
            self.header_picker.close();
            return;
        }
        match kind {
            HeaderPickerKind::Repositories => self.open_header_repositories(),
            HeaderPickerKind::Worktrees => self.open_header_worktrees(),
            HeaderPickerKind::Branches => self.open_header_branches(),
            HeaderPickerKind::DiffTargets => self.open_header_diff_targets(),
            HeaderPickerKind::Issues => self.open_header_issues(),
        }
    }

    pub(crate) fn begin_header_branch_creation(&mut self) {
        if self.session.can_start_mutation() {
            self.open_header_branch_bases();
        } else {
            self.header_picker.message = Some("Wait for the current Git operation".to_owned());
        }
    }

    pub(crate) fn activate_header_picker(&mut self, index: usize) {
        let Some(item) = self.header_picker.items.get(index).cloned() else {
            return;
        };
        if self.header_picker.selecting_worktree_base()
            && let HeaderPickerItem::BranchBase(branch) = item
        {
            self.create_header_worktree(branch);
            return;
        }
        if let HeaderPickerItem::BranchBase(branch) = item {
            self.header_picker.open_branch_name(branch);
            return;
        }
        self.header_picker.close();
        match item {
            HeaderPickerItem::BranchBase(_) => unreachable!(),
            HeaderPickerItem::Repository { path, .. } => {
                if self
                    .repository()
                    .is_some_and(|repository| same_path(&repository.root, &path))
                {
                    return;
                }
                self.open_header_path(path);
            }
            HeaderPickerItem::Worktree { worktree, .. } => {
                if self
                    .repository()
                    .is_some_and(|repository| same_path(&repository.root, &worktree.path))
                {
                    return;
                }
                self.open_header_path(worktree.path);
            }
            HeaderPickerItem::Branch(branch) => {
                if branch.current {
                    self.notice = Some(format!("{} is already checked out", branch.name));
                    return;
                }
                let branch_name = branch.name;
                if self
                    .session
                    .start_branch_checkout(branch_name.clone(), branch.remote)
                {
                    self.changes.clear_branch_comparison();
                    self.notice = Some(format!("Checking out {branch_name}…"));
                } else {
                    self.notice = Some("Another repository operation is running".to_owned());
                }
            }
            HeaderPickerItem::DiffTarget {
                label, revision, ..
            } => {
                let Some(repository) = self.git_repository() else {
                    return;
                };
                let root = repository.root.clone();
                let current = repository.branch.clone();
                let current_revision = repository
                    .branches
                    .iter()
                    .find(|branch| branch.current)
                    .map(Branch::revision)
                    .or_else(|| repository.history.first().map(|commit| commit.oid.clone()))
                    .unwrap_or_else(|| "HEAD".to_owned());
                self.show_left_pane(LeftPane::Worktree);
                self.changes
                    .preview_branch_diff(&root, current, label, current_revision, revision);
            }
            HeaderPickerItem::Issue { number, .. } => {
                let Some(issue) = self.issues.issue(number) else {
                    return;
                };
                self.changes.show_issue(
                    issue.number,
                    issue.kind_label(),
                    &issue.title,
                    &issue.body,
                );
                self.initial_pane_pending = false;
                self.dismiss_agent_preview();
                self.show_main_pane();
            }
        }
    }

    fn reload(&mut self, scope: RefreshScope) {
        let Some(request) = self
            .session
            .request_refresh(scope, self.settings.fetch_interval())
        else {
            return;
        };
        self.track_refresh_request(request, true);
    }

    fn track_refresh_request(&mut self, request: RefreshRequest, show_notice: bool) {
        let Some(repo) = self.repository() else {
            return;
        };
        let selection = self.changes.capture_selection(repo);
        let details_ready = repo.details_ready;
        let local = repo.is_local();
        let selected_oid = self
            .selected_graph_commit()
            .map(|commit| commit.oid.clone());

        self.pending_reload = Some((selection, selected_oid));
        if request == RefreshRequest::Started && show_notice {
            self.notice = Some(
                if details_ready {
                    "Refreshing…"
                } else if local {
                    "Indexing workspace files…"
                } else {
                    "Loading repository details…"
                }
                .to_owned(),
            );
        }
    }

    pub(crate) fn selected_explorer_file_path(&self) -> Option<&RepoPath> {
        self.changes
            .selected_explorer_file_path(self.session.data()?)
    }

    pub(crate) fn markdown_preview_available(&self) -> bool {
        self.view == View::Changes && self.changes.preview.markdown_available()
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

    fn toggle_fullscreen(&mut self) {
        if let Err(error) = self.herdr.toggle_fullscreen() {
            self.notice = Some(format!("Could not toggle fullscreen: {error}"));
        }
    }

    fn handle_main_navigation(&mut self, key: KeyEvent) -> bool {
        match self
            .settings
            .shortcuts
            .main_action(key, self.herdr_available())
        {
            Some(ShortcutAction::ShowChanges) => self.show_sidebar_pane(LeftPane::Worktree),
            Some(ShortcutAction::ShowFiles) => self.show_sidebar_pane(LeftPane::Files),
            Some(ShortcutAction::ShowAgents) => self.show_agents_pane(),
            Some(ShortcutAction::ToggleGraph) if self.mode == Mode::Normal => self.toggle_graph(),
            _ => return false,
        }
        true
    }

    pub(super) fn show_main_pane(&mut self) {
        self.view = View::Changes;
        self.graph_commit_open = false;
        self.single_panel_detail_open = false;
    }

    pub(crate) fn single_panel_layout(&self) -> bool {
        self.regions
            .screen
            .is_some_and(|area| area.width < SPLIT_VIEW_MIN_WIDTH)
    }

    pub(crate) fn single_panel_detail_visible(&self) -> bool {
        self.single_panel_layout() && self.single_panel_detail_open
    }

    pub(super) fn show_detail_panel(&mut self) {
        self.show_main_pane();
        self.single_panel_detail_open = true;
    }

    pub(super) fn show_previous_panel(&mut self) {
        if self.graph_commit_open {
            self.graph_commit_open = false;
        } else {
            self.changes.deactivate_sqlite();
            self.single_panel_detail_open = false;
            self.agent_preview_transcript_scroll = None;
            self.agent_preview_message_selection = None;
            self.agent_preview_expanded_requests = None;
        }
    }

    fn move_agent_panel_selection(&mut self, delta: isize) {
        if self.herdr.showing_stash || self.herdr.agents.is_empty() {
            self.herdr.scroll_agents(delta);
            return;
        }
        let current = self.agents_pane_index().unwrap_or(0);
        let index = current
            .saturating_add_signed(delta)
            .min(self.herdr.agents.len() - 1);
        if index != current {
            self.herdr.agent_scroll = self.herdr.agent_card_index(index).unwrap_or(0);
        }
        self.select_agent_preview(index);
    }

    fn open_agent_detail(&mut self, index: usize) {
        self.select_agent_preview(index);
        self.single_panel_detail_open = true;
    }

    fn show_left_pane(&mut self, pane: LeftPane) {
        self.initial_pane_pending = false;
        self.dismiss_agent_preview();
        self.changes.set_pane(pane, self.session.data());
        self.show_main_pane();
    }

    fn show_sidebar_pane(&mut self, pane: LeftPane) {
        self.initial_pane_pending = false;
        self.dismiss_agent_preview();
        self.changes.set_pane_preserving_preview(pane);
        self.single_panel_detail_open = false;
    }

    fn dismiss_agent_preview(&mut self) {
        self.agents_pane_pinned = false;
        self.agent_preview_selection = None;
        self.agent_preview_transcript_scroll = None;
        self.agent_preview_message_selection = None;
        self.agent_preview_expanded_requests = None;
        self.agent_preview_picker_open = false;
        self.agent_preview_button_flash = None;
        if matches!(
            self.hovered_hit_target,
            Some(
                HitTarget::Agent(_)
                    | HitTarget::AgentStashToggle
                    | HitTarget::AgentStash(_)
                    | HitTarget::StashedAgent(_)
                    | HitTarget::AgentPreviewPicker(_)
                    | HitTarget::AgentPreviewPickerItem(_)
                    | HitTarget::AgentPreviewPrevious(_)
                    | HitTarget::AgentPreviewNext(_)
                    | HitTarget::AgentPreviewMessageTimeline(_)
                    | HitTarget::AgentPreviewRequest { .. }
                    | HitTarget::AgentTooltip { .. }
                    | HitTarget::AgentMessage { .. }
            )
        ) {
            self.hovered_hit_target = None;
        }
    }

    fn stash_agent(&mut self, index: usize) {
        let Some(path) = self.herdr.agent_destination(index).map(Path::to_path_buf) else {
            self.notice =
                Some("Could not stash agent: working directory was not reported".to_owned());
            return;
        };
        let destination = self.linked_worktrees.agent_destination(&path);
        let repository = destination.as_ref().map_or_else(
            || {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            },
            |destination| destination.repository().to_owned(),
        );
        let branch = destination
            .as_ref()
            .map_or("unknown", AgentDestinationMetadata::branch)
            .to_owned();
        let repository_root = destination.as_ref().map_or_else(
            || path.clone(),
            |destination| destination.repository_root().to_owned(),
        );
        match self
            .herdr
            .stash_agent(index, repository_root, repository, branch)
        {
            Ok(()) => self.notice = Some("Closing and stashing agent".to_owned()),
            Err(error) => self.notice = Some(format!("Could not stash agent: {error}")),
        }
    }

    fn restore_stashed_agent(&mut self, index: usize) {
        let Some(agent) = self.herdr.stashed_agents().get(index).cloned() else {
            self.notice = Some("Stashed agent is no longer available".to_owned());
            return;
        };
        self.herdr.show_live_agents();
        match self
            .herdr_prompt
            .prepare_stashed_agent(agent.worktree, agent.session_id)
        {
            Ok(()) => self.notice = Some("Loading active Herdr tab layout".to_owned()),
            Err(error) => self.notice = Some(format!("Could not restore agent: {error}")),
        }
    }

    pub(crate) fn agent_preview_button_flash(&self) -> Option<bool> {
        self.agent_preview_button_flash
            .filter(|(_, deadline)| Instant::now() < *deadline)
            .map(|(forward, _)| forward)
    }

    pub(crate) fn agent_preview_picker_open(&self) -> bool {
        self.agent_preview_picker_open
    }

    pub(crate) fn agent_preview_transcript_scroll(&self, index: usize) -> Option<usize> {
        let scroll = self.agent_preview_transcript_scroll.as_ref()?;
        let message = self.agent_preview_message(index).or_else(|| {
            self.herdr
                .agent_user_messages(index)
                .and_then(|messages| messages.len().checked_sub(1))
        })?;
        (self.herdr.agent_key(index).as_ref() == Some(&scroll.agent) && scroll.message == message)
            .then_some(scroll.offset)
    }

    pub(crate) fn agent_preview_swipe(&self, index: usize) -> Option<(i32, usize)> {
        let drag = self.mobile_scroll_drag.as_ref()?;
        let key = self.herdr.agent_key(index)?;
        if drag.agent_preview.as_ref() != Some(&key)
            || drag.axis == Some(MobileDragAxis::Vertical)
            || self.herdr.agents.len() < 2
        {
            return None;
        }
        let offset = i32::from(drag.previous.x) - i32::from(drag.start.x);
        let vertical = drag.start.y.abs_diff(drag.previous.y);
        if offset == 0 || (drag.axis.is_none() && offset.unsigned_abs() <= u32::from(vertical)) {
            return None;
        }
        let neighbor = if offset < 0 {
            (index + 1) % self.herdr.agents.len()
        } else {
            (index + self.herdr.agents.len() - 1) % self.herdr.agents.len()
        };
        Some((offset, neighbor))
    }

    pub(crate) fn agent_preview_message(&self, index: usize) -> Option<usize> {
        let selection = self
            .agent_preview_message_selection
            .as_ref()
            .filter(|selection| self.herdr.agent_key(index).as_ref() == Some(&selection.agent))?;
        let last = self
            .herdr
            .agent_user_messages(index)?
            .len()
            .checked_sub(1)?;
        Some(selection.message.min(last))
    }

    pub(crate) fn agent_preview_expanded_requests(&self, index: usize) -> &[usize] {
        let Some(agent) = self.herdr.agent_key(index) else {
            return &[];
        };
        let Some(message) = self.agent_preview_message(index).or_else(|| {
            self.herdr
                .agent_user_messages(index)
                .and_then(|messages| messages.len().checked_sub(1))
        }) else {
            return &[];
        };
        let Some(expanded) = self.agent_preview_expanded_requests.as_ref() else {
            return &[];
        };
        if expanded.agent == agent && expanded.message == message {
            &expanded.requests
        } else {
            &[]
        }
    }

    pub(super) fn show_agents_pane(&mut self) {
        if !self.herdr_available() {
            return;
        }
        let selection = self
            .agents_pane_index()
            .or_else(|| self.default_agent_preview_index())
            .and_then(|index| self.herdr.agent_key(index));
        self.initial_pane_pending = false;
        self.agents_visible = true;
        self.agents_pane_pinned = true;
        self.agent_preview_selection = selection;
        self.single_panel_detail_open = false;
    }

    fn show_graph(&mut self) {
        self.view = View::Graph;
        self.graph_commit_open = false;
        self.graph_hidden = false;
        self.single_panel_detail_open = false;
    }

    fn toggle_left_pane(&mut self) {
        if self.agents_pane_pinned {
            self.show_sidebar_pane(LeftPane::Worktree);
            return;
        }
        match self.changes.pane {
            LeftPane::Worktree => self.show_sidebar_pane(LeftPane::Files),
            LeftPane::Files if self.herdr_available() => self.show_agents_pane(),
            LeftPane::Files => self.show_sidebar_pane(LeftPane::Worktree),
        }
    }

    fn toggle_graph(&mut self) {
        if self.visible_view() == View::Graph {
            self.show_main_pane();
            self.graph_hidden = true;
            return;
        }
        if self.require_git_repository() {
            self.show_graph();
        }
    }
}

fn fetch_is_fresh(fetched_at: Option<&Instant>, now: Instant) -> bool {
    fetched_at.is_some_and(|fetched_at| {
        now.saturating_duration_since(*fetched_at) < WORKSPACE_FETCH_FRESHNESS
    })
}

fn insert_recent_fetch(
    recent_fetches: &mut HashMap<PathBuf, Instant>,
    root: PathBuf,
    now: Instant,
) {
    recent_fetches.retain(|_, fetched_at| fetch_is_fresh(Some(fetched_at), now));
    recent_fetches.insert(root, now);
}

fn first_error(stderr: &str, fallback: &str) -> String {
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

pub(crate) fn is_markdown_path(path: &RepoPath) -> bool {
    path.as_path()
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["md", "markdown", "mdown", "mkd", "mkdn"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
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
