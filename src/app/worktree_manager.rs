use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::{
    filesystem::same_path,
    git::{self, LinkedWorktree},
};

mod known_repositories;

use known_repositories::KnownRepositoryStore;

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorktreeManagerEffect {
    Close,
    Open(PathBuf),
    Refresh,
    CreateHerdr {
        cwd: PathBuf,
        path: PathBuf,
        branch: String,
        start_point: String,
    },
    RemoveNative {
        common_dir: PathBuf,
        path: PathBuf,
    },
    RemoveHerdr {
        workspace_id: String,
        path: PathBuf,
    },
    Notice(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeCreateField {
    Branch,
    Path,
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeCreateDialog {
    pub(crate) repository_label: String,
    pub(crate) start_label: String,
    pub(crate) branch: String,
    pub(crate) path: String,
    pub(crate) field: WorktreeCreateField,
    pub(crate) error: Option<String>,
    cwd: PathBuf,
    start_point: String,
    base_dir: PathBuf,
    path_automatic: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeRemoveDialog {
    pub(crate) common_dir: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    pub(crate) herdr_workspace_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeRepository {
    pub(crate) common_dir: PathBuf,
    pub(crate) label: String,
    pub(crate) group: Option<String>,
    pub(crate) worktrees: Vec<LinkedWorktree>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeCandidate {
    pub(crate) path: PathBuf,
    pub(crate) group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeManagerRow {
    Group(usize),
    Worktree { repository: usize, worktree: usize },
    Status(usize),
}

struct InventoryCompletion {
    generation: u64,
    repositories: Vec<WorktreeRepository>,
    discovered: Vec<PathBuf>,
    pruned: Vec<PathBuf>,
}

struct RemovalCompletion {
    path: PathBuf,
    result: Result<(), String>,
}

struct CreationCompletion {
    path: PathBuf,
    result: Result<(), String>,
}

enum Completion {
    Inventory(InventoryCompletion),
    Creation(CreationCompletion),
    Removal(RemovalCompletion),
}

#[derive(Default)]
pub(crate) struct WorktreeManagerPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
    pub(crate) open_path: Option<PathBuf>,
}

pub(crate) struct WorktreeManager {
    pub(crate) query: String,
    pub(crate) state: ListState,
    pub(crate) repositories: Vec<WorktreeRepository>,
    pub(crate) loading: bool,
    pub(crate) create_running: bool,
    pub(crate) create_dialog: Option<WorktreeCreateDialog>,
    pub(crate) remove_running: bool,
    pub(crate) remove_dialog: Option<WorktreeRemoveDialog>,
    current_path: Option<PathBuf>,
    candidates: Vec<WorktreeCandidate>,
    herdr_worktrees: Vec<(PathBuf, String)>,
    herdr_enabled: bool,
    herdr_verified: bool,
    store: KnownRepositoryStore,
    generation: u64,
    content_generation: u64,
    last_click: Option<(PathBuf, Instant)>,
    pending_create: Option<WorktreeCreateDialog>,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
}

impl WorktreeManager {
    pub(crate) fn new(store_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            query: String::new(),
            state: ListState::default(),
            repositories: Vec::new(),
            loading: false,
            create_running: false,
            create_dialog: None,
            remove_running: false,
            remove_dialog: None,
            current_path: None,
            candidates: Vec::new(),
            herdr_worktrees: Vec::new(),
            herdr_enabled: false,
            herdr_verified: true,
            store: KnownRepositoryStore::new(store_path),
            generation: 0,
            content_generation: 0,
            last_click: None,
            pending_create: None,
            sender,
            receiver,
        }
    }

    pub(crate) fn remember(&mut self, common_dir: &Path) -> Result<(), String> {
        self.store.extend_and_save(vec![common_dir.to_owned()])
    }

    pub(crate) fn open(
        &mut self,
        candidates: Vec<WorktreeCandidate>,
        herdr_worktrees: Vec<(PathBuf, String)>,
        current_path: Option<PathBuf>,
        herdr_enabled: bool,
        herdr_verified: bool,
    ) -> Option<String> {
        self.query.clear();
        self.create_dialog = None;
        self.remove_dialog = None;
        self.current_path = current_path;
        self.candidates = candidates;
        self.herdr_worktrees = herdr_worktrees;
        self.herdr_enabled = herdr_enabled;
        self.herdr_verified = herdr_verified;
        self.last_click = None;
        self.bump_content_generation();
        self.start_refresh();
        self.store.load_error.clone()
    }

    pub(crate) fn update_herdr_inventory(
        &mut self,
        candidates: Vec<WorktreeCandidate>,
        herdr_worktrees: Vec<(PathBuf, String)>,
        verified: bool,
    ) -> bool {
        let candidates_changed = self.candidates != candidates;
        self.candidates = candidates;
        self.herdr_worktrees = herdr_worktrees;
        self.herdr_verified = verified;
        candidates_changed
    }

    pub(crate) fn start_refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let known = self.store.repositories.clone();
        let candidates = self.candidates.clone();
        let sender = self.sender.clone();
        self.loading = true;
        thread::spawn(move || {
            let mut common_dirs = known;
            let mut seen = common_dirs.iter().cloned().collect::<HashSet<_>>();
            let mut candidate_ranks = HashMap::new();
            let mut discovered = Vec::new();
            for (rank, candidate) in candidates.into_iter().enumerate() {
                let Ok(common_dir) = git::common_git_dir(&candidate.path) else {
                    continue;
                };
                candidate_ranks
                    .entry(common_dir.clone())
                    .or_insert((rank, candidate.group));
                if seen.insert(common_dir.clone()) {
                    discovered.push(common_dir.clone());
                    common_dirs.push(common_dir);
                }
            }
            common_dirs.sort_by_cached_key(|path| {
                (
                    !candidate_ranks.contains_key(path),
                    candidate_ranks
                        .get(path)
                        .map_or(usize::MAX, |(rank, _)| *rank),
                    path.to_string_lossy().to_lowercase(),
                )
            });
            let mut pruned = Vec::new();
            let repositories = common_dirs
                .into_iter()
                .filter_map(|common_dir| {
                    let group = candidate_ranks
                        .get(&common_dir)
                        .and_then(|(_, group)| group.clone());
                    let is_candidate = candidate_ranks.contains_key(&common_dir);
                    match git::list_worktrees(&common_dir) {
                        Ok(worktrees) => Some(WorktreeRepository {
                            label: repository_label(&common_dir, &worktrees),
                            group,
                            common_dir,
                            worktrees,
                            error: None,
                        }),
                        Err(error) => {
                            if is_candidate {
                                Some(WorktreeRepository {
                                    label: repository_label(&common_dir, &[]),
                                    group,
                                    common_dir,
                                    worktrees: Vec::new(),
                                    error: Some(error.to_string()),
                                })
                            } else {
                                pruned.push(common_dir);
                                None
                            }
                        }
                    }
                })
                .collect();
            let _ = sender.send(Completion::Inventory(InventoryCompletion {
                generation,
                repositories,
                discovered,
                pruned,
            }));
        });
    }

    pub(crate) fn start_remove(&mut self, common_dir: PathBuf, path: PathBuf) -> bool {
        if self.operation_running() {
            return false;
        }
        self.remove_running = true;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result =
                git::remove_worktree(&common_dir, &path).map_err(|error| error.to_string());
            let _ = sender.send(Completion::Removal(RemovalCompletion { path, result }));
        });
        true
    }

    pub(crate) fn start_create(
        &mut self,
        cwd: PathBuf,
        path: PathBuf,
        branch: String,
        start_point: String,
    ) -> bool {
        if self.operation_running() {
            return false;
        }
        self.create_running = true;
        self.pending_create = self.create_dialog.take();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = super::workspace_panel::create_managed_worktree(
                cwd,
                path.clone(),
                branch,
                start_point,
            );
            let _ = sender.send(Completion::Creation(CreationCompletion { path, result }));
        });
        true
    }

    pub(crate) fn poll(&mut self) -> WorktreeManagerPoll {
        let mut result = WorktreeManagerPoll::default();
        let mut refresh = false;
        while let Ok(completion) = self.receiver.try_recv() {
            result.changed = true;
            match completion {
                Completion::Inventory(completion) => {
                    if completion.generation != self.generation {
                        continue;
                    }
                    self.loading = false;
                    let selected_path = self
                        .selected_worktree()
                        .map(|(_, worktree)| worktree.path.clone());
                    self.repositories = completion.repositories;
                    self.bump_content_generation();
                    if !completion.discovered.is_empty()
                        && let Err(error) = self.store.extend_and_save(completion.discovered)
                    {
                        result.notice = Some(error);
                    }
                    if !completion.pruned.is_empty()
                        && let Err(error) = self.store.prune_and_save(&completion.pruned)
                    {
                        result.notice = Some(error);
                    }
                    self.restore_selection(selected_path.as_deref());
                }
                Completion::Creation(completion) => {
                    self.create_running = false;
                    match completion.result {
                        Ok(()) => {
                            self.pending_create = None;
                            result.notice =
                                Some(format!("Created worktree {}", completion.path.display()));
                            result.open_path = Some(completion.path);
                            refresh = true;
                        }
                        Err(error) => {
                            if let Some(mut dialog) = self.pending_create.take() {
                                dialog.error = Some(error.clone());
                                self.create_dialog = Some(dialog);
                            }
                            result.notice = Some(error);
                        }
                    }
                }
                Completion::Removal(completion) => {
                    self.remove_running = false;
                    match completion.result {
                        Ok(()) => {
                            result.notice =
                                Some(format!("Removed worktree {}", completion.path.display()));
                            refresh = true;
                        }
                        Err(error) => result.notice = Some(error),
                    }
                }
            }
        }
        if refresh {
            self.start_refresh();
        }
        result
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<WorktreeManagerEffect> {
        if self.create_dialog.is_some() {
            return self.handle_create_dialog(key);
        }
        if self.remove_dialog.is_some() {
            return self.handle_remove_dialog(key);
        }
        match key.code {
            KeyCode::Esc => Some(WorktreeManagerEffect::Close),
            KeyCode::Enter => self.activate_selected(),
            KeyCode::Char('N') => self.begin_create(),
            KeyCode::Delete if key.modifiers.is_empty() => self.begin_remove(),
            KeyCode::Down => {
                self.move_selection(1);
                None
            }
            KeyCode::Up => {
                self.move_selection(-1);
                None
            }
            KeyCode::Home => {
                self.select_boundary(false);
                None
            }
            KeyCode::End => {
                self.select_boundary(true);
                None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.bump_content_generation();
                self.select_first();
                None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.bump_content_generation();
                self.select_first();
                None
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(WorktreeManagerEffect::Refresh)
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.bump_content_generation();
                self.select_first();
                None
            }
            _ => None,
        }
    }

    pub(crate) fn paste(&mut self, text: &str) {
        if self.remove_dialog.is_some() {
            return;
        }
        if self.create_dialog.is_some() {
            self.paste_create_dialog(text);
            return;
        }
        self.query.extend(
            text.chars()
                .filter(|character| !matches!(character, '\r' | '\n')),
        );
        self.bump_content_generation();
        self.select_first();
    }

    pub(crate) fn rows(&self) -> Vec<WorktreeManagerRow> {
        let query = self.query.to_lowercase();
        let mut rows = Vec::new();
        let show_groups = self
            .repositories
            .iter()
            .any(|repository| repository.group.is_some());
        let mut previous_group = None;
        for (repository_index, repository) in self.repositories.iter().enumerate() {
            let repository_matches = query.is_empty()
                || repository.label.to_lowercase().contains(&query)
                || repository
                    .common_dir
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query)
                || repository
                    .error
                    .as_deref()
                    .is_some_and(|error| error.to_lowercase().contains(&query));
            let matching = repository
                .worktrees
                .iter()
                .enumerate()
                .filter_map(|(worktree_index, worktree)| {
                    (repository_matches || worktree_matches(worktree, &query))
                        .then_some(worktree_index)
                })
                .collect::<Vec<_>>();
            if matching.is_empty() && (!repository_matches || repository.error.is_none()) {
                continue;
            }
            if show_groups && previous_group.as_ref() != Some(&repository.group) {
                rows.push(WorktreeManagerRow::Group(repository_index));
                previous_group = Some(repository.group.clone());
            }
            if repository.error.is_some() {
                rows.push(WorktreeManagerRow::Status(repository_index));
            } else {
                rows.extend(
                    matching
                        .into_iter()
                        .map(|worktree| WorktreeManagerRow::Worktree {
                            repository: repository_index,
                            worktree,
                        }),
                );
            }
        }
        rows
    }

    pub(crate) fn worktree_count(&self) -> usize {
        self.repositories
            .iter()
            .map(|repository| repository.worktrees.len())
            .sum()
    }

    pub(crate) fn is_current(&self, path: &Path) -> bool {
        self.current_path
            .as_deref()
            .is_some_and(|current| same_path(current, path))
    }

    pub(crate) fn is_herdr(&self, path: &Path) -> bool {
        self.herdr_worktrees
            .iter()
            .any(|(candidate, _)| same_path(candidate, path))
    }

    pub(crate) fn content_generation(&self) -> u64 {
        self.content_generation
    }

    pub(crate) fn select_row(&mut self, generation: u64, row: usize) -> bool {
        if generation != self.content_generation {
            return false;
        }
        if !matches!(
            self.rows().get(row),
            Some(WorktreeManagerRow::Worktree { .. })
        ) {
            return false;
        }
        self.state.select(Some(row));
        true
    }

    pub(crate) fn click_row(
        &mut self,
        generation: u64,
        row: usize,
    ) -> Option<WorktreeManagerEffect> {
        if !self.select_row(generation, row) {
            return None;
        }
        let (_, worktree) = self.selected_worktree()?;
        let path = worktree.path.clone();
        let double_click = self.last_click.as_ref().is_some_and(|(previous, at)| {
            same_path(previous, &path) && at.elapsed() <= DOUBLE_CLICK_INTERVAL
        });
        if double_click {
            self.last_click = None;
            self.activate_selected()
        } else {
            self.last_click = Some((path, Instant::now()));
            None
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let rows = self.rows();
        let worktree_rows = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                matches!(row, WorktreeManagerRow::Worktree { .. }).then_some(index)
            })
            .collect::<Vec<_>>();
        if worktree_rows.is_empty() {
            self.state.select(None);
            return;
        }
        let current = self
            .state
            .selected()
            .and_then(|selected| worktree_rows.iter().position(|row| *row == selected))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(worktree_rows.len() - 1);
        self.state.select(Some(worktree_rows[next]));
    }

    pub(crate) fn remove_dialog_open(&self) -> bool {
        self.remove_dialog.is_some()
    }

    pub(crate) fn dialog_open(&self) -> bool {
        self.create_dialog.is_some() || self.remove_dialog_open()
    }

    pub(crate) fn operation_running(&self) -> bool {
        self.create_running || self.remove_running
    }

    pub(crate) fn open_protection(&self, worktree: &LinkedWorktree) -> Option<String> {
        if self.operation_running() {
            return Some("Wait for the worktree operation to finish".to_owned());
        }
        if worktree.is_bare {
            return Some("Bare repositories cannot be opened as workspaces".to_owned());
        }
        worktree
            .prunable
            .then(|| "This worktree is missing and can only be pruned".to_owned())
    }

    pub(crate) fn create_protection(&self, worktree: &LinkedWorktree) -> Option<String> {
        if self.operation_running() {
            return Some("A worktree operation is already running".to_owned());
        }
        (worktree.is_bare || worktree.prunable)
            .then(|| "Select an available checkout as the starting point".to_owned())
    }

    pub(crate) fn remove_protection(&self, worktree: &LinkedWorktree) -> Option<String> {
        if self.operation_running() {
            return Some("A worktree operation is already running".to_owned());
        }
        if self.herdr_enabled && !self.herdr_verified {
            return Some("Waiting for Herdr to verify linked worktree ownership".to_owned());
        }
        if worktree.is_main {
            return Some("The primary worktree cannot be removed".to_owned());
        }
        if worktree.locked {
            return Some(worktree.locked_reason.as_ref().map_or_else(
                || "Unlock this worktree before removing it".to_owned(),
                |reason| format!("Worktree is locked: {reason}"),
            ));
        }
        if worktree.prunable {
            return Some("This missing worktree requires repository metadata pruning".to_owned());
        }
        self.current_path
            .as_deref()
            .is_some_and(|current| same_path(current, &worktree.path))
            .then(|| "Open another worktree before removing the current one".to_owned())
    }

    fn activate_selected(&self) -> Option<WorktreeManagerEffect> {
        let (_, worktree) = self.selected_worktree()?;
        if let Some(reason) = self.open_protection(worktree) {
            return Some(WorktreeManagerEffect::Notice(reason));
        }
        Some(WorktreeManagerEffect::Open(worktree.path.clone()))
    }

    fn begin_remove(&mut self) -> Option<WorktreeManagerEffect> {
        let (repository, worktree) = self.selected_worktree()?;
        if let Some(reason) = self.remove_protection(worktree) {
            return Some(WorktreeManagerEffect::Notice(reason));
        }
        let herdr_workspace_id = self
            .herdr_worktrees
            .iter()
            .find(|(path, _)| same_path(path, &worktree.path))
            .map(|(_, workspace_id)| workspace_id.clone());
        self.remove_dialog = Some(WorktreeRemoveDialog {
            common_dir: repository.common_dir.clone(),
            path: worktree.path.clone(),
            label: worktree_label(worktree),
            herdr_workspace_id,
        });
        None
    }

    fn begin_create(&mut self) -> Option<WorktreeManagerEffect> {
        let (repository, worktree) = self.selected_worktree()?;
        if let Some(reason) = self.create_protection(worktree) {
            return Some(WorktreeManagerEffect::Notice(reason));
        }
        let base_dir = worktree
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| worktree.path.clone());
        let repository_label = repository.label.clone();
        let cwd = worktree.path.clone();
        let start_label = worktree_label(worktree);
        let start_point = worktree.head.clone().unwrap_or_default();
        self.create_dialog = Some(WorktreeCreateDialog {
            repository_label: repository_label.clone(),
            start_label,
            branch: String::new(),
            path: base_dir
                .join(format!("{repository_label}-"))
                .display()
                .to_string(),
            field: WorktreeCreateField::Branch,
            error: None,
            cwd,
            start_point,
            base_dir,
            path_automatic: true,
        });
        None
    }

    fn handle_create_dialog(&mut self, key: KeyEvent) -> Option<WorktreeManagerEffect> {
        match key.code {
            KeyCode::Esc => {
                self.create_dialog = None;
                None
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let dialog = self.create_dialog.as_mut()?;
                dialog.field = match dialog.field {
                    WorktreeCreateField::Branch => WorktreeCreateField::Path,
                    WorktreeCreateField::Path => WorktreeCreateField::Branch,
                };
                dialog.error = None;
                None
            }
            KeyCode::Enter => {
                if self.create_dialog.as_ref()?.field == WorktreeCreateField::Branch {
                    self.create_dialog.as_mut()?.field = WorktreeCreateField::Path;
                    return None;
                }
                self.create_effect()
            }
            KeyCode::Backspace => {
                let dialog = self.create_dialog.as_mut()?;
                match dialog.field {
                    WorktreeCreateField::Branch => {
                        dialog.branch.pop();
                        update_automatic_path(dialog);
                    }
                    WorktreeCreateField::Path => {
                        dialog.path.pop();
                        dialog.path_automatic = false;
                    }
                }
                dialog.error = None;
                None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let dialog = self.create_dialog.as_mut()?;
                match dialog.field {
                    WorktreeCreateField::Branch => {
                        dialog.branch.clear();
                        update_automatic_path(dialog);
                    }
                    WorktreeCreateField::Path => {
                        dialog.path.clear();
                        dialog.path_automatic = false;
                    }
                }
                dialog.error = None;
                None
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let dialog = self.create_dialog.as_mut()?;
                match dialog.field {
                    WorktreeCreateField::Branch => {
                        dialog.branch.push(character);
                        update_automatic_path(dialog);
                    }
                    WorktreeCreateField::Path => {
                        dialog.path.push(character);
                        dialog.path_automatic = false;
                    }
                }
                dialog.error = None;
                None
            }
            _ => None,
        }
    }

    fn paste_create_dialog(&mut self, text: &str) {
        let Some(dialog) = self.create_dialog.as_mut() else {
            return;
        };
        let text = text
            .chars()
            .filter(|character| !matches!(character, '\r' | '\n'))
            .collect::<String>();
        match dialog.field {
            WorktreeCreateField::Branch => {
                dialog.branch.push_str(&text);
                update_automatic_path(dialog);
            }
            WorktreeCreateField::Path => {
                dialog.path.push_str(&text);
                dialog.path_automatic = false;
            }
        }
        dialog.error = None;
    }

    fn create_effect(&mut self) -> Option<WorktreeManagerEffect> {
        let dialog = self.create_dialog.as_mut()?;
        if dialog.branch.is_empty() {
            dialog.error = Some("Enter a branch name".to_owned());
            dialog.field = WorktreeCreateField::Branch;
            return None;
        }
        if dialog.path.is_empty() {
            dialog.error = Some("Enter a destination path".to_owned());
            return None;
        }
        let path = expand_home(&dialog.path);
        let path = if path.is_absolute() {
            path
        } else {
            dialog.base_dir.join(path)
        };
        Some(WorktreeManagerEffect::CreateHerdr {
            cwd: dialog.cwd.clone(),
            path,
            branch: dialog.branch.clone(),
            start_point: dialog.start_point.clone(),
        })
    }

    fn handle_remove_dialog(&mut self, key: KeyEvent) -> Option<WorktreeManagerEffect> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.remove_dialog = None;
                None
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                let dialog = self.remove_dialog.take()?;
                Some(if let Some(workspace_id) = dialog.herdr_workspace_id {
                    WorktreeManagerEffect::RemoveHerdr {
                        workspace_id,
                        path: dialog.path,
                    }
                } else {
                    WorktreeManagerEffect::RemoveNative {
                        common_dir: dialog.common_dir,
                        path: dialog.path,
                    }
                })
            }
            _ => None,
        }
    }

    pub(crate) fn selected_worktree(&self) -> Option<(&WorktreeRepository, &LinkedWorktree)> {
        let row = *self.rows().get(self.state.selected()?)?;
        let WorktreeManagerRow::Worktree {
            repository,
            worktree,
        } = row
        else {
            return None;
        };
        Some((
            self.repositories.get(repository)?,
            self.repositories.get(repository)?.worktrees.get(worktree)?,
        ))
    }

    fn select_first(&mut self) {
        let first = self
            .rows()
            .iter()
            .position(|row| matches!(row, WorktreeManagerRow::Worktree { .. }));
        self.state.select(first);
    }

    fn select_boundary(&mut self, end: bool) {
        let rows = self.rows();
        let selected = if end {
            rows.iter()
                .rposition(|row| matches!(row, WorktreeManagerRow::Worktree { .. }))
        } else {
            rows.iter()
                .position(|row| matches!(row, WorktreeManagerRow::Worktree { .. }))
        };
        self.state.select(selected);
    }

    fn restore_selection(&mut self, path: Option<&Path>) {
        let rows = self.rows();
        let selected = path.and_then(|path| {
            rows.iter().position(|row| match row {
                WorktreeManagerRow::Worktree {
                    repository,
                    worktree,
                } => self.repositories[*repository]
                    .worktrees
                    .get(*worktree)
                    .is_some_and(|candidate| same_path(&candidate.path, path)),
                _ => false,
            })
        });
        self.state.select(selected.or_else(|| {
            rows.iter()
                .position(|row| matches!(row, WorktreeManagerRow::Worktree { .. }))
        }));
    }

    fn bump_content_generation(&mut self) {
        self.content_generation = self.content_generation.wrapping_add(1);
        self.last_click = None;
    }
}

pub(crate) fn worktree_label(worktree: &LinkedWorktree) -> String {
    worktree
        .branch
        .as_deref()
        .and_then(|branch| branch.strip_prefix("refs/heads/"))
        .map(str::to_owned)
        .or_else(|| {
            worktree
                .head
                .as_deref()
                .map(|head| format!("detached @ {}", short_head(head)))
        })
        .unwrap_or_else(|| "bare repository".to_owned())
}

pub(crate) fn short_head(head: &str) -> String {
    head.chars().take(7).collect()
}

fn worktree_matches(worktree: &LinkedWorktree, query: &str) -> bool {
    query.is_empty()
        || worktree_label(worktree).to_lowercase().contains(query)
        || worktree
            .path
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
        || worktree
            .head
            .as_deref()
            .is_some_and(|head| head.to_lowercase().contains(query))
}

fn repository_label(common_dir: &Path, worktrees: &[LinkedWorktree]) -> String {
    worktrees
        .first()
        .and_then(|worktree| worktree.path.file_name())
        .or_else(|| {
            (common_dir.file_name().is_some_and(|name| name == ".git"))
                .then(|| common_dir.parent().and_then(Path::file_name))
                .flatten()
        })
        .or_else(|| common_dir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| common_dir.display().to_string())
}

fn update_automatic_path(dialog: &mut WorktreeCreateDialog) {
    if !dialog.path_automatic {
        return;
    }
    let suffix = dialog
        .branch
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    dialog.path = dialog
        .base_dir
        .join(format!("{}-{suffix}", dialog.repository_label))
        .display()
        .to_string();
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return std::env::var_os("HOME").map_or_else(|| PathBuf::from(path), PathBuf::from);
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    use super::*;

    fn linked(path: &str, branch: &str, is_main: bool) -> LinkedWorktree {
        LinkedWorktree {
            path: PathBuf::from(path),
            head: Some("1234567890abcdef".to_owned()),
            branch: Some(format!("refs/heads/{branch}")),
            is_main,
            is_detached: false,
            is_bare: false,
            locked: false,
            locked_reason: None,
            prunable: false,
            prunable_reason: None,
        }
    }

    fn manager() -> WorktreeManager {
        let mut manager = WorktreeManager::new(None);
        manager.repositories = vec![WorktreeRepository {
            common_dir: PathBuf::from("/repo/.git"),
            label: "repo".to_owned(),
            group: None,
            worktrees: vec![
                linked("/repo", "main", true),
                linked("/repo-feature", "feature/modal", false),
            ],
            error: None,
        }];
        manager.current_path = Some(PathBuf::from("/repo"));
        manager.select_first();
        manager
    }

    #[test]
    fn filters_branch_and_path_to_checkout_rows() {
        let mut manager = manager();
        manager.query = "modal".to_owned();
        manager.select_first();

        assert_eq!(
            manager.rows(),
            [WorktreeManagerRow::Worktree {
                repository: 0,
                worktree: 1,
            }]
        );
        assert_eq!(manager.state.selected(), Some(0));
    }

    #[test]
    fn inserts_group_headings_only_when_groups_change() {
        let mut manager = manager();
        manager.repositories = vec![
            WorktreeRepository {
                common_dir: PathBuf::from("/alpha/.git"),
                label: "alpha".to_owned(),
                group: Some("Projects".to_owned()),
                worktrees: vec![linked("/alpha", "main", true)],
                error: None,
            },
            WorktreeRepository {
                common_dir: PathBuf::from("/zulu/.git"),
                label: "zulu".to_owned(),
                group: Some("Projects".to_owned()),
                worktrees: vec![linked("/zulu", "main", true)],
                error: None,
            },
            WorktreeRepository {
                common_dir: PathBuf::from("/solo/.git"),
                label: "solo".to_owned(),
                group: None,
                worktrees: vec![linked("/solo", "main", true)],
                error: None,
            },
        ];
        manager.select_first();

        assert_eq!(
            manager.rows(),
            [
                WorktreeManagerRow::Group(0),
                WorktreeManagerRow::Worktree {
                    repository: 0,
                    worktree: 0,
                },
                WorktreeManagerRow::Worktree {
                    repository: 1,
                    worktree: 0,
                },
                WorktreeManagerRow::Group(2),
                WorktreeManagerRow::Worktree {
                    repository: 2,
                    worktree: 0,
                },
            ]
        );
        assert_eq!(manager.state.selected(), Some(1));
    }

    #[test]
    fn opens_selected_linked_worktree() {
        let mut manager = manager();
        manager.move_selection(1);

        assert_eq!(
            manager.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::Open(PathBuf::from("/repo-feature")))
        );
    }

    #[test]
    fn creates_from_the_selected_checkout_with_an_automatic_path() {
        let mut manager = manager();
        manager.move_selection(1);

        assert_eq!(
            manager.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT)),
            None
        );
        manager.paste("feature/new");
        let dialog = manager.create_dialog.as_ref().unwrap();
        assert_eq!(dialog.path, "/repo-feature-new");

        manager.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            manager.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::CreateHerdr {
                cwd: PathBuf::from("/repo-feature"),
                path: PathBuf::from("/repo-feature-new"),
                branch: "feature/new".to_owned(),
                start_point: "1234567890abcdef".to_owned(),
            })
        );
    }

    #[test]
    fn protects_primary_and_current_worktrees_from_removal() {
        let mut manager = manager();
        assert!(matches!(
            manager.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::Notice(_))
        ));

        manager.repositories[0].worktrees[0].is_main = false;
        assert!(matches!(
            manager.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::Notice(_))
        ));
    }

    #[test]
    fn routes_live_herdr_worktree_removal_after_confirmation() {
        let mut manager = manager();
        manager.herdr_worktrees = vec![(PathBuf::from("/repo-feature"), "workspace-2".to_owned())];
        manager.move_selection(1);

        assert_eq!(
            manager.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            None
        );
        assert!(manager.remove_dialog_open());
        manager.paste("hidden filter");
        assert!(manager.query.is_empty());
        assert_eq!(
            manager.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::RemoveHerdr {
                workspace_id: "workspace-2".to_owned(),
                path: PathBuf::from("/repo-feature"),
            })
        );
    }

    #[test]
    fn persists_known_repository_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known-repositories.json");
        let mut manager = WorktreeManager::new(Some(path.clone()));
        manager.remember(Path::new("/repo/.git")).unwrap();

        let restored = WorktreeManager::new(Some(path));
        assert_eq!(restored.store.repositories, [PathBuf::from("/repo/.git")]);
    }

    #[test]
    fn rejects_stale_semantic_rows_after_filtering() {
        let mut manager = manager();
        let generation = manager.content_generation();

        manager.paste("feature");

        assert!(!manager.select_row(generation, 1));
        assert_eq!(manager.state.selected(), Some(0));
    }

    #[test]
    fn waits_for_verified_herdr_inventory_before_removing() {
        let mut manager = manager();
        manager.herdr_enabled = true;
        manager.herdr_verified = false;
        manager.move_selection(1);

        assert!(matches!(
            manager.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::Notice(message))
                if message.contains("Waiting for Herdr")
        ));
        assert!(!manager.remove_dialog_open());
    }

    #[test]
    fn detects_new_candidate_paths_from_a_late_herdr_snapshot() {
        let mut manager = manager();
        let paths = vec![WorktreeCandidate {
            path: PathBuf::from("/another-repository"),
            group: None,
        }];

        assert!(manager.update_herdr_inventory(paths.clone(), Vec::new(), true));
        assert!(!manager.update_herdr_inventory(paths, Vec::new(), true));
    }

    #[test]
    fn orders_repositories_by_workspace_candidate_order() {
        let directory = tempfile::tempdir().unwrap();
        let alpha = directory.path().join("alpha");
        let zulu = directory.path().join("zulu");
        for repository in [&alpha, &zulu] {
            fs::create_dir(repository).unwrap();
            assert!(
                Command::new("git")
                    .args(["init", "--quiet"])
                    .current_dir(repository)
                    .status()
                    .unwrap()
                    .success()
            );
        }

        let mut manager = WorktreeManager::new(None);
        manager.candidates = vec![
            WorktreeCandidate {
                path: zulu,
                group: Some("First".to_owned()),
            },
            WorktreeCandidate {
                path: alpha,
                group: Some("Second".to_owned()),
            },
        ];
        manager.start_refresh();
        let deadline = Instant::now() + Duration::from_secs(2);
        while manager.loading && Instant::now() < deadline {
            manager.poll();
            thread::sleep(Duration::from_millis(10));
        }

        assert!(!manager.loading);
        assert_eq!(
            manager
                .repositories
                .iter()
                .map(|repository| repository.label.as_str())
                .collect::<Vec<_>>(),
            ["zulu", "alpha"]
        );
        assert_eq!(manager.repositories[0].group.as_deref(), Some("First"));
        assert_eq!(manager.repositories[1].group.as_deref(), Some("Second"));
    }

    #[test]
    fn prunes_stale_known_repositories_that_no_longer_resolve() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("live");
        fs::create_dir(&live).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&live)
                .status()
                .unwrap()
                .success()
        );
        let live_common = live.join(".git");
        let stale_common = directory.path().join("renamed-away").join(".git");

        let store_path = directory.path().join("known-repositories.json");
        fs::write(
            &store_path,
            format!(
                "{{\"version\":1,\"repositories\":[{{\"common_dir\":\"{}\"}},{{\"common_dir\":\"{}\"}}]}}",
                stale_common.display(),
                live_common.display()
            ),
        )
        .unwrap();

        let mut manager = WorktreeManager::new(Some(store_path.clone()));
        manager.candidates = vec![WorktreeCandidate {
            path: live,
            group: None,
        }];
        manager.start_refresh();
        let deadline = Instant::now() + Duration::from_secs(2);
        while manager.loading && Instant::now() < deadline {
            manager.poll();
            thread::sleep(Duration::from_millis(10));
        }

        assert!(!manager.loading);
        assert_eq!(manager.repositories.len(), 1);
        assert_eq!(manager.repositories[0].common_dir, live_common);
        assert!(
            !manager
                .repositories
                .iter()
                .any(|repository| repository.common_dir == stale_common)
        );
        let restored = WorktreeManager::new(Some(store_path));
        assert_eq!(restored.store.repositories, [live_common]);
    }

    #[test]
    fn malformed_inventory_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known-repositories.json");
        fs::write(&path, b"not json").unwrap();
        let mut manager = WorktreeManager::new(Some(path.clone()));

        assert!(manager.remember(Path::new("/repo/.git")).is_err());
        assert!(manager.store.repositories.is_empty());
        assert_eq!(fs::read(path).unwrap(), b"not json");
    }

    #[cfg(unix)]
    #[test]
    fn persists_non_utf8_repository_identity_without_loss() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known-repositories.json");
        let common_dir = PathBuf::from(std::ffi::OsString::from_vec(b"/repo/\xff/.git".to_vec()));
        let mut manager = WorktreeManager::new(Some(path.clone()));

        manager.remember(&common_dir).unwrap();

        let restored = WorktreeManager::new(Some(path));
        assert_eq!(restored.store.repositories, [common_dir]);
    }

    #[test]
    fn short_head_handles_multibyte_and_malformed_values() {
        assert_eq!(short_head("åbcdefgh"), "åbcdefg");
        assert_eq!(short_head("bad"), "bad");
    }
}
