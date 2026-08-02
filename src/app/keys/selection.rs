use super::super::*;

impl App {
    fn magic_commit_blocks_staging(&mut self) -> bool {
        if self.magic_commit_running_for_active_repository() {
            self.notice = Some("Magic Commit is using the Git index".to_owned());
            true
        } else {
            false
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        match self.visible_view() {
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
            View::RepositorySearch => self.file_search.move_selection(delta),
        }
    }

    pub(crate) fn select_first(&mut self) {
        match self.visible_view() {
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
            View::RepositorySearch => self.file_search.select_first(),
        }
    }

    pub(crate) fn select_last(&mut self) {
        match self.visible_view() {
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
            View::RepositorySearch => self.file_search.select_last(),
        }
    }

    pub(crate) fn toggle_stage(&mut self) {
        if self.magic_commit_blocks_staging() {
            return;
        }
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

    pub(crate) fn stage_hunk(&mut self, index: usize, preserve_selection: bool) {
        if self.magic_commit_blocks_staging() {
            return;
        }
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

    pub(crate) fn stage_all(&mut self) {
        if self.magic_commit_blocks_staging() {
            return;
        }
        if self.require_git_repository() {
            let _ = self.session.start_mutation(Mutation::StageAll);
        }
    }

    pub(crate) fn toggle_all_staging(&mut self) {
        let all_staged = self.repository().is_some_and(|repo| {
            !repo.changes.is_empty() && repo.changes.iter().all(|change| change.staged)
        });
        if all_staged {
            self.unstage_all();
        } else {
            self.stage_all();
        }
    }

    pub(crate) fn unstage_all(&mut self) {
        if self.magic_commit_blocks_staging() {
            return;
        }
        if self.require_git_repository() {
            let _ = self.session.start_mutation(Mutation::UnstageAll);
        }
    }
}
