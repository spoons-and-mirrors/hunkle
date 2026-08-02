use super::super::*;

impl App {
    pub(crate) fn open_browser_branch(&mut self, oid: &str) {
        let Some((author, actual_index)) = self.repository().and_then(|repo| {
            repo.commits
                .iter()
                .position(|commit| commit.oid.starts_with(oid))
                .map(|index| (repo.commits[index].author.clone(), index))
        }) else {
            self.mode = Mode::Normal;
            self.notice = Some("Branch tip is outside the loaded graph".to_owned());
            return;
        };
        self.author_filter.ensure_enabled(&author);
        let Some(index) = self
            .visible_graph_indices()
            .iter()
            .position(|index| *index == actual_index)
        else {
            return;
        };
        self.graph_state.select(Some(index));
        *self.graph_state.offset_mut() = index.saturating_sub(5);
        self.graph_scroll_to_selection = false;
        self.show_graph();
        self.mode = Mode::Normal;
    }

    pub(crate) fn apply_repository_browser_effect_option(
        &mut self,
        effect: Option<RepositoryBrowserEffect>,
    ) {
        if let Some(effect) = effect {
            self.apply_repository_browser_effect(effect);
        }
    }

    pub(crate) fn apply_repository_browser_effect(&mut self, effect: RepositoryBrowserEffect) {
        match effect {
            RepositoryBrowserEffect::Close => self.mode = Mode::Normal,
            RepositoryBrowserEffect::OpenBranch(oid) => self.open_browser_branch(&oid),
            RepositoryBrowserEffect::CheckoutBranch { branch, remote } => {
                if self.session.start_branch_checkout(branch.clone(), remote) {
                    self.changes.clear_branch_comparison();
                    self.mode = Mode::Normal;
                    self.notice = Some(format!("Checking out {branch}…"));
                } else {
                    self.notice = Some("Another repository operation is running".to_owned());
                }
            }
            RepositoryBrowserEffect::DeleteBranch {
                branch,
                remote,
                force,
            } => {
                if self.session.start_branch_delete(branch, remote, force) {
                    self.notice = Some(if force {
                        "Force deleting branch…".to_owned()
                    } else {
                        "Deleting branch…".to_owned()
                    });
                } else {
                    self.notice = Some("Another repository operation is running".to_owned());
                }
            }
            RepositoryBrowserEffect::Notice(notice) => self.notice = Some(notice),
        }
    }

    pub(crate) fn open_repository_browser(&mut self) {
        if self.mode == Mode::Explorer && self.explorer_tab == ExplorerTab::Branches {
            return;
        }
        let Some(repo) = self.git_repository() else {
            self.require_git_repository();
            return;
        };
        if !repo.details_ready {
            self.notice = Some("Repository details are still loading".to_owned());
            return;
        }
        let root = repo.root.clone();
        let branches = repo.branches.clone();
        let prefetch = repo.github_remote;
        self.repository_browser.open(&root, &branches, prefetch);
        self.explorer_tab = ExplorerTab::Branches;
        self.mode = Mode::Explorer;
    }

    pub(crate) fn open_header_repositories(&mut self) {
        let details = self.repository_picker_details();
        let items = details
            .iter()
            .map(|detail| HeaderPickerItem::Repository {
                path: detail.root.clone(),
                common_dir: detail.common_dir.clone(),
                label: detail.label.clone(),
                stats: detail.stats,
                branch: detail.branch.clone(),
            })
            .collect::<Vec<_>>();
        let selected = self
            .git_repository()
            .and_then(|repository| repository.common_dir.as_deref())
            .and_then(|current| {
                items.iter().position(|item| {
                    matches!(item, HeaderPickerItem::Repository { common_dir, .. } if common_dir == current)
                })
            })
            .unwrap_or(0);
        if items.is_empty() {
            self.header_picker.open_message(
                HeaderPickerKind::Repositories,
                "No recent repositories".to_owned(),
            );
        } else {
            self.header_picker
                .open(HeaderPickerKind::Repositories, items, selected);
        }
    }

    pub(crate) fn repository_picker_details(&self) -> Vec<RepositoryPickerItem> {
        let mut details = self.linked_worktrees.recent_repository_picker_items();
        let Some(repository) = self
            .git_repository()
            .filter(|repository| repository.details_ready)
        else {
            return details;
        };
        let Some(common_dir) = repository.common_dir.as_deref() else {
            return details;
        };
        if let Some(current) = details
            .iter_mut()
            .find(|detail| detail.common_dir == common_dir)
        {
            current.stats = Some(git::change_line_counts(&repository.changes));
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

    pub(crate) fn open_header_agent_destinations(&mut self) {
        if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            self.header_picker.open_message(
                HeaderPickerKind::AgentDestinations,
                "Agents can only be started inside Herdr".to_owned(),
            );
            return;
        }

        let current = self.repository().map(|repository| repository.root.clone());
        let snapshot = self.linked_worktrees.snapshot();
        let mut items = Vec::new();
        for repository in snapshot
            .repositories
            .iter()
            .filter(|repository| repository.error.is_none())
        {
            for worktree in repository
                .worktrees
                .iter()
                .filter(|worktree| !worktree.is_bare)
            {
                let branch = worktree
                    .branch
                    .as_deref()
                    .map(|branch| {
                        branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(branch)
                            .to_owned()
                    })
                    .unwrap_or_else(|| "detached HEAD".to_owned());
                items.push(HeaderPickerItem::AgentDestination {
                    path: worktree.path.clone(),
                    repository: repository.label.clone(),
                    branch,
                    kind: if worktree.is_main {
                        AgentDestinationKind::Repository
                    } else {
                        AgentDestinationKind::Worktree
                    },
                });
            }
        }
        let selected = current
            .as_deref()
            .and_then(|current| {
                items.iter().position(|item| {
                    matches!(item, HeaderPickerItem::AgentDestination { path, .. } if same_path(path, current))
                })
            })
            .unwrap_or(0);
        if items.is_empty() {
            self.header_picker.open_message(
                HeaderPickerKind::AgentDestinations,
                if snapshot.loading {
                    "Agent destinations are still loading".to_owned()
                } else {
                    "No agent destinations".to_owned()
                },
            );
        } else {
            self.header_picker
                .open(HeaderPickerKind::AgentDestinations, items, selected);
        }
    }

    pub(crate) fn handle_header_picker(&mut self, key: KeyEvent) {
        if self.header_picker.naming_branch() {
            self.handle_new_branch_name(key);
            return;
        }
        match key.code {
            KeyCode::Esc if self.header_picker.branch_step == BranchPickerStep::Base => {
                self.open_header_branches();
            }
            KeyCode::Esc => self.header_picker.close(),
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

    pub(crate) fn prefetch_repository_browser(&mut self) {
        let root = self
            .git_repository()
            .filter(|repo| repo.github_remote)
            .map(|repo| repo.root.clone());
        if let Some(root) = root {
            self.repository_browser.prefetch(&root);
        }
    }

    pub(crate) fn handle_repository_browser(&mut self, key: KeyEvent) {
        let key = if self.repository_browser.branch_delete_open() {
            key
        } else {
            self.settings.shortcuts.remap_repository_browser(key)
        };
        let effect = self.repository_browser.handle_key(key);
        self.apply_repository_browser_effect_option(effect);
    }
}
