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
        {
            if !repository.is_local() {
                current.stats = Some(git::change_line_counts(&repository.changes));
                current.branch = Some(repository.branch.clone());
            }
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
        let current_stats = repository
            .details_ready
            .then(|| git::change_line_counts(&repository.changes));
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
                            stats: if same_path(&worktree.path, &current) {
                                current_stats
                            } else {
                                None
                            },
                            worktree,
                        })
                        .collect(),
                    selected,
                );
                self.header_picker.start_change_details();
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
        let items = repository
            .branches
            .iter()
            .filter(|branch| !branch.current)
            .cloned()
            .map(HeaderPickerItem::DiffTarget)
            .collect::<Vec<_>>();
        let selected = items
            .iter()
            .position(|item| matches!(item, HeaderPickerItem::DiffTarget(branch) if branch.default))
            .unwrap_or(0);
        if items.is_empty() {
            self.header_picker.open_message(
                HeaderPickerKind::DiffTargets,
                "No target branches".to_owned(),
            );
        } else {
            self.header_picker
                .open(HeaderPickerKind::DiffTargets, items, selected);
        }
    }

    pub(crate) fn start_header_agent(&mut self) {
        if self.mode != Mode::Normal || self.session.open_running() {
            return;
        }
        self.header_picker.close();
        let Some(repository) = self.git_repository() else {
            self.notice = Some("Agents require a Git repository".to_owned());
            return;
        };
        if !repository.details_ready {
            self.notice = Some("Repository details are still loading".to_owned());
            return;
        }
        let path = repository.root.clone();
        let branch = repository.branch.clone();
        if let Err(error) = self.herdr_prompt.prepare_agent(path, branch) {
            self.notice = Some(error);
        } else {
            self.notice = Some("Loading active Herdr tab layout".to_owned());
        }
    }

    pub(crate) fn handle_header_picker(&mut self, key: KeyEvent) {
        if self.header_picker.naming_branch() {
            self.handle_new_branch_name(key);
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
        match key.code {
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
            KeyCode::Enter => self.activate_header_picker(self.header_picker.selected),
            KeyCode::Backspace => self.edit_header_picker_query(TextInput::backspace),
            KeyCode::Delete => self.edit_header_picker_query(TextInput::delete),
            KeyCode::Left => self.header_picker.query.move_left(),
            KeyCode::Right => self.header_picker.query.move_right(),
            KeyCode::Home => self.header_picker.query.move_home(),
            KeyCode::End => self.header_picker.query.move_end(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.query.select_all();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.query.move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_header_picker_query(TextInput::clear);
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_header_picker_query(TextInput::delete_word);
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.edit_header_picker_query(|input| input.insert_char(character));
            }
            _ => {}
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
        if self.header_picker.worktree_creation_running() {
            self.header_picker.message = Some("Wait for the current worktree creation".to_owned());
            return;
        }
        let Some(repository) = self.git_repository() else {
            self.header_picker.message = Some("Not a Git repository".to_owned());
            return;
        };
        let base = repository.branch.clone();
        self.header_picker.begin_worktree_creation(&base);
    }

    pub(crate) fn handle_worktree_creation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.open_header_worktrees(),
            KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
                let field = match self.header_picker.worktree_field {
                    WorktreePickerField::Name => WorktreePickerField::Base,
                    WorktreePickerField::Base => WorktreePickerField::Name,
                };
                self.header_picker.set_worktree_field(field);
            }
            KeyCode::Enter if self.header_picker.worktree_field == WorktreePickerField::Name => {
                self.header_picker
                    .set_worktree_field(WorktreePickerField::Base);
            }
            KeyCode::Enter => {
                let name = self.header_picker.worktree_name.text().trim();
                let base = self.header_picker.worktree_base.text().trim();
                if name.is_empty() {
                    self.header_picker.message = Some("Worktree name is required".to_owned());
                    self.header_picker
                        .set_worktree_field(WorktreePickerField::Name);
                    return;
                }
                if base.is_empty() {
                    self.header_picker.message = Some("Starting branch is required".to_owned());
                    return;
                }
                if self.header_picker.worktree_creation_running() {
                    self.header_picker.message =
                        Some("Wait for the current worktree creation".to_owned());
                    return;
                }
                let Some(cwd) = self
                    .git_repository()
                    .map(|repository| repository.root.clone())
                else {
                    self.header_picker.message = Some("Not a Git repository".to_owned());
                    return;
                };
                let name = name.to_owned();
                let base = base.to_owned();
                if self
                    .header_picker
                    .start_worktree_creation(cwd, name.clone(), base)
                {
                    self.header_picker.close();
                    self.notice = Some(format!("Creating worktree {name}…"));
                }
            }
            KeyCode::Backspace => self.header_picker.worktree_input_mut().backspace(),
            KeyCode::Delete => self.header_picker.worktree_input_mut().delete(),
            KeyCode::Left => self.header_picker.worktree_input_mut().move_left(),
            KeyCode::Right => self.header_picker.worktree_input_mut().move_right(),
            KeyCode::Home => self.header_picker.worktree_input_mut().move_home(),
            KeyCode::End => self.header_picker.worktree_input_mut().move_end(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_input_mut().select_all();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_input_mut().move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_input_mut().clear();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.worktree_input_mut().delete_word();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.header_picker
                    .worktree_input_mut()
                    .insert_char(character);
            }
            _ => {}
        }
        if !matches!(key.code, KeyCode::Enter) {
            self.header_picker.message = None;
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
            KeyCode::Backspace => self.header_picker.clone_input_mut().backspace(),
            KeyCode::Delete => self.header_picker.clone_input_mut().delete(),
            KeyCode::Left => self.header_picker.clone_input_mut().move_left(),
            KeyCode::Right => self.header_picker.clone_input_mut().move_right(),
            KeyCode::Home => self.header_picker.clone_input_mut().move_home(),
            KeyCode::End => self.header_picker.clone_input_mut().move_end(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.clone_input_mut().select_all();
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
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.header_picker.clone_input_mut().insert_char(character);
            }
            _ => {}
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
            KeyCode::Backspace => self.header_picker.branch_name.backspace(),
            KeyCode::Delete => self.header_picker.branch_name.delete(),
            KeyCode::Left => self.header_picker.branch_name.move_left(),
            KeyCode::Right => self.header_picker.branch_name.move_right(),
            KeyCode::Home => self.header_picker.branch_name.move_home(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.select_all();
            }
            KeyCode::End => self.header_picker.branch_name.move_end(),
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.move_end();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.clear();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.header_picker.branch_name.delete_word();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.header_picker.branch_name.insert_char(character);
            }
            _ => {}
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
        if !self.start_repository_open(path, true)
            && let Some(error) = self.workspace_explorer.error.clone()
        {
            self.notice = Some(error);
        }
    }
}
