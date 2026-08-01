use super::super::*;

impl App {
    pub(crate) fn open_worktree_manager(&mut self) {
        if self.mode == Mode::Explorer && self.explorer_tab == ExplorerTab::Worktrees {
            return;
        }
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
        self.explorer_tab = ExplorerTab::Worktrees;
        self.mode = Mode::Explorer;
        if let Some(warning) = warning {
            self.notice = Some(warning);
        }
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

    pub(crate) fn worktree_removal_running(&self) -> bool {
        self.worktree_manager.operation_running()
            || self.workspace_panel.destructive_action_running()
    }
}
