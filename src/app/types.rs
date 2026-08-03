use super::*;

pub(crate) const APP_MIN_WIDTH: u16 = 40;
pub(crate) const SPLIT_VIEW_MIN_WIDTH: u16 = 60;

pub(crate) struct CommitDraftResult {
    pub(super) root: PathBuf,
    pub(super) result: Result<(PathBuf, Option<String>), String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Changes,
    Graph,
    RepositorySearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Commit,
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
    HeaderAgent,
    HeaderFullscreen,
    AgentPanePickerOverlay,
    AgentPane(usize),
    AgentPaneSplit(usize, AgentPaneDirection),
    HeaderPickerOverlay,
    HeaderPickerNewBranch,
    HeaderPickerOpenExplorer,
    HeaderPickerClone,
    HeaderPickerCloneDirectory,
    HeaderPickerCloneUrl,
    HeaderPickerNewWorktree,
    HeaderPickerWorktreeName,
    HeaderPickerDeleteWorktree(usize),
    HeaderPickerConfirmDeleteWorktree,
    HeaderPickerCancelDeleteWorktree,
    HeaderPickerItem(usize),
    Changes(ChangesHitTarget),
    CommitMessageGenerate,
    MarkdownPreviewToggle,
    Graph(GraphHitTarget),
    Explorer(ExplorerHitTarget),
    FileSearch(FileSearchHitTarget),
    Settings(SettingsHitTarget),
    Agent(usize),
    AgentStashToggle,
    AgentStash(usize),
    StashedAgent(usize),
    AgentPreviewPicker(usize),
    AgentPreviewPickerItem(usize),
    AgentPreviewPrevious(usize),
    AgentPreviewNext(usize),
    AgentTooltip { agent: usize, message: usize },
    AgentMessage { agent: usize, message: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsHitTarget {
    Overlay,
    Page(SettingsPage),
    Shortcut(ShortcutAction),
    OpenCodeModel,
    OpenCodeReasoning,
    AutoFetch,
    FetchInterval,
    FetchIntervalDown,
    FetchIntervalUp,
    FormatOnSave,
    CrossWorkspaceAgents,
    AgentHarness,
    AgentTime,
    ClearAgentTimings,
    MediaPreview,
    Editor,
}

impl SettingsHitTarget {
    pub(crate) fn from_general_index(index: usize) -> Option<Self> {
        [
            Self::AutoFetch,
            Self::FetchInterval,
            Self::FormatOnSave,
            Self::CrossWorkspaceAgents,
            Self::AgentHarness,
            Self::AgentTime,
            Self::ClearAgentTimings,
            Self::MediaPreview,
            Self::Editor,
        ]
        .get(index)
        .copied()
    }

    pub(crate) fn general_index(self) -> Option<usize> {
        match self {
            Self::AutoFetch => Some(0),
            Self::FetchInterval | Self::FetchIntervalDown | Self::FetchIntervalUp => Some(1),
            Self::FormatOnSave => Some(2),
            Self::CrossWorkspaceAgents => Some(3),
            Self::AgentHarness => Some(4),
            Self::AgentTime => Some(5),
            Self::ClearAgentTimings => Some(6),
            Self::MediaPreview => Some(7),
            Self::Editor => Some(8),
            Self::Overlay
            | Self::Page(_)
            | Self::Shortcut(_)
            | Self::OpenCodeModel
            | Self::OpenCodeReasoning => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileSearchHitTarget {
    Scope(SearchScope),
    CaseSensitive,
    WholeWord,
    Regex,
    IncludeIgnored,
    Result { generation: u64, row: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentPaneDirection {
    Up,
    Down,
    Left,
    Right,
}

impl AgentPaneDirection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
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
    pub action_menu: Option<Rect>,
    pub action_list: Option<Rect>,
    pub command_overlay: Option<Rect>,
    pub command_output: Option<Rect>,
    pub herdr_prompt_overlay: Option<Rect>,
    pub editor_overlay: Option<Rect>,
    pub file_search: Option<Rect>,
    pub file_search_list: Option<Rect>,
    pub files_add: Option<Rect>,
    pub files_root: Option<Rect>,
    pub file_dialog_overlay: Option<Rect>,
    pub file_dialog_primary: Option<Rect>,
    pub file_dialog_secondary: Option<Rect>,
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

    pub(crate) fn settings_shortcut_rows(&self) -> usize {
        self.hit_regions
            .iter()
            .filter(|region| {
                matches!(
                    region.target,
                    HitTarget::Settings(SettingsHitTarget::Shortcut(_))
                )
            })
            .count()
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
