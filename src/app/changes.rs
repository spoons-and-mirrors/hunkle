mod directory_loader;
mod preview_loader;
pub(crate) mod sqlite_browser;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use image::DynamicImage;
use ratatui::widgets::ListState;

use crate::{
    git::{Branch, Change, Commit, DiffSummary, InventoryRefresh, RepositoryData},
    repo_path::RepoPath,
    tree::{ExplorerRow, FileTree, PreparedFileTree, WorktreeRow, WorktreeSection, WorktreeTree},
    ui::preview::{DiffDocument, PreviewPresentation},
};

use directory_loader::DirectoryLoader;
use preview_loader::{LoadedPreview, PreviewLoader};
use sqlite_browser::{SqliteBrowser, SqliteDatabase, SqlitePageKey};
pub(crate) use sqlite_browser::{SqliteFocus, SqlitePage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftPane {
    Worktree,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangesHitTarget {
    WorktreeTab,
    FilesTab,
    AgentsTab,
    StageAll,
    WorktreeBackground(u64),
    WorktreeRow { generation: u64, index: usize },
    WorktreeStage { generation: u64, index: usize },
    HunkAction { generation: u64, index: usize },
    DiffFileHeader { generation: u64, index: usize },
    SqliteObjectsPane { generation: u64 },
    SqliteRowsPane { generation: u64 },
    SqliteObject { generation: u64, index: usize },
    SqliteRow { generation: u64, index: usize },
    SqlitePreviousPage { generation: u64 },
    SqliteNextPage { generation: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChangesEffect {
    PaneActivated,
    SidebarPaneActivated,
    AgentsPaneActivated,
    WorktreeDirectoryActivated,
    ToggleAllStaging,
    ToggleSelectedStage,
    StageHunk(usize),
    OpenDiffFileHeader(usize),
    WorktreeFileSelected { path: RepoPath, staged: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExplorerEntry {
    pub(super) path: RepoPath,
    pub(super) is_directory: bool,
}

pub struct ChangesState {
    pub(super) pane: LeftPane,
    pub(crate) preview: PreviewState,
    pub(crate) worktree_state: ListState,
    pub(crate) explorer_state: ListState,
    pub(crate) worktree_scroll: usize,
    pub(crate) worktree_scroll_to_selection: bool,
    pub(crate) explorer_scroll: usize,
    explorer_scroll_to_selection: bool,
    pub(crate) diff_scroll: usize,
    pub(crate) diff_wrap: bool,
    pub(crate) markdown_rendered: bool,
    markdown_alternate_scroll: Option<usize>,
    pub(crate) hunk_selection: Option<usize>,
    hunk_pin_pending: bool,
    pending_hunk_selection: Option<PendingHunkSelection>,
    pub(crate) collapsed_directories: HashSet<RepoPath>,
    pub(crate) expanded_explorer_directories: HashSet<RepoPath>,
    worktree_rows_cache: Vec<WorktreeRow>,
    worktree_rows_generation: u64,
    explorer_rows_cache: Vec<ExplorerRow>,
    file_tree: Option<FileTree>,
    directory_loader: DirectoryLoader,
    directory_generation: u64,
    loading_directories: HashSet<RepoPath>,
    failed_directories: HashSet<RepoPath>,
    pending_explorer_selection: Option<(RepoPath, usize)>,
    pending_preview_line: Option<(RepoPath, usize)>,
    worktree_tree: Option<WorktreeTree>,
    worktree_tree_fingerprint: Option<u64>,
    change_codes: HashMap<RepoPath, char>,
    selection_summary: Option<DiffSummary>,
    pub(crate) preview_presentation: PreviewPresentation,
    preview_loader: PreviewLoader,
}

pub(crate) struct PreviewState {
    generation: u64,
    origin: PreviewOrigin,
    payload: PreviewPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewOrigin {
    IdlePane(LeftPane),
    ExplorerFile {
        path: RepoPath,
    },
    ExplorerDirectory {
        path: RepoPath,
    },
    WorktreeChange {
        path: RepoPath,
        staged: bool,
        untracked: bool,
    },
    WorktreeSection(WorktreeSection),
    WorktreeDirectory {
        path: RepoPath,
        section: WorktreeSection,
    },
    Commit {
        oid: String,
    },
    BranchComparison(BranchComparison),
    Issue(IssuePreview),
}

pub(crate) enum PreviewPayload {
    Empty,
    Loading,
    Message(String),
    Source(String),
    Diff(DiffDocument),
    Image(Arc<DynamicImage>),
    Database(SqliteBrowser),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchComparison {
    pub(crate) current: String,
    pub(crate) target: String,
    target_revision: String,
    current_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssuePreview {
    pub(crate) number: u64,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) body: Arc<str>,
    pub(crate) pull_request: Option<PullRequestPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestPreview {
    pub(crate) base_ref_name: String,
    pub(crate) base_ref_oid: String,
    pub(crate) head_ref_name: String,
    pub(crate) head_ref_oid: String,
    pub(crate) changed_files: Option<u64>,
    pub(crate) additions: Option<u64>,
    pub(crate) deletions: Option<u64>,
}

struct PendingHunkSelection {
    path: RepoPath,
    index: usize,
}

impl PreviewState {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn origin(&self) -> &PreviewOrigin {
        &self.origin
    }

    pub(crate) fn pane(&self) -> LeftPane {
        match self.origin {
            PreviewOrigin::ExplorerFile { .. }
            | PreviewOrigin::ExplorerDirectory { .. }
            | PreviewOrigin::Issue(_) => LeftPane::Files,
            PreviewOrigin::IdlePane(pane) => pane,
            _ => LeftPane::Worktree,
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match &self.payload {
            PreviewPayload::Empty => Some(""),
            PreviewPayload::Loading => Some("Loading preview…"),
            PreviewPayload::Message(message) | PreviewPayload::Error(message) => Some(message),
            PreviewPayload::Source(source) => Some(source),
            PreviewPayload::Diff(document) => Some(document.as_str()),
            PreviewPayload::Image(_) | PreviewPayload::Database(_) => None,
        }
    }

    pub(crate) fn content(&self) -> Option<crate::ui::preview::PreviewContent<'_>> {
        match &self.payload {
            PreviewPayload::Diff(document) => {
                Some(crate::ui::preview::PreviewContent::Diff(document))
            }
            PreviewPayload::Empty => Some(crate::ui::preview::PreviewContent::Source("")),
            PreviewPayload::Loading => Some(crate::ui::preview::PreviewContent::Source(
                "Loading preview…",
            )),
            PreviewPayload::Message(message)
            | PreviewPayload::Source(message)
            | PreviewPayload::Error(message) => {
                Some(crate::ui::preview::PreviewContent::Source(message))
            }
            PreviewPayload::Image(_) | PreviewPayload::Database(_) => None,
        }
    }

    pub(crate) fn document(&self) -> Option<&DiffDocument> {
        let PreviewPayload::Diff(document) = &self.payload else {
            return None;
        };
        Some(document)
    }

    pub(crate) fn image(&self) -> Option<&Arc<DynamicImage>> {
        let PreviewPayload::Image(image) = &self.payload else {
            return None;
        };
        Some(image)
    }

    pub(crate) fn database(&self) -> Option<&SqliteBrowser> {
        let PreviewPayload::Database(browser) = &self.payload else {
            return None;
        };
        Some(browser)
    }

    pub(crate) fn database_mut(&mut self) -> Option<&mut SqliteBrowser> {
        let PreviewPayload::Database(browser) = &mut self.payload else {
            return None;
        };
        Some(browser)
    }

    pub(crate) fn issue(&self) -> Option<&IssuePreview> {
        let PreviewOrigin::Issue(issue) = &self.origin else {
            return None;
        };
        Some(issue)
    }

    pub(crate) fn branch_comparison(&self) -> Option<&BranchComparison> {
        let PreviewOrigin::BranchComparison(comparison) = &self.origin else {
            return None;
        };
        Some(comparison)
    }

    pub(crate) fn show_file_headers(&self) -> bool {
        matches!(
            self.origin,
            PreviewOrigin::Commit { .. }
                | PreviewOrigin::WorktreeSection(_)
                | PreviewOrigin::WorktreeDirectory { .. }
                | PreviewOrigin::BranchComparison(_)
                | PreviewOrigin::Issue(IssuePreview {
                    pull_request: Some(_),
                    ..
                })
        )
    }

    pub(crate) fn markdown_available(&self) -> bool {
        matches!(
            self.origin,
            PreviewOrigin::Issue(IssuePreview {
                pull_request: None,
                ..
            })
        ) || matches!(&self.origin, PreviewOrigin::ExplorerFile { path } if crate::app::is_markdown_path(path))
            && matches!(self.payload, PreviewPayload::Source(_))
    }

    pub(crate) fn wrappable(&self) -> bool {
        !matches!(
            self.payload,
            PreviewPayload::Image(_) | PreviewPayload::Database(_)
        )
    }

    pub(crate) fn editable(&self) -> bool {
        matches!(
            (&self.origin, &self.payload),
            (
                PreviewOrigin::ExplorerFile { .. },
                PreviewPayload::Source(_)
            ) | (
                PreviewOrigin::WorktreeChange { .. },
                PreviewPayload::Diff(_)
            ) | (
                PreviewOrigin::WorktreeSection(_)
                    | PreviewOrigin::WorktreeDirectory { .. }
                    | PreviewOrigin::BranchComparison(_),
                PreviewPayload::Diff(_),
            )
        )
    }

    pub(crate) fn hunk_actions(&self) -> bool {
        matches!(
            (&self.origin, &self.payload),
            (
                PreviewOrigin::WorktreeChange {
                    staged: false,
                    untracked: false,
                    ..
                },
                PreviewPayload::Diff(_)
            )
        )
    }
}

pub(super) struct ChangesSelection {
    change: Option<(RepoPath, bool)>,
    section: Option<WorktreeSection>,
    directory: Option<(RepoPath, WorktreeSection)>,
    explorer_file: Option<RepoPath>,
    explorer_directory: Option<RepoPath>,
}

impl ChangesState {
    pub(super) fn initial_pane(repo: Option<&RepositoryData>) -> LeftPane {
        if repo
            .is_some_and(|repo| !repo.is_local() && repo.details_ready && !repo.changes.is_empty())
        {
            LeftPane::Worktree
        } else {
            LeftPane::Files
        }
    }

    pub(super) fn new(repo: Option<&RepositoryData>) -> Self {
        let file_tree = repo.map(|repo| FileTree::from_root(&repo.root));
        let initial_pane = Self::initial_pane(repo);
        let mut state = Self {
            pane: initial_pane,
            preview: PreviewState {
                generation: 0,
                origin: PreviewOrigin::IdlePane(initial_pane),
                payload: PreviewPayload::Empty,
            },
            worktree_state: ListState::default(),
            explorer_state: ListState::default(),
            worktree_scroll: 0,
            worktree_scroll_to_selection: true,
            explorer_scroll: 0,
            explorer_scroll_to_selection: false,
            diff_scroll: 0,
            diff_wrap: true,
            markdown_rendered: false,
            markdown_alternate_scroll: None,
            hunk_selection: None,
            hunk_pin_pending: false,
            pending_hunk_selection: None,
            collapsed_directories: HashSet::new(),
            expanded_explorer_directories: HashSet::new(),
            worktree_rows_cache: Vec::new(),
            worktree_rows_generation: 0,
            explorer_rows_cache: Vec::new(),
            file_tree,
            directory_loader: DirectoryLoader::new(),
            directory_generation: 0,
            loading_directories: HashSet::new(),
            failed_directories: HashSet::new(),
            pending_explorer_selection: None,
            pending_preview_line: None,
            worktree_tree: repo.map(|repo| WorktreeTree::new(&repo.changes)),
            worktree_tree_fingerprint: repo.map(|repo| repo.changes_fingerprint),
            change_codes: repo.map_or_else(HashMap::new, |repo| change_codes(&repo.changes)),
            selection_summary: None,
            preview_presentation: PreviewPresentation::default(),
            preview_loader: PreviewLoader::new(),
        };
        state.rebuild_worktree_rows(repo);
        state.rebuild_explorer_rows(repo);
        if let Some(repo) = repo
            && state
                .file_tree
                .as_ref()
                .is_none_or(|tree| !tree.has_directory(&RepoPath::default()))
        {
            state.request_explorer_directory(repo, RepoPath::default());
        }
        state.select_initial_rows(repo);
        state.refresh_diff(repo);
        state
    }

    pub(super) fn reset_repository(
        &mut self,
        repo: Option<&RepositoryData>,
        prepared_file_tree: Option<PreparedFileTree>,
    ) {
        self.pane = Self::initial_pane(repo);
        self.preview.origin = PreviewOrigin::IdlePane(self.pane);
        self.worktree_state = ListState::default();
        self.explorer_state = ListState::default();
        self.worktree_scroll = 0;
        self.worktree_scroll_to_selection = true;
        self.explorer_scroll = 0;
        self.explorer_scroll_to_selection = false;
        self.set_preview_payload(PreviewPayload::Empty);
        self.diff_scroll = 0;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.collapsed_directories.clear();
        self.expanded_explorer_directories.clear();
        self.directory_generation = self.directory_generation.wrapping_add(1);
        self.loading_directories.clear();
        self.failed_directories.clear();
        self.pending_explorer_selection = None;
        self.pending_preview_line = None;
        if let Some(prepared) = prepared_file_tree {
            let tree = prepared.into_tree();
            if let Some(previous) = self.file_tree.replace(tree) {
                crate::diagnostics::drop_in_background("file-tree", previous);
            }
        } else {
            self.file_tree = repo.map(|repo| FileTree::from_root(&repo.root));
        }
        self.rebuild_worktree_rows(repo);
        self.rebuild_explorer_rows(repo);
        if let Some(repo) = repo
            && self
                .file_tree
                .as_ref()
                .is_none_or(|tree| !tree.has_directory(&RepoPath::default()))
        {
            self.request_explorer_directory(repo, RepoPath::default());
        }
        self.select_initial_rows(repo);
        self.refresh_diff(repo);
    }

    pub(super) fn capture_selection(&self, repo: &RepositoryData) -> ChangesSelection {
        ChangesSelection {
            change: self
                .selected_change_index(repo)
                .and_then(|index| repo.changes.get(index))
                .map(|change| (change.path.clone(), change.staged)),
            section: self.selected_diff_section(),
            directory: self.selected_directory_path(repo).and_then(|path| {
                let section = self.selected_worktree_section()?;
                Some((path, section))
            }),
            explorer_file: self.selected_explorer_file_path(repo).cloned(),
            explorer_directory: self.selected_explorer_directory_path(),
        }
    }

    pub(super) fn restore_selection(
        &mut self,
        repo: &RepositoryData,
        selection: ChangesSelection,
        inventory_refresh: InventoryRefresh,
    ) {
        let branch_comparison = self.branch_comparison().cloned();
        self.rebuild_worktree_rows(Some(repo));
        self.refresh_explorer_directories(repo, inventory_refresh);

        let change_index = selection.change.and_then(|(path, staged)| {
            repo.changes
                .iter()
                .position(|change| change.path == path && change.staged == staged)
                .or_else(|| repo.changes.iter().position(|change| change.path == path))
        });
        let change_row = change_index
            .and_then(|index| self.row_for_change(repo, index))
            .or_else(|| {
                let section = selection.section?;
                self.worktree_rows(repo)
                    .iter()
                    .position(|row| row.section == Some(section) && row.section_stats.is_some())
            })
            .or_else(|| {
                let (directory, section) = selection.directory.as_ref()?;
                self.worktree_rows(repo)
                    .iter()
                    .enumerate()
                    .position(|(index, row)| {
                        row.directory_path.as_ref() == Some(directory)
                            && self.worktree_section(index) == Some(*section)
                    })
            })
            .or_else(|| self.first_change_row(repo));
        self.worktree_state.select(change_row);
        self.worktree_scroll_to_selection = true;

        let explorer_row = selection
            .explorer_file
            .and_then(|path| self.row_for_explorer_file(&path))
            .or_else(|| {
                let directory = selection.explorer_directory.as_ref()?;
                self.explorer_rows()
                    .iter()
                    .position(|row| row.directory_path.as_ref() == Some(directory))
            })
            .or_else(|| self.initial_explorer_row());
        self.explorer_state.select(explorer_row);
        self.refresh_diff(Some(repo));
        if let Some(comparison) = branch_comparison
            && repo.branch == comparison.current
            && (comparison.target_revision == "HEAD~"
                || repo
                    .branches
                    .iter()
                    .any(|branch| branch.revision() == comparison.target_revision))
        {
            let current_revision = repo
                .branches
                .iter()
                .find(|branch| branch.current)
                .map(Branch::revision)
                .or_else(|| repo.history.first().map(|commit| commit.oid.clone()))
                .unwrap_or(comparison.current_revision);
            self.preview_branch_diff(
                &repo.root,
                comparison.current,
                comparison.target,
                current_revision,
                comparison.target_revision,
            );
        }
    }

    pub(crate) fn worktree_rows(&self, _repo: &RepositoryData) -> &[WorktreeRow] {
        &self.worktree_rows_cache
    }

    pub(crate) fn explorer_rows(&self) -> &[ExplorerRow] {
        &self.explorer_rows_cache
    }

    #[cfg(test)]
    pub(super) fn worktree_rows_generation_for_test(&self) -> u64 {
        self.worktree_rows_generation
    }

    #[cfg(test)]
    pub(super) fn preview_request_generation_for_test(&self) -> u64 {
        self.preview_loader.generation_for_test()
    }

    pub(crate) fn selected_explorer_file_path(&self, _repo: &RepositoryData) -> Option<&RepoPath> {
        let selected = self.explorer_state.selected()?;
        self.explorer_rows().get(selected)?.file_path.as_ref()
    }

    pub(super) fn selected_change_index(&self, repo: &RepositoryData) -> Option<usize> {
        let selected = self.worktree_state.selected()?;
        self.worktree_rows(repo).get(selected)?.change_index
    }

    pub(super) fn selected_directory_path(&self, repo: &RepositoryData) -> Option<RepoPath> {
        let selected = self.worktree_state.selected()?;
        self.worktree_rows(repo)
            .get(selected)?
            .directory_path
            .clone()
    }

    fn selected_worktree_section(&self) -> Option<WorktreeSection> {
        self.worktree_state
            .selected()
            .and_then(|index| self.worktree_section(index))
    }

    pub(crate) fn selected_diff_section(&self) -> Option<WorktreeSection> {
        let selected = self.worktree_state.selected()?;
        let row = self.worktree_rows_cache.get(selected)?;
        row.section.filter(|_| row.section_stats.is_some())
    }

    fn worktree_section(&self, index: usize) -> Option<WorktreeSection> {
        self.worktree_rows_cache[..=index]
            .iter()
            .rev()
            .find_map(|row| row.section)
    }

    pub(super) fn selected_explorer_directory_path(&self) -> Option<RepoPath> {
        let selected = self.explorer_state.selected()?;
        self.explorer_rows().get(selected)?.directory_path.clone()
    }

    pub(super) fn selected_explorer_entry(&self, repo: &RepositoryData) -> Option<ExplorerEntry> {
        let selected = self.explorer_state.selected()?;
        self.explorer_entry(repo, selected)
    }

    pub(super) fn explorer_entry(
        &self,
        _repo: &RepositoryData,
        index: usize,
    ) -> Option<ExplorerEntry> {
        let row = self.explorer_rows().get(index)?;
        if let Some(path) = &row.file_path {
            return Some(ExplorerEntry {
                path: path.clone(),
                is_directory: false,
            });
        }
        Some(ExplorerEntry {
            path: row.directory_path.clone()?,
            is_directory: true,
        })
    }

    pub(super) fn select_explorer_path(
        &mut self,
        repo: &RepositoryData,
        path: &RepoPath,
        viewport: usize,
    ) -> bool {
        self.clear_issue_preview();
        if self
            .pending_preview_line
            .as_ref()
            .is_some_and(|(pending, _)| pending != path)
        {
            self.pending_preview_line = None;
        }
        expand_ancestors(&mut self.expanded_explorer_directories, path);
        self.request_explorer_ancestors(repo, path);
        self.rebuild_explorer_rows(Some(repo));
        let row = self.explorer_rows().iter().position(|row| {
            row.directory_path.as_ref() == Some(path) || row.file_path.as_ref() == Some(path)
        });
        let Some(row) = row else {
            let parent = path.parent().unwrap_or_default();
            if self
                .file_tree
                .as_ref()
                .is_some_and(|tree| tree.has_directory(&parent))
                && !self.loading_directories.contains(&parent)
            {
                self.pending_explorer_selection = None;
                return false;
            }
            self.pending_explorer_selection = Some((path.clone(), viewport));
            if repo.files.iter().any(|candidate| candidate == path) {
                self.preview.origin = PreviewOrigin::ExplorerFile { path: path.clone() };
                self.preview_loader.invalidate();
                self.set_preview_payload(PreviewPayload::Loading);
            }
            return true;
        };
        self.pending_explorer_selection = None;
        self.explorer_state.select(Some(row));
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Files);
        ensure_selection_visible(&mut self.explorer_scroll, Some(row), viewport);
        self.explorer_scroll = self
            .explorer_scroll
            .min(self.explorer_rows_cache.len().saturating_sub(viewport));
        self.explorer_scroll_to_selection = true;
        self.refresh_diff(Some(repo));
        true
    }

    pub(crate) fn reveal_explorer_selection(&mut self, viewport: usize) {
        if !self.explorer_scroll_to_selection {
            return;
        }
        ensure_selection_visible(
            &mut self.explorer_scroll,
            self.explorer_state.selected(),
            viewport,
        );
        self.explorer_scroll = self
            .explorer_scroll
            .min(self.explorer_rows_cache.len().saturating_sub(viewport));
        self.explorer_scroll_to_selection = false;
    }

    pub(super) fn set_pane(&mut self, pane: LeftPane, repo: Option<&RepositoryData>) -> bool {
        let changed = self.set_pane_preserving_preview(pane);
        if !changed
            && self.preview.pane() == pane
            && !matches!(self.preview.origin, PreviewOrigin::Issue(_))
        {
            return false;
        }
        self.preview.origin = PreviewOrigin::IdlePane(pane);
        self.refresh_diff(repo);
        true
    }

    pub(super) fn set_pane_preserving_preview(&mut self, pane: LeftPane) -> bool {
        if self.pane == pane {
            return false;
        }
        self.pane = pane;
        if pane == LeftPane::Files && self.explorer_state.selected().is_none() {
            self.explorer_state.select(self.initial_explorer_row());
        }
        true
    }

    pub(super) fn select_worktree_row(&mut self, repo: &RepositoryData, index: usize) -> bool {
        let Some(row) = self.worktree_rows(repo).get(index) else {
            return false;
        };
        if row.section.is_some() && row.section_stats.is_none() {
            return false;
        }
        self.clear_issue_preview();
        self.worktree_state.select(Some(index));
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Worktree);
        self.refresh_diff(Some(repo));
        true
    }

    pub(super) fn activate_target(
        &mut self,
        target: ChangesHitTarget,
        repo: &RepositoryData,
    ) -> Option<ChangesEffect> {
        match target {
            ChangesHitTarget::WorktreeTab => {
                self.set_pane_preserving_preview(LeftPane::Worktree);
                Some(ChangesEffect::SidebarPaneActivated)
            }
            ChangesHitTarget::FilesTab => {
                self.set_pane_preserving_preview(LeftPane::Files);
                Some(ChangesEffect::SidebarPaneActivated)
            }
            ChangesHitTarget::AgentsTab => Some(ChangesEffect::AgentsPaneActivated),
            ChangesHitTarget::StageAll => {
                if self.pane != LeftPane::Worktree {
                    return None;
                }
                Some(ChangesEffect::ToggleAllStaging)
            }
            ChangesHitTarget::WorktreeBackground(_) => None,
            ChangesHitTarget::WorktreeRow { generation, index } => {
                if !self.is_current_worktree_target(generation) {
                    return None;
                }
                if !self.select_worktree_row(repo, index) {
                    self.refresh_diff(Some(repo));
                    return None;
                }
                if self.selected_diff_section().is_some() {
                    return Some(ChangesEffect::PaneActivated);
                }
                if self.selected_directory_path(repo).is_some() {
                    self.toggle_selected_directory(Some(repo));
                    return Some(ChangesEffect::WorktreeDirectoryActivated);
                }
                self.selected_change_index(repo)
                    .and_then(|index| repo.changes.get(index))
                    .map(|change| ChangesEffect::WorktreeFileSelected {
                        path: change.path.clone(),
                        staged: change.staged,
                    })
            }
            ChangesHitTarget::WorktreeStage { .. } => self.stage_target(target, repo),
            ChangesHitTarget::HunkAction { generation, index } => {
                (generation == self.preview.generation()).then_some(ChangesEffect::StageHunk(index))
            }
            ChangesHitTarget::DiffFileHeader { generation, index } => (generation
                == self.preview.generation())
            .then_some(ChangesEffect::OpenDiffFileHeader(index)),
            ChangesHitTarget::SqliteObjectsPane { generation } => {
                let browser = self.current_sqlite_target(generation)?;
                browser.active = true;
                browser.focus = SqliteFocus::Objects;
                None
            }
            ChangesHitTarget::SqliteRowsPane { generation } => {
                let browser = self.current_sqlite_target(generation)?;
                browser.active = true;
                browser.focus = SqliteFocus::Rows;
                None
            }
            ChangesHitTarget::SqliteObject { generation, index } => {
                let browser = self.current_sqlite_target(generation)?;
                browser.active = true;
                let key = browser.select_object(index, 0);
                if let Some(key) = key {
                    self.request_sqlite_page(repo, key);
                }
                None
            }
            ChangesHitTarget::SqliteRow { generation, index } => {
                let browser = self.current_sqlite_target(generation)?;
                browser.active = true;
                browser.select_row(index, 0);
                None
            }
            ChangesHitTarget::SqlitePreviousPage { generation } => {
                if self.current_sqlite_target(generation).is_some() {
                    self.page_sqlite(repo, -1);
                }
                None
            }
            ChangesHitTarget::SqliteNextPage { generation } => {
                if self.current_sqlite_target(generation).is_some() {
                    self.page_sqlite(repo, 1);
                }
                None
            }
        }
    }

    pub(super) fn stage_target(
        &mut self,
        target: ChangesHitTarget,
        repo: &RepositoryData,
    ) -> Option<ChangesEffect> {
        let (generation, index) = match target {
            ChangesHitTarget::WorktreeRow { generation, index }
            | ChangesHitTarget::WorktreeStage { generation, index } => (generation, index),
            _ => return None,
        };
        if !self.is_current_worktree_target(generation) {
            return None;
        }
        self.select_worktree_row(repo, index)
            .then(|| self.selected_change_index(repo))
            .flatten()
            .map(|_| ChangesEffect::ToggleSelectedStage)
    }

    pub(crate) fn worktree_background_target(&self) -> ChangesHitTarget {
        ChangesHitTarget::WorktreeBackground(self.worktree_rows_generation)
    }

    pub(crate) fn worktree_row_target(&self, index: usize) -> ChangesHitTarget {
        ChangesHitTarget::WorktreeRow {
            generation: self.worktree_rows_generation,
            index,
        }
    }

    pub(crate) fn worktree_stage_target(&self, index: usize) -> ChangesHitTarget {
        ChangesHitTarget::WorktreeStage {
            generation: self.worktree_rows_generation,
            index,
        }
    }

    pub(crate) fn hunk_action_target(&self, index: usize) -> ChangesHitTarget {
        ChangesHitTarget::HunkAction {
            generation: self.preview.generation(),
            index,
        }
    }

    pub(crate) fn sqlite_objects_target(&self) -> Option<ChangesHitTarget> {
        Some(ChangesHitTarget::SqliteObjectsPane {
            generation: self.preview.database()?.generation,
        })
    }

    pub(crate) fn sqlite_rows_target(&self) -> Option<ChangesHitTarget> {
        Some(ChangesHitTarget::SqliteRowsPane {
            generation: self.preview.database()?.generation,
        })
    }

    pub(crate) fn sqlite_object_target(&self, index: usize) -> Option<ChangesHitTarget> {
        Some(ChangesHitTarget::SqliteObject {
            generation: self.preview.database()?.generation,
            index,
        })
    }

    pub(crate) fn sqlite_row_target(&self, index: usize) -> Option<ChangesHitTarget> {
        Some(ChangesHitTarget::SqliteRow {
            generation: self.preview.database()?.generation,
            index,
        })
    }

    pub(crate) fn sqlite_page_target(&self, next: bool) -> Option<ChangesHitTarget> {
        let generation = self.preview.database()?.generation;
        Some(if next {
            ChangesHitTarget::SqliteNextPage { generation }
        } else {
            ChangesHitTarget::SqlitePreviousPage { generation }
        })
    }

    pub(super) fn sqlite_active(&self) -> bool {
        self.preview
            .database()
            .is_some_and(|browser| browser.active)
    }

    pub(super) fn activate_sqlite(&mut self) -> bool {
        let Some(browser) = self.preview.database_mut() else {
            return false;
        };
        browser.active = true;
        true
    }

    pub(super) fn deactivate_sqlite(&mut self) {
        if let Some(browser) = self.preview.database_mut() {
            browser.active = false;
        }
    }

    pub(super) fn toggle_sqlite_focus(&mut self) {
        let Some(browser) = self.preview.database_mut() else {
            return;
        };
        browser.focus = match browser.focus {
            SqliteFocus::Objects => SqliteFocus::Rows,
            SqliteFocus::Rows => SqliteFocus::Objects,
        };
    }

    pub(super) fn focus_sqlite_rows(&mut self) {
        if let Some(browser) = self.preview.database_mut() {
            browser.focus = SqliteFocus::Rows;
        }
    }

    pub(super) fn focus_sqlite_objects(&mut self) {
        if let Some(browser) = self.preview.database_mut() {
            browser.focus = SqliteFocus::Objects;
        }
    }

    pub(super) fn move_sqlite_selection(
        &mut self,
        repo: &RepositoryData,
        delta: isize,
        object_viewport: usize,
        row_viewport: usize,
    ) {
        let Some(browser) = self.preview.database_mut() else {
            return;
        };
        let key = match browser.focus {
            SqliteFocus::Objects => browser.move_object(delta, object_viewport),
            SqliteFocus::Rows => {
                browser.move_row(delta, row_viewport);
                None
            }
        };
        if let Some(key) = key {
            self.request_sqlite_page(repo, key);
        }
    }

    pub(super) fn select_sqlite_boundary(
        &mut self,
        repo: &RepositoryData,
        last: bool,
        object_viewport: usize,
        row_viewport: usize,
    ) {
        let Some(browser) = self.preview.database_mut() else {
            return;
        };
        let key = match browser.focus {
            SqliteFocus::Objects => browser.select_object_boundary(last, object_viewport),
            SqliteFocus::Rows => {
                let row = if last {
                    browser
                        .page
                        .as_ref()
                        .and_then(|page| page.rows.len().checked_sub(1))
                } else {
                    Some(0)
                };
                if let Some(row) = row {
                    browser.select_row(row, row_viewport);
                }
                None
            }
        };
        if let Some(key) = key {
            self.request_sqlite_page(repo, key);
        }
    }

    pub(super) fn page_sqlite(&mut self, repo: &RepositoryData, delta: isize) {
        let key = self
            .preview
            .database_mut()
            .and_then(|browser| browser.page_by(delta));
        if let Some(key) = key {
            self.request_sqlite_page(repo, key);
        }
    }

    pub(super) fn shift_sqlite_columns(&mut self, delta: isize) {
        if let Some(browser) = self.preview.database_mut() {
            browser.shift_columns(delta);
        }
    }

    pub(super) fn scroll_sqlite_objects(&mut self, viewport: usize, delta: isize) {
        let Some(browser) = self.preview.database_mut() else {
            return;
        };
        scroll_viewport(
            &mut browser.object_scroll,
            browser.objects.len(),
            viewport,
            delta,
        );
    }

    pub(super) fn scroll_sqlite_rows(&mut self, viewport: usize, delta: isize) {
        let Some(browser) = self.preview.database_mut() else {
            return;
        };
        let len = browser.page.as_ref().map_or(0, |page| page.rows.len());
        scroll_viewport(&mut browser.row_scroll, len, viewport, delta);
    }

    fn current_sqlite_target(&mut self, generation: u64) -> Option<&mut SqliteBrowser> {
        let browser = self.preview.database_mut()?;
        (browser.generation == generation).then_some(browser)
    }

    fn request_sqlite_page(&mut self, repo: &RepositoryData, key: SqlitePageKey) {
        let Some(path) = self.preview.database().map(|browser| browser.path.clone()) else {
            return;
        };
        self.preview_loader
            .request_sqlite_page(&repo.root, path, key);
    }

    fn is_current_worktree_target(&self, generation: u64) -> bool {
        self.pane == LeftPane::Worktree && generation == self.worktree_rows_generation
    }

    pub(super) fn select_explorer_row(&mut self, repo: &RepositoryData, index: usize) -> bool {
        if index >= self.explorer_rows().len() {
            return false;
        }
        self.clear_issue_preview();
        self.pending_explorer_selection = None;
        self.pending_preview_line = None;
        self.explorer_state.select(Some(index));
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Files);
        self.refresh_diff(Some(repo));
        true
    }

    pub(super) fn move_selection(
        &mut self,
        repo: Option<&RepositoryData>,
        delta: isize,
        worktree_viewport: usize,
        explorer_viewport: usize,
    ) {
        let Some(repo) = repo else {
            return;
        };
        let previous = self.preview_selection();
        if self.pane == LeftPane::Files {
            self.pending_explorer_selection = None;
            move_list(
                &mut self.explorer_state,
                self.explorer_rows_cache.len(),
                delta,
            );
            ensure_selection_visible(
                &mut self.explorer_scroll,
                self.explorer_state.selected(),
                explorer_viewport,
            );
        } else {
            move_worktree_selection(&mut self.worktree_state, &self.worktree_rows_cache, delta);
            ensure_selection_visible(
                &mut self.worktree_scroll,
                self.worktree_state.selected(),
                worktree_viewport,
            );
        }
        if self.preview_selection() != previous {
            self.clear_issue_preview();
            self.pending_preview_line = None;
            self.preview.origin = PreviewOrigin::IdlePane(self.pane);
            self.refresh_diff(Some(repo));
        }
    }

    pub(super) fn select_first(
        &mut self,
        repo: Option<&RepositoryData>,
        worktree_viewport: usize,
        explorer_viewport: usize,
    ) {
        let Some(repo) = repo else {
            return;
        };
        let previous = self.preview_selection();
        if self.pane == LeftPane::Files {
            self.pending_explorer_selection = None;
            self.explorer_state
                .select((!self.explorer_rows().is_empty()).then_some(0));
            ensure_selection_visible(
                &mut self.explorer_scroll,
                self.explorer_state.selected(),
                explorer_viewport,
            );
        } else {
            self.worktree_state.select(self.first_change_row(repo));
            ensure_selection_visible(
                &mut self.worktree_scroll,
                self.worktree_state.selected(),
                worktree_viewport,
            );
        }
        if self.preview_selection() != previous {
            self.clear_issue_preview();
            self.preview.origin = PreviewOrigin::IdlePane(self.pane);
            self.refresh_diff(Some(repo));
        }
    }

    pub(super) fn select_last(
        &mut self,
        repo: Option<&RepositoryData>,
        worktree_viewport: usize,
        explorer_viewport: usize,
    ) {
        let Some(repo) = repo else {
            return;
        };
        let previous = self.preview_selection();
        if self.pane == LeftPane::Files {
            self.pending_explorer_selection = None;
            self.explorer_state
                .select(self.explorer_rows().len().checked_sub(1));
            ensure_selection_visible(
                &mut self.explorer_scroll,
                self.explorer_state.selected(),
                explorer_viewport,
            );
        } else {
            self.worktree_state.select(self.last_change_row(repo));
            ensure_selection_visible(
                &mut self.worktree_scroll,
                self.worktree_state.selected(),
                worktree_viewport,
            );
        }
        if self.preview_selection() != previous {
            self.clear_issue_preview();
            self.preview.origin = PreviewOrigin::IdlePane(self.pane);
            self.refresh_diff(Some(repo));
        }
    }

    pub(super) fn scroll_worktree(
        &mut self,
        repo: Option<&RepositoryData>,
        viewport: usize,
        delta: isize,
    ) {
        let len = repo.map_or(0, |repo| self.worktree_rows(repo).len());
        self.worktree_scroll_to_selection = false;
        scroll_viewport(&mut self.worktree_scroll, len, viewport, delta);
    }

    pub(super) fn scroll_explorer(&mut self, viewport: usize, delta: isize) {
        self.explorer_scroll_to_selection = false;
        scroll_viewport(
            &mut self.explorer_scroll,
            self.explorer_rows_cache.len(),
            viewport,
            delta,
        );
    }

    pub(super) fn scroll_diff_by(&mut self, maximum: usize, delta: isize) {
        self.diff_scroll = if delta > 0 {
            self.diff_scroll.saturating_add(delta as usize).min(maximum)
        } else {
            self.diff_scroll.saturating_sub(delta.unsigned_abs())
        };
    }

    pub(super) fn set_diff_scroll_from_track(
        &mut self,
        row: u16,
        track_y: u16,
        track_height: u16,
        thumb_height: u16,
        drag_offset: u16,
        maximum: usize,
    ) {
        let travel = track_height.saturating_sub(thumb_height);
        if travel == 0 || maximum == 0 {
            self.diff_scroll = 0;
            return;
        }
        let position = row
            .saturating_sub(track_y)
            .saturating_sub(drag_offset)
            .min(travel);
        self.diff_scroll =
            (usize::from(position) * maximum + usize::from(travel) / 2) / usize::from(travel);
    }

    pub(super) fn toggle_wrap(&mut self) -> bool {
        self.diff_wrap = !self.diff_wrap;
        self.diff_wrap
    }

    pub(super) fn toggle_markdown_rendered(&mut self) {
        let outgoing_scroll = self.diff_scroll;
        self.diff_scroll = self
            .markdown_alternate_scroll
            .replace(outgoing_scroll)
            .unwrap_or(outgoing_scroll);
        self.markdown_rendered = !self.markdown_rendered;
        self.hunk_selection = None;
        self.preview_presentation.clear();
    }

    pub(super) fn preview_commit(&mut self, repo: &RepositoryData, commit: &Commit) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::Commit {
            oid: commit.oid.clone(),
        };
        self.diff_scroll = 0;
        self.markdown_alternate_scroll = None;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.set_preview_payload(PreviewPayload::Loading);
        self.preview_loader
            .request_commit(&repo.root, commit.oid.clone());
    }

    pub(crate) fn branch_comparison(&self) -> Option<&BranchComparison> {
        self.preview.branch_comparison()
    }

    pub(super) fn clear_branch_comparison(&mut self) {
        if matches!(self.preview.origin, PreviewOrigin::BranchComparison(_)) {
            self.preview.origin = PreviewOrigin::IdlePane(self.pane);
            self.preview_loader.invalidate();
            self.set_preview_payload(PreviewPayload::Empty);
            self.diff_scroll = 0;
            self.hunk_selection = None;
        }
    }

    pub(super) fn preview_branch_diff(
        &mut self,
        root: &Path,
        current: String,
        target: String,
        current_revision: String,
        target_revision: String,
    ) {
        self.clear_issue_preview();
        self.diff_scroll = 0;
        self.markdown_alternate_scroll = None;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.pending_explorer_selection = None;
        self.preview.origin = PreviewOrigin::BranchComparison(BranchComparison {
            current,
            target: target.clone(),
            target_revision: target_revision.clone(),
            current_revision: current_revision.clone(),
        });
        self.set_preview_payload(PreviewPayload::Loading);
        self.preview_loader
            .request_branch_diff(root, target_revision, current_revision);
    }

    pub(super) fn enter_hunk_selection(&mut self, repo: &RepositoryData) -> bool {
        if !self.preview.hunk_actions() {
            return false;
        }
        let Some(change) = self
            .selected_change_index(repo)
            .and_then(|index| repo.changes.get(index))
        else {
            return false;
        };
        if change.staged
            || self
                .preview
                .document()
                .is_none_or(|diff| diff.hunk_count() == 0)
        {
            return false;
        }
        self.hunk_selection = Some(0);
        self.hunk_pin_pending = true;
        self.diff_scroll = 0;
        true
    }

    pub(super) fn move_hunk_selection(&mut self, delta: isize) {
        let count = self.preview.document().map_or(0, DiffDocument::hunk_count);
        let Some(selected) = self.hunk_selection else {
            return;
        };
        let next = if delta > 0 {
            selected.saturating_add(1).min(count.saturating_sub(1))
        } else {
            selected.saturating_sub(delta.unsigned_abs())
        };
        if next != selected {
            self.hunk_selection = Some(next);
            self.hunk_pin_pending = true;
        }
    }

    pub(super) fn select_hunk(&mut self, index: usize) -> bool {
        if self.hunk_selection.is_some()
            && index < self.preview.document().map_or(0, DiffDocument::hunk_count)
            && self.hunk_selection != Some(index)
        {
            self.hunk_selection = Some(index);
            self.hunk_pin_pending = true;
            return true;
        }
        false
    }

    pub(super) fn leave_hunk_selection(&mut self) {
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
    }

    pub(super) fn preserve_hunk_selection_after_stage(&mut self, path: RepoPath, index: usize) {
        self.hunk_selection = Some(index);
        self.pending_hunk_selection = Some(PendingHunkSelection { path, index });
    }

    pub(super) fn cancel_pending_hunk_stage(&mut self) {
        self.pending_hunk_selection = None;
    }

    pub(crate) fn take_hunk_pin_request(&mut self) -> bool {
        std::mem::take(&mut self.hunk_pin_pending)
    }

    pub(super) fn toggle_selected_explorer_directory(&mut self, repo: Option<&RepositoryData>) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Files);
        self.pending_explorer_selection = None;
        let Some(path) = self.selected_explorer_directory_path() else {
            return;
        };
        if !self.expanded_explorer_directories.remove(&path) {
            self.expanded_explorer_directories.insert(path.clone());
            if let Some(repo) = repo {
                self.request_explorer_directory(repo, path.clone());
            }
        }
        self.rebuild_explorer_rows(repo);
        self.select_explorer_directory(&path);
        self.refresh_diff(repo);
    }

    pub(super) fn expand_or_descend_explorer(&mut self, repo: Option<&RepositoryData>) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Files);
        self.pending_explorer_selection = None;
        let Some(index) = self.explorer_state.selected() else {
            return;
        };
        let Some(row) = self.explorer_rows_cache.get(index) else {
            return;
        };
        let Some(path) = row.directory_path.clone() else {
            return;
        };
        let expanded = row.directory_expanded;
        let depth = row.depth;
        if expanded == Some(false) {
            self.expanded_explorer_directories.insert(path.clone());
            if let Some(repo) = repo {
                self.request_explorer_directory(repo, path.clone());
            }
            self.rebuild_explorer_rows(repo);
            self.select_explorer_directory(&path);
        } else if self
            .explorer_rows_cache
            .get(index + 1)
            .is_some_and(|child| child.depth > depth)
        {
            self.explorer_state.select(Some(index + 1));
        }
        self.refresh_diff(repo);
    }

    pub(super) fn collapse_or_ascend_explorer(&mut self, repo: Option<&RepositoryData>) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Files);
        self.pending_explorer_selection = None;
        let Some(index) = self.explorer_state.selected() else {
            return;
        };
        let Some(row) = self.explorer_rows_cache.get(index) else {
            return;
        };
        let row_depth = row.depth;
        let directory = row.directory_path.clone();
        if row.directory_expanded == Some(true)
            && let Some(path) = directory
        {
            self.expanded_explorer_directories.remove(&path);
            self.rebuild_explorer_rows(repo);
            self.select_explorer_directory(&path);
            self.refresh_diff(repo);
            return;
        }
        if let Some(parent) = self.explorer_rows_cache[..index]
            .iter()
            .rposition(|candidate| candidate.depth < row_depth)
        {
            self.explorer_state.select(Some(parent));
            self.refresh_diff(repo);
        }
    }

    pub(super) fn toggle_selected_directory(&mut self, repo: Option<&RepositoryData>) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Worktree);
        let Some(repo) = repo else {
            return;
        };
        let Some(path) = self.selected_directory_path(repo) else {
            return;
        };
        let Some(section) = self.selected_worktree_section() else {
            return;
        };
        if !self.collapsed_directories.remove(&path) {
            self.collapsed_directories.insert(path.clone());
        }
        self.rebuild_worktree_rows(Some(repo));
        self.select_directory(repo, &path, section);
        self.refresh_diff(Some(repo));
    }

    pub(super) fn expand_or_descend_worktree(&mut self, repo: Option<&RepositoryData>) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Worktree);
        let Some(repo) = repo else {
            return;
        };
        let Some(index) = self.worktree_state.selected() else {
            return;
        };
        let Some(row) = self.worktree_rows(repo).get(index) else {
            return;
        };
        let Some(path) = row.directory_path.clone() else {
            return;
        };
        let expanded = row.directory_expanded;
        let descend = self
            .worktree_rows(repo)
            .get(index + 1)
            .is_some_and(|child| child.depth > row.depth);
        let section = self.worktree_section(index);
        if expanded == Some(false) {
            self.collapsed_directories.remove(&path);
            self.rebuild_worktree_rows(Some(repo));
            if let Some(section) = section {
                self.select_directory(repo, &path, section);
            }
        } else if descend {
            self.worktree_state.select(Some(index + 1));
        }
        self.refresh_diff(Some(repo));
    }

    pub(super) fn collapse_or_ascend_worktree(&mut self, repo: Option<&RepositoryData>) {
        self.clear_issue_preview();
        self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Worktree);
        let Some(repo) = repo else {
            return;
        };
        let Some(index) = self.worktree_state.selected() else {
            return;
        };
        let Some(row) = self.worktree_rows(repo).get(index) else {
            return;
        };
        let row_depth = row.depth;
        let directory = row.directory_path.clone();
        let section = self.worktree_section(index);
        if let Some(path) = directory
            && row.directory_expanded == Some(true)
        {
            self.collapsed_directories.insert(path.clone());
            self.rebuild_worktree_rows(Some(repo));
            if let Some(section) = section {
                self.select_directory(repo, &path, section);
            }
            self.refresh_diff(Some(repo));
            return;
        }
        if let Some(parent) = self.worktree_rows(repo)[..index]
            .iter()
            .rposition(|candidate| candidate.section.is_none() && candidate.depth < row_depth)
        {
            self.worktree_state.select(Some(parent));
            self.refresh_diff(Some(repo));
        }
    }

    pub(super) fn refresh_diff(&mut self, repo: Option<&RepositoryData>) {
        if matches!(self.preview.origin, PreviewOrigin::Issue(_)) {
            return;
        }
        self.markdown_alternate_scroll = None;
        self.selection_summary = None;
        let preserve_hunk = self.pending_hunk_selection.as_ref().is_some_and(|pending| {
            repo.and_then(|repo| {
                self.selected_change_index(repo)
                    .and_then(|index| repo.changes.get(index))
            })
            .is_some_and(|change| !change.staged && change.path == pending.path)
        });
        if !preserve_hunk {
            self.diff_scroll = 0;
            self.hunk_selection = None;
            self.hunk_pin_pending = false;
            self.pending_hunk_selection = None;
        }
        self.preview_loader.invalidate();
        let Some(repo) = repo else {
            self.preview.origin = PreviewOrigin::IdlePane(self.pane);
            self.set_preview_payload(PreviewPayload::Empty);
            return;
        };
        if self.preview.pane() == LeftPane::Files {
            let Some(row) = self
                .explorer_state
                .selected()
                .and_then(|index| self.explorer_rows_cache.get(index))
            else {
                self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Files);
                self.set_preview_payload(PreviewPayload::Message(
                    "Select a file to preview".to_owned(),
                ));
                return;
            };
            let file_path = row.file_path.clone();
            let directory = row.directory_path.clone();
            let descendant_count = row.descendant_count;
            if let Some(path) = file_path {
                self.preview.origin = PreviewOrigin::ExplorerFile { path: path.clone() };
                self.set_preview_payload(PreviewPayload::Loading);
                self.preview_loader.request_file(&repo.root, path);
            } else if let Some(path) = directory {
                self.preview.origin = PreviewOrigin::ExplorerDirectory { path: path.clone() };
                let loaded = self
                    .file_tree
                    .as_ref()
                    .is_some_and(|tree| tree.has_directory(&path));
                if loaded {
                    self.set_preview_payload(PreviewPayload::Message(format!(
                        "{descendant_count} items in {path}/"
                    )));
                } else {
                    self.set_preview_payload(PreviewPayload::Message(format!("Folder {path}/")));
                }
            }
            return;
        }
        if !repo.details_ready {
            self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Worktree);
            self.set_preview_payload(PreviewPayload::Message(
                if repo.is_local() {
                    "Indexing workspace files…"
                } else {
                    "Loading repository details…"
                }
                .to_owned(),
            ));
            return;
        }
        let rows = self.worktree_rows(repo);
        let Some(row) = self
            .worktree_state
            .selected()
            .and_then(|index| rows.get(index))
        else {
            self.preview.origin = PreviewOrigin::IdlePane(LeftPane::Worktree);
            self.set_preview_payload(PreviewPayload::Message("Working tree clean".to_owned()));
            return;
        };
        if let Some(index) = row.change_index {
            let change = &repo.changes[index];
            self.selection_summary = Some(DiffSummary {
                files: vec![change.path.clone()],
                files_truncated: false,
                additions: change.additions,
                deletions: change.deletions,
            });
            self.preview.origin = PreviewOrigin::WorktreeChange {
                path: change.path.clone(),
                staged: change.staged,
                untracked: change.code == '?',
            };
            self.set_preview_payload(PreviewPayload::Loading);
            self.preview_loader.request_diff(&repo.root, change.clone());
        } else if let Some(section) = row.section.filter(|_| row.section_stats.is_some()) {
            self.selection_summary = Some(worktree_summary(repo, section, None));
            self.preview.origin = PreviewOrigin::WorktreeSection(section);
            self.set_preview_payload(PreviewPayload::Loading);
            self.preview_loader.request_section_diff(
                &repo.root,
                repo.changes.clone(),
                section == WorktreeSection::Staged,
            );
        } else if let Some(path) = &row.directory_path {
            let path = path.clone();
            let descendant_count = row.descendant_count;
            let section = self
                .selected_worktree_section()
                .unwrap_or(WorktreeSection::Unstaged);
            self.selection_summary = Some(worktree_summary(repo, section, Some(&path)));
            self.preview.origin = PreviewOrigin::WorktreeDirectory {
                path: path.clone(),
                section,
            };
            self.set_preview_payload(PreviewPayload::Message(format!(
                "{} changed files in {path}/",
                descendant_count
            )));
        }
    }

    pub(crate) fn selection_summary(&self) -> Option<&DiffSummary> {
        self.selection_summary.as_ref()
    }

    pub(super) fn poll_preview(&mut self, active_root: Option<&Path>) -> bool {
        let Some(content) = self.preview_loader.poll(active_root) else {
            return false;
        };
        match content {
            LoadedPreview::Text(content) => {
                let payload = match self.preview.origin {
                    PreviewOrigin::ExplorerFile { .. } => PreviewPayload::Source(content),
                    PreviewOrigin::WorktreeChange {
                        untracked: true, ..
                    } => PreviewPayload::Diff(DiffDocument::parse_untracked(content)),
                    PreviewOrigin::WorktreeChange { .. }
                    | PreviewOrigin::WorktreeSection(_)
                    | PreviewOrigin::Commit { .. }
                    | PreviewOrigin::BranchComparison(_)
                    | PreviewOrigin::Issue(IssuePreview {
                        pull_request: Some(_),
                        ..
                    }) => PreviewPayload::Diff(DiffDocument::parse(content)),
                    _ => PreviewPayload::Message(content),
                };
                self.set_preview_payload(payload);
            }
            LoadedPreview::Error(content) => {
                self.set_preview_payload(PreviewPayload::Error(content))
            }
            LoadedPreview::Database { path, database } => self.set_database(path, database),
            LoadedPreview::DatabasePage { path, key, result } => {
                if let Some(browser) = self.preview.database_mut()
                    && browser.path == path
                {
                    browser.apply_page(&key, result);
                }
            }
            LoadedPreview::Image(image) => self.set_image(image),
        }
        if let Some(pending) = self.pending_hunk_selection.take() {
            let count = self.preview.document().map_or(0, DiffDocument::hunk_count);
            self.hunk_selection = (count > 0).then(|| pending.index.min(count - 1));
            self.hunk_pin_pending = self.hunk_selection.is_some();
        }
        true
    }

    pub(super) fn poll_directories(&mut self, repo: Option<&RepositoryData>) -> bool {
        let mut changed = false;
        let mut directories_changed = false;
        let mut selected_index = None;
        let mut selected_offset = None;
        let mut selected = None;
        while let Some(completion) = self.directory_loader.poll() {
            let Some(repo) = repo else {
                continue;
            };
            if completion.generation != self.directory_generation || completion.root != repo.root {
                continue;
            }
            self.loading_directories.remove(&completion.directory);
            let entries = match completion.result {
                Ok(entries) => entries,
                Err(error) => {
                    self.failed_directories.insert(completion.directory.clone());
                    let blocks_pending =
                        self.pending_explorer_selection
                            .as_ref()
                            .is_some_and(|(path, _)| {
                                completion.directory.is_empty()
                                    || path.as_path().starts_with(completion.directory.as_path())
                            });
                    let selected_directory = self
                        .selected_explorer_directory_path()
                        .is_some_and(|path| path == completion.directory);
                    if blocks_pending {
                        self.pending_explorer_selection = None;
                    }
                    if self.branch_comparison().is_none()
                        && (completion.directory.is_empty() || blocks_pending || selected_directory)
                    {
                        self.preview_loader.invalidate();
                        self.set_preview_payload(PreviewPayload::Error(error));
                        changed = true;
                    }
                    continue;
                }
            };
            self.failed_directories.remove(&completion.directory);
            let replaced = self
                .file_tree
                .as_mut()
                .is_some_and(|tree| tree.replace_directory(completion.directory, entries));
            if !replaced {
                continue;
            }
            if !directories_changed {
                selected_index = self.explorer_state.selected();
                selected_offset =
                    selected_index.map(|index| index.saturating_sub(self.explorer_scroll));
                selected = selected_index.and_then(|index| self.explorer_entry(repo, index));
            }
            changed = true;
            directories_changed = true;
        }
        if directories_changed {
            self.rebuild_explorer_rows(None);
            if let Some(selected) = selected {
                let row = self.explorer_rows().iter().position(|row| {
                    row.directory_path.as_ref() == Some(&selected.path)
                        || row.file_path.as_ref() == Some(&selected.path)
                });
                let row = row.or_else(|| self.initial_explorer_row());
                self.explorer_state.select(row);
                self.explorer_scroll = row
                    .zip(selected_offset)
                    .map_or(0, |(row, offset)| row.saturating_sub(offset));
            } else if selected_index.is_none() {
                self.explorer_state.select(self.initial_explorer_row());
                self.explorer_scroll = 0;
            }
        }
        if directories_changed && self.branch_comparison().is_some() {
            return changed;
        }
        if directories_changed
            && let Some((path, viewport)) = self.pending_explorer_selection.clone()
        {
            if !self.select_explorer_path(
                repo.expect("changed completion has repository"),
                &path,
                viewport,
            ) {
                self.refresh_diff(repo);
            }
        } else if directories_changed {
            self.refresh_diff(repo);
        }
        changed
    }

    pub(super) fn shutdown(&mut self) {
        self.directory_loader.shutdown();
        self.preview_loader.shutdown();
        self.preview_presentation.shutdown();
    }

    fn rebuild_explorer_rows(&mut self, repo: Option<&RepositoryData>) {
        // Directory completions do not carry repository data and must not clear Git status.
        if repo.is_some() {
            self.sync_repository_caches(repo);
        }
        let rows = self.file_tree.as_ref().map_or_else(Vec::new, |tree| {
            tree.rows_expanded(&self.expanded_explorer_directories)
        });
        let previous = std::mem::replace(&mut self.explorer_rows_cache, rows);
        if previous.len() >= 10_000 {
            crate::diagnostics::drop_in_background("explorer-rows", previous);
        }
    }

    fn rebuild_worktree_rows(&mut self, repo: Option<&RepositoryData>) {
        self.sync_repository_caches(repo);
        self.worktree_rows_cache = self
            .worktree_tree
            .as_ref()
            .map_or_else(Vec::new, |tree| tree.rows(&self.collapsed_directories));
        self.worktree_rows_generation = self.worktree_rows_generation.wrapping_add(1);
    }

    fn request_explorer_directory(&mut self, repo: &RepositoryData, directory: RepoPath) {
        let loaded = self
            .file_tree
            .as_ref()
            .is_some_and(|tree| tree.has_directory(&directory));
        let retry_failed = self.failed_directories.remove(&directory);
        if loaded && !retry_failed {
            if self.loading_directories.contains(&directory) {
                self.directory_loader
                    .prioritize(self.directory_generation, &directory);
            }
            return;
        }
        if !self.loading_directories.insert(directory.clone()) {
            self.directory_loader
                .prioritize(self.directory_generation, &directory);
            return;
        }
        self.directory_loader
            .request_interactive(self.directory_generation, &repo.root, directory);
    }

    fn request_explorer_ancestors(&mut self, repo: &RepositoryData, path: &RepoPath) {
        self.request_explorer_directory(repo, RepoPath::default());
        let mut directories = Vec::new();
        let mut parent = path.parent();
        while let Some(path) = parent {
            directories.push(path.clone());
            parent = path.parent();
        }
        for directory in directories.into_iter().rev() {
            self.request_explorer_directory(repo, directory);
        }
    }

    fn refresh_explorer_directories(&mut self, repo: &RepositoryData, refresh: InventoryRefresh) {
        if refresh == InventoryRefresh::Unchanged {
            return;
        }
        let pending: Vec<_> = self.loading_directories.drain().collect();
        let tree = self.file_tree.as_ref();
        let refresh_all = refresh == InventoryRefresh::All;
        let mut directories = match refresh {
            InventoryRefresh::Unchanged => unreachable!(),
            InventoryRefresh::All => tree.map_or_else(Vec::new, FileTree::loaded_directories),
            InventoryRefresh::Directories(directories) => directories
                .into_iter()
                .filter(|directory| tree.is_some_and(|tree| tree.has_directory(directory)))
                .collect(),
        };
        directories.extend(pending);
        directories.sort_unstable();
        directories.dedup();
        if refresh_all && !directories.iter().any(RepoPath::is_empty) {
            directories.push(RepoPath::default());
        }
        if directories.is_empty() {
            return;
        }
        self.directory_generation = self.directory_generation.wrapping_add(1);
        for directory in directories {
            self.loading_directories.insert(directory.clone());
            self.directory_loader.request_background(
                self.directory_generation,
                &repo.root,
                directory,
            );
        }
    }

    fn sync_repository_caches(&mut self, repo: Option<&RepositoryData>) {
        let changes_fingerprint = repo.map(|repo| repo.changes_fingerprint);
        if self.worktree_tree_fingerprint != changes_fingerprint {
            self.worktree_tree = repo.map(|repo| WorktreeTree::new(&repo.changes));
            self.change_codes = repo.map_or_else(HashMap::new, |repo| change_codes(&repo.changes));
            self.worktree_tree_fingerprint = changes_fingerprint;
        }
    }

    pub(crate) fn explorer_change_code(&self, path: &RepoPath) -> Option<char> {
        self.change_codes.get(path).copied()
    }

    pub(crate) fn set_preview_payload(&mut self, payload: PreviewPayload) {
        self.preview.payload = payload;
        self.preview.generation = self.preview.generation.wrapping_add(1);
        self.preview_presentation.clear();
    }

    #[cfg(test)]
    pub(crate) fn set_source_for_test(&mut self, content: String) {
        self.set_preview_payload(PreviewPayload::Source(content));
    }

    #[cfg(test)]
    pub(crate) fn set_diff_for_test(&mut self, content: String) {
        self.set_preview_payload(PreviewPayload::Diff(DiffDocument::parse(content)));
    }

    #[cfg(test)]
    pub(crate) fn set_pull_request_for_test(&mut self, body: &str, content: String) {
        self.preview.origin = PreviewOrigin::Issue(IssuePreview {
            number: 17,
            kind: "PULL".to_owned(),
            title: "Improve previews".to_owned(),
            body: Arc::from(body),
            pull_request: Some(PullRequestPreview {
                base_ref_name: "main".to_owned(),
                base_ref_oid: "base".to_owned(),
                head_ref_name: "topic".to_owned(),
                head_ref_oid: "head".to_owned(),
                changed_files: Some(1),
                additions: Some(1),
                deletions: Some(1),
            }),
        });
        self.markdown_rendered = false;
        self.set_diff_for_test(content);
    }

    pub(crate) fn show_issue(&mut self, number: u64, kind: &str, title: &str, body: &str) {
        self.preview_loader.invalidate();
        self.preview.origin = PreviewOrigin::Issue(IssuePreview {
            number,
            kind: kind.to_owned(),
            title: title.to_owned(),
            body: Arc::from(body),
            pull_request: None,
        });
        self.diff_scroll = 0;
        self.markdown_rendered = true;
        self.markdown_alternate_scroll = None;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.pending_preview_line = None;
        self.set_preview_payload(PreviewPayload::Source(if body.trim().is_empty() {
            "_No description provided._".to_owned()
        } else {
            body.to_owned()
        }));
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show_pull_request(
        &mut self,
        root: &Path,
        repository: &str,
        repository_url: &str,
        number: u64,
        title: &str,
        body: &str,
        base_ref_name: &str,
        base_ref_oid: &str,
        head_ref_name: &str,
        head_ref_oid: &str,
        changed_files: Option<u64>,
        additions: Option<u64>,
        deletions: Option<u64>,
    ) {
        self.preview.origin = PreviewOrigin::Issue(IssuePreview {
            number,
            kind: "PULL".to_owned(),
            title: title.to_owned(),
            body: Arc::from(if body.trim().is_empty() {
                "_No description provided._"
            } else {
                body
            }),
            pull_request: Some(PullRequestPreview {
                base_ref_name: base_ref_name.to_owned(),
                base_ref_oid: base_ref_oid.to_owned(),
                head_ref_name: head_ref_name.to_owned(),
                head_ref_oid: head_ref_oid.to_owned(),
                changed_files,
                additions,
                deletions,
            }),
        });
        self.diff_scroll = 0;
        self.markdown_rendered = false;
        self.markdown_alternate_scroll = None;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.pending_preview_line = None;
        self.set_preview_payload(PreviewPayload::Loading);
        self.preview_loader.request_pull_request_diff(
            root,
            repository.to_owned(),
            repository_url.to_owned(),
            base_ref_oid.to_owned(),
            head_ref_oid.to_owned(),
        );
    }

    fn clear_issue_preview(&mut self) {
        if matches!(self.preview.origin, PreviewOrigin::Issue(_)) {
            self.preview.origin = PreviewOrigin::IdlePane(self.pane);
        }
    }

    pub(crate) fn pin_preview_line(&mut self, path: RepoPath, line: usize) {
        self.pending_preview_line = Some((path, line.max(1)));
        self.markdown_rendered = false;
    }

    pub(crate) fn take_preview_line(&mut self, path: &RepoPath) -> Option<usize> {
        if matches!(self.preview.payload, PreviewPayload::Loading) {
            return None;
        }
        if self
            .pending_preview_line
            .as_ref()
            .is_some_and(|(pending, _)| pending == path)
        {
            self.pending_preview_line.take().map(|(_, line)| line)
        } else {
            None
        }
    }

    fn set_database(&mut self, path: RepoPath, database: SqliteDatabase) {
        self.diff_scroll = 0;
        self.markdown_rendered = false;
        let generation = self.preview.generation.wrapping_add(1);
        self.set_preview_payload(PreviewPayload::Database(SqliteBrowser::new(
            path, database, generation,
        )));
    }

    fn set_image(&mut self, image: Arc<DynamicImage>) {
        self.diff_scroll = 0;
        self.markdown_rendered = false;
        self.set_preview_payload(PreviewPayload::Image(image));
    }

    fn select_initial_rows(&mut self, repo: Option<&RepositoryData>) {
        self.worktree_state
            .select(repo.and_then(|repo| self.first_change_row(repo)));
        self.explorer_state.select(self.initial_explorer_row());
    }

    fn select_explorer_directory(&mut self, path: &RepoPath) {
        let row = self
            .explorer_rows()
            .iter()
            .position(|row| row.directory_path.as_ref() == Some(path));
        self.explorer_state.select(row);
    }

    fn row_for_explorer_file(&self, path: &RepoPath) -> Option<usize> {
        self.explorer_rows()
            .iter()
            .position(|row| row.file_path.as_ref() == Some(path))
    }

    fn first_explorer_file_row(&self) -> Option<usize> {
        self.explorer_rows()
            .iter()
            .position(|row| row.file_path.is_some())
    }

    fn initial_explorer_row(&self) -> Option<usize> {
        self.first_explorer_file_row()
            .or_else(|| (!self.explorer_rows().is_empty()).then_some(0))
    }

    fn select_directory(
        &mut self,
        repo: &RepositoryData,
        path: &RepoPath,
        section: WorktreeSection,
    ) {
        let mut current_section = None;
        let row = self.worktree_rows(repo).iter().position(|row| {
            if let Some(row_section) = row.section {
                current_section = Some(row_section);
            }
            row.directory_path.as_ref() == Some(path) && current_section == Some(section)
        });
        self.worktree_state.select(row);
    }

    fn row_for_change(&self, repo: &RepositoryData, change_index: usize) -> Option<usize> {
        self.worktree_rows(repo)
            .iter()
            .position(|row| row.change_index == Some(change_index))
    }

    fn first_change_row(&self, repo: &RepositoryData) -> Option<usize> {
        self.worktree_rows(repo)
            .iter()
            .position(|row| row.change_index.is_some())
    }

    fn last_change_row(&self, repo: &RepositoryData) -> Option<usize> {
        self.worktree_rows(repo)
            .iter()
            .rposition(|row| row.change_index.is_some())
    }

    fn preview_selection(&self) -> (LeftPane, Option<usize>) {
        let pane = self.preview.pane();
        let selected = if pane == LeftPane::Files {
            self.explorer_state.selected()
        } else {
            self.worktree_state.selected()
        };
        (pane, selected)
    }
}

fn worktree_summary(
    repo: &RepositoryData,
    section: WorktreeSection,
    directory: Option<&RepoPath>,
) -> DiffSummary {
    repo.changes
        .iter()
        .filter(|change| change.staged == (section == WorktreeSection::Staged))
        .filter(|change| {
            directory.is_none_or(|path| change.path.as_path().starts_with(path.as_path()))
        })
        .fold(DiffSummary::default(), |mut summary, change| {
            summary.files.push(change.path.clone());
            summary.additions = summary.additions.saturating_add(change.additions);
            summary.deletions = summary.deletions.saturating_add(change.deletions);
            summary
        })
}

fn change_codes(changes: &[Change]) -> HashMap<RepoPath, char> {
    let mut codes = HashMap::new();
    for change in changes {
        let mut path = Some(change.path.clone());
        while let Some(current) = path {
            codes
                .entry(current.clone())
                .and_modify(|code| {
                    if change_code_priority(change.code) < change_code_priority(*code) {
                        *code = change.code;
                    }
                })
                .or_insert(change.code);
            path = current.parent();
        }
    }
    codes
}

fn expand_ancestors(expanded: &mut HashSet<RepoPath>, path: &RepoPath) {
    let mut ancestor = path.parent();
    while let Some(parent) = ancestor {
        ancestor = parent.parent();
        expanded.insert(parent);
    }
}

fn change_code_priority(code: char) -> u8 {
    match code {
        'D' | 'U' => 0,
        '?' => 1,
        'A' => 2,
        'R' => 3,
        'C' => 4,
        'M' => 5,
        'T' => 6,
        _ => 7,
    }
}

fn move_list(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0);
    let next = (current as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
    state.select(Some(next));
}

fn move_worktree_selection(state: &mut ListState, rows: &[WorktreeRow], delta: isize) {
    if rows.is_empty() || delta == 0 {
        return;
    }
    let direction = delta.signum();
    let mut remaining = delta.unsigned_abs();
    let mut index = state.selected().unwrap_or_else(|| {
        if direction > 0 {
            0
        } else {
            rows.len().saturating_sub(1)
        }
    });
    while remaining > 0 {
        let next = if direction > 0 {
            (index + 1..rows.len()).find(|candidate| rows[*candidate].section.is_none())
        } else {
            (0..index)
                .rev()
                .find(|candidate| rows[*candidate].section.is_none())
        };
        let Some(next) = next else {
            break;
        };
        index = next;
        remaining -= 1;
    }
    if rows.get(index).is_some_and(|row| row.section.is_none()) {
        state.select(Some(index));
    }
}

fn scroll_viewport(scroll: &mut usize, len: usize, viewport: usize, delta: isize) {
    let maximum = len.saturating_sub(viewport);
    *scroll = if delta > 0 {
        scroll.saturating_add(delta as usize).min(maximum)
    } else {
        scroll.saturating_sub(delta.unsigned_abs())
    };
}

fn ensure_selection_visible(scroll: &mut usize, selected: Option<usize>, viewport: usize) {
    let Some(selected) = selected else { return };
    if viewport == 0 {
        return;
    }
    if selected < *scroll {
        *scroll = selected;
    } else if selected >= scroll.saturating_add(viewport) {
        *scroll = selected.saturating_add(1).saturating_sub(viewport);
    }
}

#[cfg(test)]
mod tests;
