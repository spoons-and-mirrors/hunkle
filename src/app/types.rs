use super::*;

pub(crate) const APP_MIN_WIDTH: u16 = 40;
pub(crate) const SPLIT_VIEW_MIN_WIDTH: u16 = 60;
pub(crate) const FOOTER_MARQUEE_STEP: Duration = Duration::from_millis(120);
pub(crate) const FOOTER_MARQUEE_PAUSE: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LayoutProfile {
    Single,
    #[default]
    Columns,
}

impl LayoutProfile {
    pub(crate) fn for_area(area: Rect) -> Self {
        if area.width < SPLIT_VIEW_MIN_WIDTH {
            Self::Single
        } else {
            Self::Columns
        }
    }

    pub(crate) fn is_single(self) -> bool {
        self == Self::Single
    }
}

pub(crate) struct FooterMarquee {
    pub(super) value: String,
    pub(super) width: usize,
    pub(super) started: Instant,
    pub(super) next_frame: Instant,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum WorkspaceSurface {
    #[default]
    Master,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceContent {
    Changes(WorkspaceSurface),
    Graph(WorkspaceSurface),
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkspaceNavigation {
    content: WorkspaceContent,
    search_return: WorkspaceContent,
    agents: Option<WorkspaceSurface>,
}

impl Default for WorkspaceNavigation {
    fn default() -> Self {
        let content = WorkspaceContent::Changes(WorkspaceSurface::Master);
        Self {
            content,
            search_return: content,
            agents: None,
        }
    }
}

impl WorkspaceNavigation {
    pub(crate) fn view(self) -> View {
        match self.content {
            WorkspaceContent::Changes(_) => View::Changes,
            WorkspaceContent::Graph(_) => View::Graph,
            WorkspaceContent::Search => View::RepositorySearch,
        }
    }

    pub(crate) fn showing(self, view: View) -> bool {
        self.view() == view
    }

    pub(crate) fn show_changes(&mut self) {
        self.content = WorkspaceContent::Changes(WorkspaceSurface::Master);
    }

    pub(crate) fn show_graph(&mut self) {
        self.content = WorkspaceContent::Graph(WorkspaceSurface::Master);
    }

    pub(crate) fn open_search(&mut self) {
        if !self.showing(View::RepositorySearch) {
            self.search_return = self.content;
            self.content = WorkspaceContent::Search;
        }
    }

    pub(crate) fn close_search(&mut self) {
        if self.showing(View::RepositorySearch) {
            self.content = self.search_return;
        }
    }

    pub(crate) fn changes_detail_open(self) -> bool {
        self.content == WorkspaceContent::Changes(WorkspaceSurface::Detail)
    }

    pub(crate) fn show_changes_detail(&mut self) {
        self.content = WorkspaceContent::Changes(WorkspaceSurface::Detail);
    }

    pub(crate) fn close_changes_detail(&mut self) {
        if self.changes_detail_open() {
            self.show_changes();
        }
    }

    pub(crate) fn graph_commit_open(self) -> bool {
        self.content == WorkspaceContent::Graph(WorkspaceSurface::Detail)
    }

    pub(crate) fn show_graph_commit(&mut self) {
        self.content = WorkspaceContent::Graph(WorkspaceSurface::Detail);
    }

    pub(crate) fn close_graph_commit(&mut self) {
        if self.graph_commit_open() {
            self.show_graph();
        }
    }

    pub(crate) fn agents_selected(self) -> bool {
        self.agents.is_some()
    }

    pub(crate) fn agent_detail_open(self) -> bool {
        self.agents == Some(WorkspaceSurface::Detail)
    }

    pub(crate) fn select_agents(&mut self) {
        self.close_changes_detail();
        self.agents = Some(WorkspaceSurface::Master);
    }

    pub(crate) fn show_agent_detail(&mut self) {
        self.agents = Some(WorkspaceSurface::Detail);
    }

    pub(crate) fn close_agent_detail(&mut self) {
        if self.agent_detail_open() {
            self.agents = Some(WorkspaceSurface::Master);
        }
    }

    pub(crate) fn select_sidebar(&mut self) {
        self.agents = None;
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;

    #[test]
    fn layout_profile_selects_composition_at_workspace_boundary() {
        assert!(LayoutProfile::for_area(Rect::new(0, 0, 59, 40)).is_single());
        assert!(!LayoutProfile::for_area(Rect::new(0, 0, 60, 40)).is_single());
    }

    #[test]
    fn workspace_navigation_preserves_primary_selection_across_layers() {
        let mut navigation = WorkspaceNavigation::default();
        navigation.select_agents();
        navigation.show_agent_detail();
        navigation.show_graph();
        navigation.show_graph_commit();
        navigation.open_search();

        assert!(navigation.showing(View::RepositorySearch));
        assert!(navigation.agents_selected());
        assert!(navigation.agent_detail_open());

        navigation.close_search();
        assert!(navigation.showing(View::Graph));
        assert!(navigation.agents_selected());
        assert!(navigation.agent_detail_open());
        assert!(navigation.graph_commit_open());

        navigation.close_graph_commit();
        assert!(!navigation.graph_commit_open());
        assert!(navigation.agent_detail_open());

        navigation.close_agent_detail();
        assert!(navigation.agents_selected());
        assert!(!navigation.agent_detail_open());
    }

    #[test]
    fn nested_scroll_target_owns_the_gesture() {
        let mut regions = Regions::default();
        regions.register_scroll_target(ScrollTarget::Preview, Rect::new(0, 0, 20, 20));
        regions.register_scroll_target(ScrollTarget::SqliteRows, Rect::new(5, 5, 10, 10));

        assert_eq!(
            regions.scroll_target_at(Position::new(7, 7)),
            Some(ScrollTarget::SqliteRows)
        );
        assert_eq!(
            regions.scroll_target_at(Position::new(2, 2)),
            Some(ScrollTarget::Preview)
        );
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HitTarget {
    HeaderRepository,
    HeaderWorktrees,
    HeaderBranch,
    HeaderDiff,
    HeaderIssue,
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
    HeaderPickerDeleteBranch(usize),
    HeaderPickerConfirmDeleteBranch,
    HeaderPickerCancelDeleteBranch,
    HeaderPickerDeleteWorktree(usize),
    HeaderPickerConfirmDeleteWorktree,
    HeaderPickerCancelDeleteWorktree,
    HeaderPickerItem(usize),
    HeaderPickerIssueScope,
    Changes(ChangesHitTarget),
    CommitMessageGenerate,
    MarkdownPreviewToggle,
    Graph(GraphHitTarget),
    Explorer(ExplorerHitTarget),
    FileSearch(FileSearchHitTarget),
    Settings(SettingsHitTarget),
    Agent(AgentKey),
    AgentStashToggle,
    AgentStash(AgentKey),
    StashedAgent(usize),
    AgentPreviewPicker(AgentKey),
    AgentPreviewPickerItem(AgentKey),
    AgentPreviewPrevious(AgentKey),
    AgentPreviewNext(AgentKey),
    AgentPreviewMessageTimeline(AgentKey),
    AgentPreviewRequest {
        agent: AgentKey,
        message: usize,
        request: usize,
    },
    AgentTooltip {
        agent: AgentKey,
        message: usize,
    },
    AgentMessage {
        agent: AgentKey,
        message: usize,
    },
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

#[derive(Clone, Debug)]
pub(crate) struct MobileScrollDrag {
    pub(crate) start: Position,
    pub(crate) previous: Position,
    pub(crate) moved: bool,
    pub(crate) axis: Option<MobileDragAxis>,
    pub(crate) agent_preview: Option<AgentKey>,
    pub(crate) scroll_target: Option<ScrollTarget>,
    pub(crate) modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MobileDragAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScrollTarget {
    HeaderPicker,
    Commit,
    Worktree,
    Explorer,
    Agents,
    Preview,
    SqliteObjects,
    SqliteRows,
    Graph,
    RepositorySearch,
    AgentTimeline(AgentKey),
    AgentTranscript(AgentKey),
}

#[derive(Clone, Debug)]
struct ScrollRegion {
    target: ScrollTarget,
    rect: Rect,
}

#[derive(Debug, Clone)]
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
    pub(crate) agent_preview_scroll: usize,
    pub(crate) agent_preview_scroll_max: usize,
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
    scroll_regions: Vec<ScrollRegion>,
}

impl Regions {
    pub(crate) fn register_hit_target(&mut self, target: HitTarget, rect: Rect) {
        self.hit_regions.push(HitRegion { target, rect });
    }

    pub(crate) fn register_scroll_target(&mut self, target: ScrollTarget, rect: Rect) {
        self.scroll_regions.push(ScrollRegion { target, rect });
    }

    pub(crate) fn scroll_target_at(&self, point: Position) -> Option<ScrollTarget> {
        let semantic_target = match self.hit_target_at(point) {
            Some(
                HitTarget::HeaderPickerOverlay
                | HitTarget::HeaderPickerItem(_)
                | HitTarget::HeaderPickerDeleteBranch(_)
                | HitTarget::HeaderPickerDeleteWorktree(_),
            ) => Some(ScrollTarget::HeaderPicker),
            Some(HitTarget::AgentPreviewMessageTimeline(agent)) => {
                Some(ScrollTarget::AgentTimeline(agent))
            }
            Some(
                HitTarget::AgentTooltip { agent, .. }
                | HitTarget::AgentMessage { agent, .. }
                | HitTarget::AgentPreviewRequest { agent, .. },
            ) => Some(ScrollTarget::AgentTranscript(agent)),
            _ => None,
        };
        semantic_target.or_else(|| {
            self.scroll_regions
                .iter()
                .rev()
                .find(|region| region.rect.contains(point))
                .map(|region| region.target.clone())
        })
    }

    pub(crate) fn hit_target_at(&self, point: Position) -> Option<HitTarget> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| region.rect.contains(point))
            .map(|region| region.target.clone())
    }

    pub(crate) fn settings_shortcut_rows(&self) -> usize {
        self.hit_regions
            .iter()
            .filter(|region| {
                matches!(
                    &region.target,
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

    pub(crate) fn clear_targets_in(&mut self, rect: Rect) {
        self.hit_regions
            .retain(|region| region.rect.intersection(rect).is_empty());
        self.scroll_regions
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
