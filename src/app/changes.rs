mod preview_loader;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use image::DynamicImage;
use ratatui::widgets::ListState;

use crate::{
    git::{Change, Commit, RepositoryData},
    repo_path::RepoPath,
    tree::{ExplorerRow, FileTree, PreparedFileTree, WorktreeRow, WorktreeSection, WorktreeTree},
    ui::preview::PreviewPresentation,
};

use preview_loader::{LoadedPreview, PreviewLoader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftPane {
    Worktree,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangesHitTarget {
    WorktreeTab,
    FilesTab,
    StageAll,
    WorktreeBackground(u64),
    WorktreeRow { generation: u64, index: usize },
    WorktreeStage { generation: u64, index: usize },
    HunkAction { generation: u64, index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChangesEffect {
    PaneActivated,
    WorktreeDirectoryActivated,
    ToggleAllStaging,
    ToggleSelectedStage,
    StageHunk(usize),
    WorktreeFileSelected { path: RepoPath, staged: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExplorerEntry {
    pub(super) path: RepoPath,
    pub(super) is_directory: bool,
}

pub struct ChangesState {
    pub(crate) pane: LeftPane,
    pub(crate) worktree_state: ListState,
    pub(crate) explorer_state: ListState,
    pub(crate) worktree_scroll: usize,
    pub(crate) worktree_scroll_to_selection: bool,
    pub(crate) explorer_scroll: usize,
    pub(crate) history_state: ListState,
    pub(crate) diff: String,
    pub(crate) diff_scroll: usize,
    pub(crate) diff_wrap: bool,
    pub(crate) markdown_rendered: bool,
    markdown_alternate_scroll: Option<usize>,
    pub(crate) hunk_selection: Option<usize>,
    hunk_pin_pending: bool,
    pending_hunk_selection: Option<PendingHunkSelection>,
    pub(crate) history_focused: bool,
    pub(crate) collapsed_directories: HashSet<RepoPath>,
    pub(crate) expanded_explorer_directories: HashSet<RepoPath>,
    worktree_rows_cache: Vec<WorktreeRow>,
    worktree_rows_generation: u64,
    explorer_rows_cache: Vec<ExplorerRow>,
    file_tree: Option<FileTree>,
    file_tree_fingerprint: Option<u64>,
    worktree_tree: Option<WorktreeTree>,
    worktree_tree_fingerprint: Option<u64>,
    change_codes: HashMap<RepoPath, char>,
    pub(crate) preview_content_generation: u64,
    pub(crate) preview_image: Option<Arc<DynamicImage>>,
    pub(crate) preview_presentation: PreviewPresentation,
    preview_loader: PreviewLoader,
}

struct PendingHunkSelection {
    path: RepoPath,
    index: usize,
}

pub(super) struct ChangesSelection {
    change: Option<(RepoPath, bool)>,
    directory: Option<(RepoPath, WorktreeSection)>,
    explorer_file: Option<RepoPath>,
    explorer_directory: Option<RepoPath>,
    history_oid: Option<String>,
}

impl ChangesState {
    pub(super) fn new(repo: Option<&RepositoryData>) -> Self {
        let file_tree = repo.map(|repo| FileTree::new(&repo.files, &repo.directories));
        let mut state = Self {
            pane: if repo.is_some_and(RepositoryData::is_local) {
                LeftPane::Files
            } else {
                LeftPane::Worktree
            },
            worktree_state: ListState::default(),
            explorer_state: ListState::default(),
            worktree_scroll: 0,
            worktree_scroll_to_selection: true,
            explorer_scroll: 0,
            history_state: ListState::default(),
            diff: String::new(),
            diff_scroll: 0,
            diff_wrap: false,
            markdown_rendered: false,
            markdown_alternate_scroll: None,
            hunk_selection: None,
            hunk_pin_pending: false,
            pending_hunk_selection: None,
            history_focused: false,
            collapsed_directories: HashSet::new(),
            expanded_explorer_directories: HashSet::new(),
            worktree_rows_cache: Vec::new(),
            worktree_rows_generation: 0,
            explorer_rows_cache: Vec::new(),
            file_tree,
            file_tree_fingerprint: repo.map(|repo| repo.files_fingerprint),
            worktree_tree: repo.map(|repo| WorktreeTree::new(&repo.changes)),
            worktree_tree_fingerprint: repo.map(|repo| repo.changes_fingerprint),
            change_codes: repo.map_or_else(HashMap::new, |repo| change_codes(&repo.changes)),
            preview_content_generation: 0,
            preview_image: None,
            preview_presentation: PreviewPresentation::default(),
            preview_loader: PreviewLoader::new(),
        };
        state.rebuild_worktree_rows(repo);
        state.rebuild_explorer_rows(repo);
        state.select_initial_rows(repo);
        state.refresh_diff(repo);
        state
    }

    pub(super) fn reset_repository(
        &mut self,
        repo: Option<&RepositoryData>,
        prepared_file_tree: Option<PreparedFileTree>,
    ) {
        self.pane = if repo.is_some_and(RepositoryData::is_local) {
            LeftPane::Files
        } else {
            LeftPane::Worktree
        };
        self.worktree_state = ListState::default();
        self.explorer_state = ListState::default();
        self.worktree_scroll = 0;
        self.worktree_scroll_to_selection = true;
        self.explorer_scroll = 0;
        self.history_state = ListState::default();
        self.set_diff(String::new());
        self.diff_scroll = 0;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.history_focused = false;
        self.collapsed_directories.clear();
        self.expanded_explorer_directories.clear();
        if let Some(prepared) = prepared_file_tree {
            let tree = prepared.into_tree();
            if let Some(previous) = self.file_tree.replace(tree) {
                crate::diagnostics::drop_in_background("file-tree", previous);
            }
            self.file_tree_fingerprint = repo.map(|repo| repo.files_fingerprint);
        } else {
            self.sync_repository_caches(repo);
        }
        self.rebuild_worktree_rows(repo);
        self.rebuild_explorer_rows(repo);
        self.select_initial_rows(repo);
        self.refresh_diff(repo);
    }

    pub(super) fn capture_selection(&self, repo: &RepositoryData) -> ChangesSelection {
        ChangesSelection {
            change: self
                .selected_change_index(repo)
                .and_then(|index| repo.changes.get(index))
                .map(|change| (change.path.clone(), change.staged)),
            directory: self.selected_directory_path(repo).and_then(|path| {
                let section = self.selected_worktree_section()?;
                Some((path, section))
            }),
            explorer_file: self.selected_explorer_file_path(repo).cloned(),
            explorer_directory: self.selected_explorer_directory_path(),
            history_oid: self
                .history_state
                .selected()
                .and_then(|index| repo.history.get(index))
                .map(|commit| commit.oid.clone()),
        }
    }

    pub(super) fn restore_selection(&mut self, repo: &RepositoryData, selection: ChangesSelection) {
        self.rebuild_worktree_rows(Some(repo));
        self.rebuild_explorer_rows(Some(repo));

        let change_index = selection.change.and_then(|(path, staged)| {
            repo.changes
                .iter()
                .position(|change| change.path == path && change.staged == staged)
                .or_else(|| repo.changes.iter().position(|change| change.path == path))
        });
        let change_row = change_index
            .and_then(|index| self.row_for_change(repo, index))
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

        let history_index = selection
            .history_oid
            .and_then(|oid| repo.history.iter().position(|commit| commit.oid == oid));
        self.history_state.select(history_index);

        let explorer_row = selection
            .explorer_file
            .and_then(|path| {
                let file_index = repo.files.iter().position(|candidate| candidate == &path)?;
                self.row_for_explorer_file(file_index)
            })
            .or_else(|| {
                let directory = selection.explorer_directory.as_ref()?;
                self.explorer_rows()
                    .iter()
                    .position(|row| row.directory_path.as_ref() == Some(directory))
            })
            .or_else(|| self.initial_explorer_row());
        self.explorer_state.select(explorer_row);
        self.refresh_diff(Some(repo));
    }

    pub(crate) fn worktree_rows(&self, _repo: &RepositoryData) -> &[WorktreeRow] {
        &self.worktree_rows_cache
    }

    pub(crate) fn explorer_rows(&self) -> &[ExplorerRow] {
        &self.explorer_rows_cache
    }

    pub(crate) fn selected_explorer_file_path<'a>(
        &self,
        repo: &'a RepositoryData,
    ) -> Option<&'a RepoPath> {
        let selected = self.explorer_state.selected()?;
        let file_index = self.explorer_rows().get(selected)?.file_index?;
        repo.files.get(file_index)
    }

    pub(super) fn selected_change_index(&self, repo: &RepositoryData) -> Option<usize> {
        let selected = self.worktree_state.selected()?;
        self.worktree_rows(repo).get(selected)?.change_index
    }

    pub(super) fn has_preview_target(&self, repo: &RepositoryData) -> bool {
        match self.pane {
            LeftPane::Files => self
                .explorer_state
                .selected()
                .and_then(|index| self.explorer_rows_cache.get(index))
                .is_some_and(|row| row.file_index.is_some() || row.directory_path.is_some()),
            LeftPane::Worktree => {
                if self.history_focused
                    && self
                        .history_state
                        .selected()
                        .is_some_and(|index| repo.history.get(index).is_some())
                {
                    return true;
                }
                self.worktree_state
                    .selected()
                    .and_then(|index| self.worktree_rows_cache.get(index))
                    .is_some_and(|row| row.change_index.is_some() || row.directory_path.is_some())
            }
        }
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
        repo: &RepositoryData,
        index: usize,
    ) -> Option<ExplorerEntry> {
        let row = self.explorer_rows().get(index)?;
        if let Some(file_index) = row.file_index {
            return Some(ExplorerEntry {
                path: repo.files.get(file_index)?.clone(),
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
        expand_ancestors(&mut self.expanded_explorer_directories, path);
        self.rebuild_explorer_rows(Some(repo));
        let row = self.explorer_rows().iter().position(|row| {
            row.directory_path.as_ref() == Some(path)
                || row
                    .file_index
                    .and_then(|index| repo.files.get(index))
                    .is_some_and(|candidate| candidate == path)
        });
        let Some(row) = row else {
            return false;
        };
        self.explorer_state.select(Some(row));
        ensure_selection_visible(&mut self.explorer_scroll, Some(row), viewport);
        self.refresh_diff(Some(repo));
        true
    }

    pub(super) fn set_pane(&mut self, pane: LeftPane, repo: Option<&RepositoryData>) -> bool {
        if self.pane == pane {
            return false;
        }
        self.pane = pane;
        self.clear_history_selection();
        if pane == LeftPane::Files && self.explorer_state.selected().is_none() {
            self.explorer_state.select(self.initial_explorer_row());
        }
        self.refresh_diff(repo);
        true
    }

    pub(super) fn select_worktree_row(&mut self, repo: &RepositoryData, index: usize) -> bool {
        if self
            .worktree_rows(repo)
            .get(index)
            .is_none_or(|row| row.section.is_some())
        {
            return false;
        }
        self.worktree_state.select(Some(index));
        self.clear_history_selection();
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
                self.set_pane(LeftPane::Worktree, Some(repo));
                Some(ChangesEffect::PaneActivated)
            }
            ChangesHitTarget::FilesTab => {
                self.set_pane(LeftPane::Files, Some(repo));
                Some(ChangesEffect::PaneActivated)
            }
            ChangesHitTarget::StageAll => {
                if self.pane != LeftPane::Worktree {
                    return None;
                }
                self.clear_history_selection();
                Some(ChangesEffect::ToggleAllStaging)
            }
            ChangesHitTarget::WorktreeBackground(generation) => {
                if !self.is_current_worktree_target(generation) {
                    return None;
                }
                self.clear_history_selection();
                self.refresh_diff(Some(repo));
                None
            }
            ChangesHitTarget::WorktreeRow { generation, index } => {
                if !self.is_current_worktree_target(generation) {
                    return None;
                }
                if !self.select_worktree_row(repo, index) {
                    self.clear_history_selection();
                    self.refresh_diff(Some(repo));
                    return None;
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
            ChangesHitTarget::HunkAction { generation, index } => (generation
                == self.preview_content_generation)
                .then_some(ChangesEffect::StageHunk(index)),
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
            generation: self.preview_content_generation,
            index,
        }
    }

    fn is_current_worktree_target(&self, generation: u64) -> bool {
        self.pane == LeftPane::Worktree && generation == self.worktree_rows_generation
    }

    pub(super) fn select_explorer_row(&mut self, repo: &RepositoryData, index: usize) -> bool {
        if index >= self.explorer_rows().len() {
            return false;
        }
        self.explorer_state.select(Some(index));
        self.refresh_diff(Some(repo));
        true
    }

    pub(super) fn select_explorer_file(
        &mut self,
        repo: &RepositoryData,
        file_index: usize,
        viewport: usize,
    ) -> bool {
        let Some(path) = repo.files.get(file_index) else {
            return false;
        };
        expand_ancestors(&mut self.expanded_explorer_directories, path);
        self.rebuild_explorer_rows(Some(repo));
        let Some(row) = self.row_for_explorer_file(file_index) else {
            return false;
        };
        self.explorer_state.select(Some(row));
        if viewport == 0 {
            self.explorer_scroll = row;
        } else {
            ensure_selection_visible(&mut self.explorer_scroll, Some(row), viewport);
        }
        self.refresh_diff(Some(repo));
        true
    }

    pub(super) fn select_history_row(
        &mut self,
        repo: &RepositoryData,
        relative_row: usize,
    ) -> bool {
        let mut rendered_row = 0;
        let index = (self.history_state.offset()..repo.history.len()).find(|index| {
            let height = if repo.history[*index].refs.is_empty() {
                1
            } else {
                2
            };
            let contains = relative_row < rendered_row + height;
            rendered_row += height;
            contains
        });
        let Some(index) = index else {
            return false;
        };
        self.history_state.select(Some(index));
        self.history_focused = true;
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
        } else if self.history_focused {
            move_list(&mut self.history_state, repo.history.len(), delta);
        } else {
            move_worktree_selection(&mut self.worktree_state, &self.worktree_rows_cache, delta);
            ensure_selection_visible(
                &mut self.worktree_scroll,
                self.worktree_state.selected(),
                worktree_viewport,
            );
        }
        if self.preview_selection() != previous {
            self.refresh_diff(Some(repo));
        }
    }

    pub(super) fn move_history_selection(&mut self, repo: &RepositoryData, delta: isize) {
        let previous = self.preview_selection();
        self.history_focused = true;
        move_list(&mut self.history_state, repo.history.len(), delta);
        if self.preview_selection() != previous {
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
            self.explorer_state
                .select((!self.explorer_rows().is_empty()).then_some(0));
            ensure_selection_visible(
                &mut self.explorer_scroll,
                self.explorer_state.selected(),
                explorer_viewport,
            );
        } else if self.history_focused {
            self.history_state
                .select((!repo.history.is_empty()).then_some(0));
        } else {
            self.worktree_state.select(self.first_change_row(repo));
            ensure_selection_visible(
                &mut self.worktree_scroll,
                self.worktree_state.selected(),
                worktree_viewport,
            );
        }
        if self.preview_selection() != previous {
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
            self.explorer_state
                .select(self.explorer_rows().len().checked_sub(1));
            ensure_selection_visible(
                &mut self.explorer_scroll,
                self.explorer_state.selected(),
                explorer_viewport,
            );
        } else if self.history_focused {
            self.history_state.select(repo.history.len().checked_sub(1));
        } else {
            self.worktree_state.select(self.last_change_row(repo));
            ensure_selection_visible(
                &mut self.worktree_scroll,
                self.worktree_state.selected(),
                worktree_viewport,
            );
        }
        if self.preview_selection() != previous {
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
        self.diff_scroll = 0;
        self.markdown_alternate_scroll = None;
        self.hunk_selection = None;
        self.hunk_pin_pending = false;
        self.pending_hunk_selection = None;
        self.set_diff("Loading preview…".to_owned());
        self.preview_loader
            .request_commit(&repo.root, commit.oid.clone());
    }

    pub(super) fn enter_hunk_selection(&mut self, repo: &RepositoryData) -> bool {
        let Some(change) = self
            .selected_change_index(repo)
            .and_then(|index| repo.changes.get(index))
        else {
            return false;
        };
        if change.staged || hunk_count(&self.diff) == 0 {
            return false;
        }
        self.hunk_selection = Some(0);
        self.hunk_pin_pending = true;
        self.diff_scroll = 0;
        true
    }

    pub(super) fn move_hunk_selection(&mut self, delta: isize) {
        let count = hunk_count(&self.diff);
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
            && index < hunk_count(&self.diff)
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

    pub(super) fn clear_history_selection(&mut self) {
        self.history_focused = false;
        self.history_state.select(None);
    }

    pub(super) fn toggle_selected_explorer_directory(&mut self, repo: Option<&RepositoryData>) {
        let Some(path) = self.selected_explorer_directory_path() else {
            return;
        };
        if !self.expanded_explorer_directories.remove(&path) {
            self.expanded_explorer_directories.insert(path.clone());
        }
        self.rebuild_explorer_rows(repo);
        self.select_explorer_directory(&path);
        self.refresh_diff(repo);
    }

    pub(super) fn expand_or_descend_explorer(&mut self, repo: Option<&RepositoryData>) {
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
        self.markdown_alternate_scroll = None;
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
            self.set_diff(String::new());
            return;
        };
        if self.pane == LeftPane::Files {
            let Some(row) = self
                .explorer_state
                .selected()
                .and_then(|index| self.explorer_rows_cache.get(index))
            else {
                self.set_diff("Select a file to preview".to_owned());
                return;
            };
            if let Some(index) = row.file_index {
                self.set_diff("Loading preview…".to_owned());
                self.preview_loader
                    .request_file(&repo.root, repo.files[index].clone());
            } else if let Some(path) = &row.directory_path {
                self.set_diff(format!("{} files in {path}/", row.descendant_count));
            }
            return;
        }
        if self.history_focused
            && let Some(commit) = self
                .history_state
                .selected()
                .and_then(|index| repo.history.get(index))
        {
            self.set_diff("Loading preview…".to_owned());
            self.preview_loader
                .request_commit(&repo.root, commit.oid.clone());
            return;
        }
        let rows = self.worktree_rows(repo);
        let Some(row) = self
            .worktree_state
            .selected()
            .and_then(|index| rows.get(index))
        else {
            self.set_diff("Working tree clean".to_owned());
            return;
        };
        if let Some(index) = row.change_index {
            self.set_diff("Loading preview…".to_owned());
            self.preview_loader
                .request_diff(&repo.root, repo.changes[index].clone());
        } else if let Some(path) = &row.directory_path {
            self.set_diff(format!("{} changed files in {path}/", row.descendant_count));
        }
    }

    pub(super) fn poll_preview(&mut self, active_root: Option<&Path>) -> bool {
        let Some(content) = self.preview_loader.poll(active_root) else {
            return false;
        };
        match content {
            LoadedPreview::Text(content) | LoadedPreview::Error(content) => self.set_diff(content),
            LoadedPreview::Image(image) => self.set_image(image),
        }
        if let Some(pending) = self.pending_hunk_selection.take() {
            let count = hunk_count(&self.diff);
            self.hunk_selection = (count > 0).then(|| pending.index.min(count - 1));
            self.hunk_pin_pending = self.hunk_selection.is_some();
        }
        true
    }

    fn rebuild_explorer_rows(&mut self, repo: Option<&RepositoryData>) {
        self.sync_repository_caches(repo);
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

    fn sync_repository_caches(&mut self, repo: Option<&RepositoryData>) {
        let files_fingerprint = repo.map(|repo| repo.files_fingerprint);
        if self.file_tree_fingerprint != files_fingerprint {
            let tree = repo.map(|repo| FileTree::new(&repo.files, &repo.directories));
            let previous = std::mem::replace(&mut self.file_tree, tree);
            if let Some(previous) = previous {
                crate::diagnostics::drop_in_background("file-tree", previous);
            }
            self.file_tree_fingerprint = files_fingerprint;
        }
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

    pub(crate) fn set_diff(&mut self, content: String) {
        self.diff = content;
        self.preview_image = None;
        self.preview_content_generation = self.preview_content_generation.wrapping_add(1);
        self.preview_presentation.clear();
    }

    fn set_image(&mut self, image: Arc<DynamicImage>) {
        self.diff.clear();
        self.preview_image = Some(image);
        self.diff_scroll = 0;
        self.markdown_rendered = false;
        self.preview_content_generation = self.preview_content_generation.wrapping_add(1);
        self.preview_presentation.clear();
    }

    fn select_initial_rows(&mut self, repo: Option<&RepositoryData>) {
        self.worktree_state
            .select(repo.and_then(|repo| self.first_change_row(repo)));
        self.history_state.select(None);
        self.explorer_state.select(self.initial_explorer_row());
    }

    fn select_explorer_directory(&mut self, path: &RepoPath) {
        let row = self
            .explorer_rows()
            .iter()
            .position(|row| row.directory_path.as_ref() == Some(path));
        self.explorer_state.select(row);
    }

    fn row_for_explorer_file(&self, file_index: usize) -> Option<usize> {
        self.explorer_rows()
            .iter()
            .position(|row| row.file_index == Some(file_index))
    }

    fn first_explorer_file_row(&self) -> Option<usize> {
        self.explorer_rows()
            .iter()
            .position(|row| row.file_index.is_some())
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

    fn preview_selection(&self) -> (LeftPane, bool, Option<usize>) {
        let selected = if self.pane == LeftPane::Files {
            self.explorer_state.selected()
        } else if self.history_focused {
            self.history_state.selected()
        } else {
            self.worktree_state.selected()
        };
        (self.pane, self.history_focused, selected)
    }
}

fn hunk_count(diff: &str) -> usize {
    diff.lines().filter(|line| line.starts_with("@@")).count()
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
mod tests {
    use std::path::PathBuf;

    use crate::git::{Change, RepositoryKind};

    use super::*;

    fn repository_data() -> RepositoryData {
        RepositoryData {
            root: PathBuf::new(),
            common_dir: None,
            kind: RepositoryKind::Git,
            branch: "main".to_owned(),
            branches: Vec::new(),
            github_remote: false,
            worktree_signature: None,
            changes: vec![Change {
                path: "src/main.rs".into(),
                original_path: None,
                code: 'M',
                staged: false,
                additions: 0,
                deletions: 0,
            }],
            files: vec![
                "src/app/mod.rs".into(),
                "src/main.rs".into(),
                "README.md".into(),
            ],
            directories: Vec::new(),
            history: Vec::new(),
            commits: Vec::new(),
            files_fingerprint: 1,
            inventory_truncated: false,
            changes_fingerprint: 1,
            change_counts: (0, 1),
            graph_width: 0,
            graph_truncated: false,
        }
    }

    #[test]
    fn starts_files_collapsed_but_keeps_worktree_expanded() {
        let repo = repository_data();

        let mut state = ChangesState::new(Some(&repo));
        assert!(state.collapsed_directories.is_empty());
        assert!(state.expanded_explorer_directories.is_empty());
        assert_eq!(
            state
                .explorer_rows()
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["src", "README.md"]
        );
        assert_eq!(state.explorer_state.selected(), Some(1));

        state.explorer_state.select(Some(0));
        state.expand_or_descend_explorer(Some(&repo));
        assert_eq!(
            state
                .explorer_rows()
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            ["src", "app", "main.rs", "README.md"]
        );
        assert_eq!(state.explorer_rows()[1].directory_expanded, Some(false));
    }

    #[test]
    fn boundary_navigation_keeps_the_current_preview() {
        let repo = repository_data();
        let mut state = ChangesState::new(Some(&repo));
        state.pane = LeftPane::Files;
        state.explorer_state.select(Some(0));

        state.select_first(Some(&repo), 10, 10);
        let first_generation = state.preview_content_generation;
        state.select_first(Some(&repo), 10, 10);
        state.move_selection(Some(&repo), -1, 10, 10);
        assert_eq!(state.preview_content_generation, first_generation);

        state.select_last(Some(&repo), 10, 10);
        let last_generation = state.preview_content_generation;
        state.select_last(Some(&repo), 10, 10);
        state.move_selection(Some(&repo), 1, 10, 10);
        assert_eq!(state.preview_content_generation, last_generation);
    }

    #[test]
    fn owns_semantic_worktree_target_transitions() {
        let repo = repository_data();
        let mut state = ChangesState::new(Some(&repo));
        let file_row = state
            .worktree_rows(&repo)
            .iter()
            .position(|row| row.change_index.is_some())
            .unwrap();

        assert_eq!(
            state.activate_target(state.worktree_row_target(file_row), &repo),
            Some(ChangesEffect::WorktreeFileSelected {
                path: "src/main.rs".into(),
                staged: false,
            })
        );
        assert_eq!(state.worktree_state.selected(), Some(file_row));
        assert_eq!(
            state.stage_target(state.worktree_row_target(file_row), &repo),
            Some(ChangesEffect::ToggleSelectedStage)
        );

        let stale_file_target = state.worktree_row_target(file_row);
        let directory_row = state
            .worktree_rows(&repo)
            .iter()
            .position(|row| row.directory_path.is_some())
            .unwrap();
        assert_eq!(
            state.activate_target(state.worktree_row_target(directory_row), &repo),
            Some(ChangesEffect::WorktreeDirectoryActivated)
        );
        assert_eq!(state.activate_target(stale_file_target, &repo), None);

        state.set_diff("@@ -1 +1 @@\n-old\n+new\n".to_owned());
        let stale_hunk_target = state.hunk_action_target(0);
        state.set_diff("Loading preview…".to_owned());
        assert_eq!(state.activate_target(stale_hunk_target, &repo), None);

        assert_eq!(
            state.activate_target(ChangesHitTarget::FilesTab, &repo),
            Some(ChangesEffect::PaneActivated)
        );
        assert_eq!(state.pane, LeftPane::Files);
        assert_eq!(
            state.activate_target(ChangesHitTarget::WorktreeTab, &repo),
            Some(ChangesEffect::PaneActivated)
        );
        assert_eq!(state.pane, LeftPane::Worktree);
    }

    #[test]
    fn remembers_independent_markdown_source_and_preview_scrolls() {
        let mut state = ChangesState::new(None);
        state.diff_scroll = 80;

        state.toggle_markdown_rendered();
        assert!(state.markdown_rendered);
        assert_eq!(state.diff_scroll, 80);

        state.diff_scroll = 12;
        state.toggle_markdown_rendered();
        assert!(!state.markdown_rendered);
        assert_eq!(state.diff_scroll, 80);
        state.toggle_markdown_rendered();
        assert_eq!(state.diff_scroll, 12);

        state.refresh_diff(None);
        state.diff_scroll = 5;
        state.toggle_markdown_rendered();
        assert_eq!(state.diff_scroll, 5);
    }
}
