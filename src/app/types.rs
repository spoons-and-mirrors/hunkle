use super::*;

pub(crate) struct CommitDraftResult {
    pub(super) root: PathBuf,
    pub(super) result: Result<(PathBuf, Option<String>), String>,
}

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
    AuthorFilter,
    ActionMenu,
    Command,
    HerdrPrompt,
    FileEdit,
    Editor,
    Files,
    WorkspacePanel,
    WorkspacePresets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExplorerTab {
    Explorer,
    Worktrees,
    Branches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    General,
    OpenCode,
    Shortcuts,
}

impl SettingsPage {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::General => Self::OpenCode,
            Self::OpenCode => Self::Shortcuts,
            Self::Shortcuts => Self::General,
        }
    }

    pub(crate) fn previous(self) -> Self {
        match self {
            Self::General => Self::Shortcuts,
            Self::OpenCode => Self::General,
            Self::Shortcuts => Self::OpenCode,
        }
    }
}

impl ExplorerTab {
    pub(crate) const ALL: [Self; 3] = [Self::Explorer, Self::Worktrees, Self::Branches];
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

#[derive(Debug, Clone)]
pub(crate) struct DiffFileHeaderRegion {
    pub(crate) rect: Rect,
    pub(crate) path: RepoPath,
    pub(crate) line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HitTarget {
    HeaderRepository,
    HeaderWorktrees,
    HeaderBranch,
    HeaderDiff,
    HeaderPickerOverlay,
    HeaderPickerNewBranch,
    HeaderPickerItem(usize),
    Changes(ChangesHitTarget),
    CommitMessageGenerate,
    MarkdownPreviewToggle,
    Graph(GraphHitTarget),
    ExplorerTab(ExplorerTab),
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
pub(crate) enum GraphColumn {
    Graph,
    Description,
    Changes,
    Date,
    Author,
    Commit,
}

impl GraphColumn {
    pub(crate) fn minimum_width(self) -> u16 {
        match self {
            Self::Graph => 2,
            Self::Description => 1,
            Self::Changes => 3,
            Self::Date => 4,
            Self::Author | Self::Commit => 3,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GraphColumnRegion {
    pub left: GraphColumn,
    pub right: GraphColumn,
    pub left_width: u16,
    pub right_width: u16,
    pub splitter: Rect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GraphColumnDrag {
    pub left: GraphColumn,
    pub right: GraphColumn,
    pub origin_x: u16,
    pub left_width: u16,
    pub right_width: u16,
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

#[derive(Debug, Clone)]
pub(crate) struct EditorRenderedRow {
    pub line: usize,
    pub columns: Vec<(usize, usize)>,
}

impl EditorRenderedRow {
    pub(crate) fn source_column_at(&self, column: usize) -> usize {
        self.columns
            .iter()
            .min_by_key(|(rendered, _)| rendered.abs_diff(column))
            .map_or(0, |(_, source)| *source)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Regions {
    pub screen: Option<Rect>,
    pub changes: Option<Rect>,
    pub graph: Option<Rect>,
    pub left_pane_toggle: Option<Rect>,
    pub explorer: Option<Rect>,
    pub settings: Option<Rect>,
    pub help: Option<Rect>,
    pub workspace_panel: Option<Rect>,
    pub workspace_panel_workspaces: Option<Rect>,
    pub workspace_panel_agents: Option<Rect>,
    pub workspace_presets_overlay: Option<Rect>,
    pub actions: Option<Rect>,
    pub worktree: Option<Rect>,
    pub worktree_list: Option<Rect>,
    pub explorer_list: Option<Rect>,
    pub agents_list: Option<Rect>,
    pub agents_splitter: Option<Rect>,
    pub agents_bounds: Option<Rect>,
    pub diff: Option<Rect>,
    pub preview_body: Option<Rect>,
    pub preview_path: Option<RepoPath>,
    pub preview_untracked: bool,
    pub preview_generation: u64,
    pub preview_scroll: usize,
    pub(crate) editor_rows: Vec<EditorRenderedRow>,
    pub(crate) diff_file_headers: Vec<DiffFileHeaderRegion>,
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
    pub(crate) graph_columns: Vec<GraphColumnRegion>,
    pub settings_overlay: Option<Rect>,
    pub settings_general_tab: Option<Rect>,
    pub settings_shortcuts_tab: Option<Rect>,
    pub settings_opencode_tab: Option<Rect>,
    pub shortcut_rows: Vec<(ShortcutAction, Rect)>,
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
    pub format_on_save_setting: Option<Rect>,
    pub opencode_model_setting: Option<Rect>,
    pub opencode_reasoning_setting: Option<Rect>,
    pub auto_fetch: Option<Rect>,
    pub workspace_panel_setting: Option<Rect>,
    pub agent_harness_setting: Option<Rect>,
    pub agent_time_setting: Option<Rect>,
    pub clear_agent_timings_setting: Option<Rect>,
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

pub(crate) struct FileEditorReturn {
    pub(super) path: RepoPath,
    pub(super) pane: LeftPane,
    pub(super) scroll: usize,
}

pub(crate) struct EditorRequest {
    pub(crate) command: Vec<String>,
    pub(crate) file: PathBuf,
    pub(crate) repository: PathBuf,
}
