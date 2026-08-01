use super::super::*;

impl App {
    pub(crate) fn open_worktree_manager(&mut self) {
        if self.mode == Mode::Explorer && self.explorer_tab == ExplorerTab::Worktrees {
            return;
        }
        self.workspace_panel.refresh_worktree_inventory();
        if self
            .linked_worktrees
            .observe_herdr(self.workspace_panel.linked_worktree_observation())
        {
            self.linked_worktrees.refresh();
        }
        self.worktree_manager.open(self.linked_worktrees.snapshot());
        self.explorer_tab = ExplorerTab::Worktrees;
        self.mode = Mode::Explorer;
    }

    pub(crate) fn handle_worktree_manager(&mut self, key: KeyEvent) {
        let key = if self.worktree_manager.dialog_open() {
            key
        } else {
            self.settings.shortcuts.remap_worktrees(key)
        };
        let effect = self.worktree_manager.handle_key(key);
        self.apply_worktree_manager_effect_option(effect);
    }

    pub(crate) fn apply_worktree_manager_effect_option(
        &mut self,
        effect: Option<WorktreeManagerEffect>,
    ) {
        if let Some(effect) = effect {
            self.apply_worktree_manager_effect(effect);
        }
    }

    pub(crate) fn apply_worktree_manager_effect(&mut self, effect: WorktreeManagerEffect) {
        match effect {
            WorktreeManagerEffect::Close => {
                if self.repository().is_some() {
                    self.mode = Mode::Normal;
                } else {
                    self.explorer_tab = ExplorerTab::Explorer;
                }
            }
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
                self.linked_worktrees.refresh();
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
            WorktreeManagerEffect::Remove(path) => {
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
                match self.linked_worktrees.removal_plan(&path) {
                    Ok(LinkedWorktreeRemovalPlan::Native { common_dir, path }) => {
                        if !self.worktree_manager.start_remove(common_dir, path) {
                            self.notice = Some("A worktree removal is already running".to_owned());
                        }
                    }
                    Ok(LinkedWorktreeRemovalPlan::Herdr { workspace_id, path }) => {
                        self.workspace_panel.delete_worktree(&workspace_id, None);
                        self.notice = Some(format!("Removing worktree {}…", path.display()));
                        self.mode = Mode::Normal;
                    }
                    Err(reason) => self.notice = Some(reason),
                }
            }
            WorktreeManagerEffect::Notice(notice) => self.notice = Some(notice),
        }
    }

    pub(crate) fn worktree_removal_running(&self) -> bool {
        self.worktree_manager.operation_running()
            || self.workspace_panel.destructive_action_running()
    }
}
