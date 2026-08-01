use super::super::*;

impl App {
    pub(crate) fn open_author_filter(&mut self) {
        let Some(repo) = self
            .session
            .data()
            .filter(|repo| !repo.is_local() && repo.details_ready)
        else {
            if self.git_repository().is_some() {
                self.notice = Some("Repository details are still loading".to_owned());
            }
            return;
        };
        self.author_filter.open(&repo.root, &repo.commits);
        self.mode = Mode::AuthorFilter;
    }

    pub(crate) fn handle_author_filter(&mut self, key: KeyEvent) {
        let key = self.settings.shortcuts.remap_author_filter(key);
        match self.author_filter.handle_key(key) {
            Some(AuthorFilterEffect::Close) => self.mode = Mode::Normal,
            Some(AuthorFilterEffect::Changed) => self.reconcile_graph_selection(),
            None => {}
        }
    }

    pub(crate) fn open_workspace_file(&mut self, path: PathBuf) {
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

    pub(crate) fn open_repository(&mut self, path: PathBuf) {
        self.start_repository_open(path, false);
    }

    pub(crate) fn open_repository_with_fetch(&mut self, path: PathBuf) {
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

    pub(crate) fn queue_workspace_restore(&mut self, path: PathBuf) {
        self.pending_workspace_restore = Some(path);
        self.try_start_workspace_restore();
    }

    pub(crate) fn try_start_workspace_restore(&mut self) {
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

    pub(crate) fn start_repository_open(&mut self, path: PathBuf, fetch_if_stale: bool) -> bool {
        diagnostics::event(format!(
            "workspace open requested path={} fetch_if_stale={fetch_if_stale}",
            path.display()
        ));
        if self.worktree_removal_running() {
            self.notice = Some("Wait for the worktree removal to finish".to_owned());
            return false;
        }
        if self.file_editor.is_some() {
            self.notice = Some("Save or close the editor before opening a workspace".to_owned());
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
            self.header_picker.close();
            self.pending_reload = None;
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

    pub(crate) fn maybe_start_workspace_fetch(&mut self) {
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
}
