use super::super::*;

impl App {
    pub(crate) fn open_header_repositories(&mut self) {
        let details = self.repository_picker_details();
        let items = details
            .iter()
            .map(|detail| HeaderPickerItem::Repository {
                path: detail.root.clone(),
                label: detail.label.clone(),
                stats: detail.stats,
                branch: detail.branch.clone(),
            })
            .collect::<Vec<_>>();
        let selected = self
            .repository()
            .map(|repository| repository.root.as_path())
            .and_then(|current| {
                items.iter().position(|item| {
                    matches!(item, HeaderPickerItem::Repository { path, .. } if same_path(path, current))
                })
            })
            .unwrap_or(0);
        if items.is_empty() {
            self.header_picker
                .open(HeaderPickerKind::Repositories, items, selected);
            self.header_picker.message = Some("No recent repositories".to_owned());
        } else {
            self.header_picker
                .open(HeaderPickerKind::Repositories, items, selected);
        }
        self.linked_worktrees.request_recent_stats();
    }

    pub(crate) fn repository_picker_details(&self) -> Vec<RepositoryPickerItem> {
        let mut details = self.linked_worktrees.recent_repository_picker_items();
        let Some(repository) = self
            .repository()
            .filter(|repository| repository.details_ready)
        else {
            return details;
        };
        if let Some(current) = details
            .iter_mut()
            .find(|detail| same_path(&detail.root, &repository.root))
            && !repository.is_local()
        {
            current.branch = Some(repository.branch.clone());
        }
        details
    }

    pub(crate) fn open_header_worktrees(&mut self) {
        let Some(repository) = self.git_repository() else {
            self.header_picker.open_message(
                HeaderPickerKind::Worktrees,
                "Not a Git repository".to_owned(),
            );
            return;
        };
        let Some(common_dir) = repository.common_dir.as_deref() else {
            self.header_picker.open_message(
                HeaderPickerKind::Worktrees,
                "Worktrees are unavailable".to_owned(),
            );
            return;
        };
        let current = repository.root.clone();
        match self.linked_worktrees.repository(common_dir) {
            Some(repository) if repository.error.is_none() => {
                let worktrees = repository
                    .worktrees
                    .iter()
                    .filter(|worktree| !worktree.is_bare)
                    .cloned()
                    .collect::<Vec<_>>();
                let selected = worktrees
                    .iter()
                    .position(|worktree| same_path(&worktree.path, &current))
                    .unwrap_or(0);
                self.header_picker.open(
                    HeaderPickerKind::Worktrees,
                    worktrees
                        .into_iter()
                        .map(|worktree| HeaderPickerItem::Worktree {
                            stats: self.linked_worktrees.change_stats(&worktree.path),
                            worktree,
                        })
                        .collect(),
                    selected,
                );
                let roots = self.header_picker.change_stats_roots();
                self.linked_worktrees.request_stats(roots);
            }
            Some(repository) => self.header_picker.open_message(
                HeaderPickerKind::Worktrees,
                repository.error.clone().unwrap_or_default(),
            ),
            None => self.header_picker.open_message(
                HeaderPickerKind::Worktrees,
                "Worktrees are still loading".to_owned(),
            ),
        }
    }

    pub(crate) fn open_header_branches(&mut self) {
        let Some(repository) = self.git_repository() else {
            self.header_picker.open_message(
                HeaderPickerKind::Branches,
                "Not a Git repository".to_owned(),
            );
            return;
        };
        if !repository.details_ready {
            self.header_picker.open_message(
                HeaderPickerKind::Branches,
                "Repository details are still loading".to_owned(),
            );
            return;
        }
        let selected = repository
            .branches
            .iter()
            .position(|branch| branch.current)
            .unwrap_or(0);
        let items = repository
            .branches
            .iter()
            .cloned()
            .map(HeaderPickerItem::Branch)
            .collect();
        self.header_picker
            .open(HeaderPickerKind::Branches, items, selected);
    }

    pub(crate) fn open_header_branch_bases(&mut self) {
        let Some(repository) = self.git_repository() else {
            self.header_picker.close();
            return;
        };
        let selected = repository
            .branches
            .iter()
            .position(|branch| branch.current)
            .unwrap_or(0);
        let items = repository
            .branches
            .iter()
            .cloned()
            .map(HeaderPickerItem::BranchBase)
            .collect();
        self.header_picker.open_branch_bases(items, selected);
    }

    pub(crate) fn open_header_diff_targets(&mut self) {
        if !self.session.can_start_mutation() {
            self.header_picker.open_message(
                HeaderPickerKind::DiffTargets,
                "Wait for the current Git operation".to_owned(),
            );
            return;
        }
        let Some(repository) = self.git_repository() else {
            self.header_picker.open_message(
                HeaderPickerKind::DiffTargets,
                "Not a Git repository".to_owned(),
            );
            return;
        };
        if !repository.details_ready {
            self.header_picker.open_message(
                HeaderPickerKind::DiffTargets,
                "Repository details are still loading".to_owned(),
            );
            return;
        }
        let mut items = vec![HeaderPickerItem::DiffTarget {
            label: "HEAD~".to_owned(),
            revision: "HEAD~".to_owned(),
            detail: "parent".to_owned(),
            default: true,
        }];
        items.extend(
            repository
                .branches
                .iter()
                .filter(|branch| !branch.current)
                .map(|branch| HeaderPickerItem::DiffTarget {
                    label: branch.name.clone(),
                    revision: branch.revision(),
                    detail: if branch.remote { "remote" } else { "local" }.to_owned(),
                    default: branch.default,
                }),
        );
        let selected = items
            .iter()
            .position(|item| matches!(item, HeaderPickerItem::DiffTarget { default: true, .. }))
            .unwrap_or(0);
        self.header_picker
            .open(HeaderPickerKind::DiffTargets, items, selected);
    }

    pub(crate) fn open_header_issues(&mut self) {
        let Some(root) = self
            .git_repository()
            .map(|repository| repository.root.clone())
        else {
            self.header_picker
                .open_message(HeaderPickerKind::Issues, "Not a Git repository".to_owned());
            return;
        };
        self.issues.request(&root);
        self.header_picker
            .open(HeaderPickerKind::Issues, Vec::new(), 0);
        self.refresh_header_issue_items();
    }

    pub(crate) fn toggle_header_issue_scope(&mut self) {
        self.issues.toggle_scope();
        let Some(root) = self
            .git_repository()
            .map(|repository| repository.root.clone())
        else {
            return;
        };
        self.issues.request(&root);
        self.refresh_header_issue_items();
    }

    pub(crate) fn refresh_header_issue_items(&mut self) {
        let selected_number = self
            .header_picker
            .items
            .get(self.header_picker.selected)
            .and_then(issue_item_number);
        let top_number = self
            .header_picker
            .items
            .get(self.header_picker.visible_start())
            .and_then(issue_item_number);
        let items = self
            .issues
            .issues()
            .unwrap_or_default()
            .iter()
            .map(|issue| HeaderPickerItem::Issue {
                number: issue.number,
                title: issue.title.clone(),
                pull_request: issue.pull_request,
                status: issue.status_label().to_owned(),
                author: issue.author.clone(),
                labels: issue.labels.clone(),
                changed_files: issue.changed_files,
                additions: issue.additions,
                deletions: issue.deletions,
            })
            .collect::<Vec<_>>();
        let message = self.issues.error().map(str::to_owned).or_else(|| {
            if self.issues.loading() && items.is_empty() {
                Some("Loading GitHub issues…".to_owned())
            } else if items.is_empty() {
                Some(match self.issues.scope() {
                    IssueScope::Open => "No open issues or pull requests".to_owned(),
                    IssueScope::Closed => "No closed issues or pull requests".to_owned(),
                })
            } else {
                None
            }
        });
        let query = self.header_picker.query.text().to_owned();
        self.header_picker.open(HeaderPickerKind::Issues, items, 0);
        if !query.is_empty() {
            self.header_picker.query.insert(&query);
            self.header_picker.apply_filter();
        }
        if let Some(number) = selected_number
            && let Some(selected) = self
                .header_picker
                .items
                .iter()
                .position(|item| issue_item_number(item) == Some(number))
        {
            self.header_picker.selected = selected;
        }
        if let Some(number) = top_number
            && let Some(top) = self
                .header_picker
                .items
                .iter()
                .position(|item| issue_item_number(item) == Some(number))
        {
            self.header_picker.scroll_to(top);
        }
        self.header_picker.message = message;
    }

    pub(crate) fn start_header_agent(&mut self) {
        if self.mode != Mode::Normal || self.session.open_running() {
            return;
        }
        if !self.herdr_available() {
            return;
        }
        if self.herdr.agent_stash_running() || self.pending_agent_preview_pane.is_some() {
            self.notice = Some("Another agent operation is still in progress".to_owned());
            return;
        }
        self.header_picker.close();
        let Some(path) = self.agent_destination_for_start() else {
            self.notice = Some("Open a workspace first".to_owned());
            return;
        };
        let background_workspace_id = self.herdr.background_workspace_id().map(str::to_owned);
        if let Err(error) = self
            .herdr_prompt
            .prepare_agent(path, background_workspace_id)
        {
            self.notice = Some(error);
        } else if self.herdr.is_background_attached() {
            self.notice = Some("Starting agent in a new Herdr tab".to_owned());
        } else {
            self.notice = Some("Loading active Herdr tab layout".to_owned());
        }
    }

    pub(crate) fn agent_destination_for_start(&self) -> Option<PathBuf> {
        Some(self.repository()?.root.clone())
    }

    pub(crate) fn handle_header_picker(&mut self, key: KeyEvent) {
        if self.header_picker.naming_branch() {
            self.handle_new_branch_name(key);
            return;
        }
        if self.header_picker.deleting_branch() {
            self.handle_branch_deletion(key);
            return;
        }
        if self.header_picker.cloning_repository() {
            self.handle_repository_clone(key);
            return;
        }
        if self.header_picker.creating_worktree() {
            self.handle_worktree_creation(key);
            return;
        }
        if self.header_picker.deleting_worktree() {
            self.handle_worktree_deletion(key);
            return;
        }
        match key.code {
            KeyCode::Tab if self.header_picker.kind == Some(HeaderPickerKind::Issues) => {
                self.toggle_header_issue_scope();
            }
            KeyCode::Esc if self.header_picker.selecting_worktree_base() => {
                self.header_picker.return_to_worktree_name();
            }
            KeyCode::Esc if self.header_picker.branch_step == BranchPickerStep::Base => {
                self.open_header_branches();
            }
            KeyCode::Esc => self.header_picker.close(),
            KeyCode::Char('n')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.header_picker.kind == Some(HeaderPickerKind::Repositories) =>
            {
                self.begin_repository_clone();
            }
            KeyCode::Char('n')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.header_picker.kind == Some(HeaderPickerKind::Worktrees) =>
            {
                self.begin_header_worktree_creation();
            }
            KeyCode::Char('n')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.header_picker.kind == Some(HeaderPickerKind::Branches)
                    && self.header_picker.branch_step == BranchPickerStep::Branches =>
            {
                self.begin_header_branch_creation();
            }
            KeyCode::Up => {
                self.hovered_hit_target = None;
                self.header_picker.move_selection(-1);
            }
            KeyCode::Down => {
                self.hovered_hit_target = None;
                self.header_picker.move_selection(1);
            }
            KeyCode::PageUp => {
                self.hovered_hit_target = None;
                self.header_picker.move_selection_page(-1);
            }
            KeyCode::PageDown => {
                self.hovered_hit_target = None;
                self.header_picker.move_selection_page(1);
            }
            KeyCode::Enter => self.activate_header_picker(self.header_picker.selected),
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.query.move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_header_picker_query(TextInput::clear);
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_header_picker_query(TextInput::delete_word);
            }
            _ => {
                if self.header_picker.query.handle_edit_key(key) == EditOutcome::Edited {
                    self.header_picker.message = None;
                    self.header_picker.apply_filter();
                }
            }
        }
    }

    pub(crate) fn begin_repository_clone(&mut self) {
        if self.header_picker.clone_running() {
            self.header_picker.message = Some("A repository clone is already running".to_owned());
            return;
        }
        let mut directory = self
            .repository()
            .and_then(|repository| repository.root.parent())
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Path::to_path_buf,
            )
            .display()
            .to_string();
        if !directory.ends_with(std::path::MAIN_SEPARATOR) {
            directory.push(std::path::MAIN_SEPARATOR);
        }
        self.header_picker.begin_clone(Path::new(&directory));
    }

    pub(crate) fn begin_header_worktree_creation(&mut self) {
        if self.header_picker.worktree_creation_running()
            || self.header_picker.worktree_deletion_running()
        {
            self.header_picker.message = Some("Wait for the current worktree operation".to_owned());
            return;
        }
        if self.git_repository().is_none() {
            self.header_picker.message = Some("Not a Git repository".to_owned());
            return;
        }
        self.header_picker.begin_worktree_creation();
    }

    pub(crate) fn begin_header_worktree_deletion(&mut self, index: usize) {
        if self.header_picker.worktree_creation_running()
            || self.header_picker.worktree_deletion_running()
        {
            self.header_picker.message = Some("Wait for the current worktree operation".to_owned());
            return;
        }
        let Some(HeaderPickerItem::Worktree { worktree, .. }) = self.header_picker.items.get(index)
        else {
            return;
        };
        if worktree.is_main {
            self.header_picker.message = Some("The main worktree cannot be deleted".to_owned());
            return;
        }
        if self
            .repository()
            .is_some_and(|repository| same_path(&repository.root, &worktree.path))
        {
            self.header_picker.message = Some("The active worktree cannot be deleted".to_owned());
            return;
        }
        self.header_picker
            .begin_worktree_deletion(worktree.path.clone());
    }

    pub(crate) fn begin_header_branch_deletion(&mut self, index: usize) {
        if !self.session.can_start_mutation() {
            self.header_picker.message = Some("Wait for the current Git operation".to_owned());
            return;
        }
        let Some(HeaderPickerItem::Branch(branch)) = self.header_picker.items.get(index) else {
            return;
        };
        if branch.current {
            self.header_picker.message = Some("The current branch cannot be deleted".to_owned());
            return;
        }
        if branch.remote {
            self.header_picker.message = Some("Remote branches cannot be deleted here".to_owned());
            return;
        }
        self.header_picker
            .begin_branch_deletion(branch.name.clone());
    }

    pub(crate) fn handle_branch_deletion(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => self.open_header_branches(),
            KeyCode::Enter | KeyCode::Char('y') => self.confirm_header_branch_deletion(),
            _ => {}
        }
    }

    pub(crate) fn confirm_header_branch_deletion(&mut self) {
        if !self.session.can_start_mutation() {
            self.header_picker.message = Some("Wait for the current Git operation".to_owned());
            return;
        }
        let Some(branch) = self.header_picker.branch_delete.clone() else {
            self.open_header_branches();
            return;
        };
        if self.session.start_branch_delete(branch.clone()) {
            self.header_picker.close();
            self.notice = Some(format!("Deleting branch {branch}…"));
        }
    }

    pub(crate) fn handle_worktree_deletion(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => self.open_header_worktrees(),
            KeyCode::Enter | KeyCode::Char('y') => self.confirm_header_worktree_deletion(),
            _ => {}
        }
    }

    pub(crate) fn confirm_header_worktree_deletion(&mut self) {
        if self.header_picker.worktree_creation_running()
            || self.header_picker.worktree_deletion_running()
        {
            self.header_picker.message = Some("Wait for the current worktree operation".to_owned());
            return;
        }
        let Some(path) = self.header_picker.worktree_delete.clone() else {
            self.open_header_worktrees();
            return;
        };
        let Some(cwd) = self
            .git_repository()
            .map(|repository| repository.root.clone())
        else {
            self.header_picker.message = Some("Not a Git repository".to_owned());
            return;
        };
        if self
            .header_picker
            .start_worktree_deletion(cwd, path.clone())
        {
            self.header_picker.close();
            self.notice = Some(format!("Deleting worktree {}…", path.display()));
        }
    }

    pub(crate) fn handle_worktree_creation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.open_header_worktrees(),
            KeyCode::Enter => {
                let name = self.header_picker.worktree_name.text().trim();
                if name.is_empty() {
                    self.header_picker.message = Some("New branch name is required".to_owned());
                    return;
                }
                self.open_header_worktree_bases();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_name.move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_name.clear();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_name.delete_word();
            }
            _ => {
                self.header_picker.worktree_name.handle_edit_key(key);
            }
        }
        if !matches!(key.code, KeyCode::Enter) {
            self.header_picker.message = None;
        }
    }

    fn open_header_worktree_bases(&mut self) {
        let Some(repository) = self.git_repository() else {
            self.header_picker.message = Some("Not a Git repository".to_owned());
            return;
        };
        let selected = repository
            .branches
            .iter()
            .position(|branch| branch.current)
            .unwrap_or(0);
        let items = repository
            .branches
            .iter()
            .cloned()
            .map(HeaderPickerItem::BranchBase)
            .collect();
        self.header_picker.open_worktree_bases(items, selected);
    }

    pub(crate) fn create_header_worktree(&mut self, base: Branch) {
        if self.header_picker.worktree_creation_running() {
            self.header_picker.message = Some("Wait for the current worktree creation".to_owned());
            return;
        }
        let Some(cwd) = self
            .git_repository()
            .map(|repository| repository.root.clone())
        else {
            self.header_picker.message = Some("Not a Git repository".to_owned());
            return;
        };
        let name = self.header_picker.worktree_name.text().trim().to_owned();
        if self
            .header_picker
            .start_worktree_creation(cwd, name.clone(), base.revision())
        {
            self.header_picker.close();
            self.notice = Some(format!("Creating worktree {name}…"));
        }
    }

    pub(crate) fn handle_repository_clone(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.open_header_repositories(),
            KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
                let field = match self.header_picker.clone_field {
                    CloneField::Directory => CloneField::Url,
                    CloneField::Url => CloneField::Directory,
                };
                self.header_picker.set_clone_field(field);
            }
            KeyCode::Enter if self.header_picker.clone_field == CloneField::Directory => {
                self.header_picker.set_clone_field(CloneField::Url);
            }
            KeyCode::Enter => {
                let directory = self.header_picker.clone_directory.text().trim();
                let url = self.header_picker.clone_url.text().trim();
                if directory.is_empty() {
                    self.header_picker.message = Some("Directory is required".to_owned());
                    self.header_picker.set_clone_field(CloneField::Directory);
                    return;
                }
                if url.is_empty() {
                    self.header_picker.message = Some("Git URL is required".to_owned());
                    return;
                }
                let mut destination = if directory == "~" {
                    std::env::var_os("HOME").map_or_else(|| PathBuf::from(directory), PathBuf::from)
                } else if let Some(rest) = directory
                    .strip_prefix("~/")
                    .or_else(|| directory.strip_prefix("~\\"))
                {
                    std::env::var_os("HOME").map_or_else(
                        || PathBuf::from(directory),
                        |home| PathBuf::from(home).join(rest),
                    )
                } else {
                    PathBuf::from(directory)
                };
                if destination.is_relative() {
                    destination = std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(destination);
                }
                if self.header_picker.start_clone(destination, url.to_owned()) {
                    self.header_picker.close();
                    self.notice = Some("Cloning repository…".to_owned());
                }
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.clone_input_mut().move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.clone_input_mut().clear();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.clone_input_mut().delete_word();
            }
            _ => {
                self.header_picker.clone_input_mut().handle_edit_key(key);
            }
        }
        if !matches!(key.code, KeyCode::Enter) {
            self.header_picker.message = None;
        }
    }

    pub(crate) fn edit_header_picker_query(&mut self, edit: impl FnOnce(&mut TextInput)) {
        edit(&mut self.header_picker.query);
        self.header_picker.message = None;
        self.header_picker.apply_filter();
    }

    pub(crate) fn handle_new_branch_name(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.open_header_branch_bases(),
            KeyCode::Enter => {
                let name = self.header_picker.branch_name.text().trim().to_owned();
                if name.is_empty() {
                    self.header_picker.message = Some("Branch name is required".to_owned());
                    return;
                }
                let Some(base) = self
                    .header_picker
                    .branch_base
                    .as_ref()
                    .map(Branch::revision)
                else {
                    self.open_header_branch_bases();
                    return;
                };
                if self.session.start_branch_create(name.clone(), base) {
                    self.changes.clear_branch_comparison();
                    self.header_picker.close();
                    self.notice = Some(format!("Creating {name}…"));
                } else {
                    self.header_picker.message =
                        Some("Wait for the current Git operation".to_owned());
                }
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.clear();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.delete_word();
            }
            _ => {
                self.header_picker.branch_name.handle_edit_key(key);
            }
        }
        if !matches!(key.code, KeyCode::Enter) {
            self.header_picker.message = None;
        }
    }

    pub(crate) fn open_header_path(&mut self, path: PathBuf) {
        if !self.session.can_start_open() {
            self.notice = Some("Another workspace operation is still running".to_owned());
            return;
        }
        if self.start_repository_open(path.clone(), true) {
            self.herdr_prompt.update_agent_destination(path);
        } else if let Some(error) = self.workspace_explorer.error.clone() {
            self.notice = Some(error);
        }
    }
}

fn issue_item_number(item: &HeaderPickerItem) -> Option<u64> {
    match item {
        HeaderPickerItem::Issue { number, .. } => Some(*number),
        _ => None,
    }
}
