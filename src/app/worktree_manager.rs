use std::{
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

use super::{LinkedWorktreeCatalogSnapshot, LinkedWorktreeRepository};

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
    Remove(PathBuf),
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
    pub(crate) path: PathBuf,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeManagerRow {
    Group(usize),
    Worktree { repository: usize, worktree: usize },
    Status(usize),
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
    Creation(CreationCompletion),
    Removal(RemovalCompletion),
}

#[derive(Default)]
pub(crate) struct WorktreeManagerPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
    pub(crate) open_path: Option<PathBuf>,
    pub(crate) refresh_catalog: bool,
}

pub(crate) struct WorktreeManager {
    pub(crate) query: String,
    pub(crate) state: ListState,
    catalog: LinkedWorktreeCatalogSnapshot,
    pub(crate) create_running: bool,
    pub(crate) create_dialog: Option<WorktreeCreateDialog>,
    pub(crate) remove_running: bool,
    pub(crate) remove_dialog: Option<WorktreeRemoveDialog>,
    content_generation: u64,
    last_click: Option<(PathBuf, Instant)>,
    pending_create: Option<WorktreeCreateDialog>,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
}

impl WorktreeManager {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            query: String::new(),
            state: ListState::default(),
            catalog: LinkedWorktreeCatalogSnapshot::default(),
            create_running: false,
            create_dialog: None,
            remove_running: false,
            remove_dialog: None,
            content_generation: 0,
            last_click: None,
            pending_create: None,
            sender,
            receiver,
        }
    }

    pub(crate) fn open(&mut self, catalog: LinkedWorktreeCatalogSnapshot) {
        self.query.clear();
        self.create_dialog = None;
        self.remove_dialog = None;
        self.last_click = None;
        self.replace_catalog(catalog);
    }

    pub(crate) fn replace_catalog(&mut self, catalog: LinkedWorktreeCatalogSnapshot) {
        if self.catalog.revision() == catalog.revision() {
            return;
        }
        let selected_path = self
            .selected_worktree()
            .map(|(_, worktree)| worktree.path.clone());
        self.catalog = catalog;
        self.bump_content_generation();
        self.restore_selection(selected_path.as_deref());
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
        while let Ok(completion) = self.receiver.try_recv() {
            result.changed = true;
            match completion {
                Completion::Creation(completion) => {
                    self.create_running = false;
                    match completion.result {
                        Ok(()) => {
                            self.pending_create = None;
                            result.notice =
                                Some(format!("Created worktree {}", completion.path.display()));
                            result.open_path = Some(completion.path);
                            result.refresh_catalog = true;
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
                            result.refresh_catalog = true;
                        }
                        Err(error) => result.notice = Some(error),
                    }
                }
            }
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
            .catalog
            .repositories
            .iter()
            .any(|repository| repository.group.is_some());
        let mut previous_group = None;
        for (repository_index, repository) in self.catalog.repositories.iter().enumerate() {
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
        self.catalog
            .repositories
            .iter()
            .map(|repository| repository.worktrees.len())
            .sum()
    }

    pub(crate) fn repositories(&self) -> &[LinkedWorktreeRepository] {
        &self.catalog.repositories
    }

    pub(crate) fn loading(&self) -> bool {
        self.catalog.loading
    }

    pub(crate) fn is_current(&self, path: &Path) -> bool {
        self.catalog.is_active(path)
    }

    pub(crate) fn is_herdr(&self, path: &Path) -> bool {
        self.catalog.is_herdr_owned(path)
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
        self.catalog.removal_plan(&worktree.path).err()
    }

    fn activate_selected(&self) -> Option<WorktreeManagerEffect> {
        let (_, worktree) = self.selected_worktree()?;
        if let Some(reason) = self.open_protection(worktree) {
            return Some(WorktreeManagerEffect::Notice(reason));
        }
        Some(WorktreeManagerEffect::Open(worktree.path.clone()))
    }

    fn begin_remove(&mut self) -> Option<WorktreeManagerEffect> {
        let (_, worktree) = self.selected_worktree()?;
        if let Some(reason) = self.remove_protection(worktree) {
            return Some(WorktreeManagerEffect::Notice(reason));
        }
        self.remove_dialog = Some(WorktreeRemoveDialog {
            path: worktree.path.clone(),
            label: worktree_label(worktree),
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
                Some(WorktreeManagerEffect::Remove(dialog.path))
            }
            _ => None,
        }
    }

    pub(crate) fn selected_worktree(&self) -> Option<(&LinkedWorktreeRepository, &LinkedWorktree)> {
        let row = *self.rows().get(self.state.selected()?)?;
        let WorktreeManagerRow::Worktree {
            repository,
            worktree,
        } = row
        else {
            return None;
        };
        Some((
            self.catalog.repositories.get(repository)?,
            self.catalog
                .repositories
                .get(repository)?
                .worktrees
                .get(worktree)?,
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
                } => self.catalog.repositories[*repository]
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
    use super::*;
    use crate::app::HerdrOwnership;

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
        let mut manager = WorktreeManager::new();
        manager.catalog = LinkedWorktreeCatalogSnapshot::for_test(
            vec![LinkedWorktreeRepository {
                common_dir: PathBuf::from("/repo/.git"),
                label: "repo".to_owned(),
                group: None,
                worktrees: vec![
                    linked("/repo", "main", true),
                    linked("/repo-feature", "feature/modal", false),
                ],
                error: None,
            }],
            Some(PathBuf::from("/repo")),
            HerdrOwnership::Disabled,
        );
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
        manager.catalog = LinkedWorktreeCatalogSnapshot::for_test(
            vec![
                LinkedWorktreeRepository {
                    common_dir: PathBuf::from("/alpha/.git"),
                    label: "alpha".to_owned(),
                    group: Some("Projects".to_owned()),
                    worktrees: vec![linked("/alpha", "main", true)],
                    error: None,
                },
                LinkedWorktreeRepository {
                    common_dir: PathBuf::from("/zulu/.git"),
                    label: "zulu".to_owned(),
                    group: Some("Projects".to_owned()),
                    worktrees: vec![linked("/zulu", "main", true)],
                    error: None,
                },
                LinkedWorktreeRepository {
                    common_dir: PathBuf::from("/solo/.git"),
                    label: "solo".to_owned(),
                    group: None,
                    worktrees: vec![linked("/solo", "main", true)],
                    error: None,
                },
            ],
            None,
            HerdrOwnership::Disabled,
        );
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

        manager.catalog.repositories[0].worktrees[0].is_main = false;
        assert!(matches!(
            manager.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::Notice(_))
        ));
    }

    #[test]
    fn emits_semantic_removal_after_confirmation() {
        let mut manager = manager();
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
            Some(WorktreeManagerEffect::Remove(PathBuf::from(
                "/repo-feature"
            )))
        );
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
        manager.catalog = LinkedWorktreeCatalogSnapshot::for_test(
            manager.catalog.repositories.clone(),
            Some(PathBuf::from("/repo")),
            HerdrOwnership::Unverified,
        );
        manager.move_selection(1);

        assert!(matches!(
            manager.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            Some(WorktreeManagerEffect::Notice(message))
                if message.contains("Waiting for Herdr")
        ));
        assert!(!manager.remove_dialog_open());
    }

    #[test]
    fn short_head_handles_multibyte_and_malformed_values() {
        assert_eq!(short_head("åbcdefgh"), "åbcdefg");
        assert_eq!(short_head("bad"), "bad");
    }
}
