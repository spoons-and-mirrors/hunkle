use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use serde_json::Value;

use crate::git::{self, LinkedWorktree};

use super::atomic_write;

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorktreeManagerEffect {
    Close,
    Open(PathBuf),
    Refresh,
    RemoveNative { common_dir: PathBuf, path: PathBuf },
    RemoveHerdr { workspace_id: String, path: PathBuf },
    Notice(String),
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
    pub(crate) worktrees: Vec<LinkedWorktree>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeManagerRow {
    Repository(usize),
    Worktree { repository: usize, worktree: usize },
    Status(usize),
}

struct InventoryCompletion {
    generation: u64,
    repositories: Vec<WorktreeRepository>,
    discovered: Vec<PathBuf>,
}

struct RemovalCompletion {
    path: PathBuf,
    result: Result<(), String>,
}

enum Completion {
    Inventory(InventoryCompletion),
    Removal(RemovalCompletion),
}

#[derive(Default)]
pub(crate) struct WorktreeManagerPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
}

pub(crate) struct WorktreeManager {
    pub(crate) query: String,
    pub(crate) state: ListState,
    pub(crate) repositories: Vec<WorktreeRepository>,
    pub(crate) loading: bool,
    pub(crate) remove_running: bool,
    pub(crate) remove_dialog: Option<WorktreeRemoveDialog>,
    current_path: Option<PathBuf>,
    candidate_paths: Vec<PathBuf>,
    herdr_worktrees: Vec<(PathBuf, String)>,
    herdr_enabled: bool,
    herdr_verified: bool,
    store: KnownRepositoryStore,
    generation: u64,
    content_generation: u64,
    last_click: Option<(PathBuf, Instant)>,
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
            remove_running: false,
            remove_dialog: None,
            current_path: None,
            candidate_paths: Vec::new(),
            herdr_worktrees: Vec::new(),
            herdr_enabled: false,
            herdr_verified: true,
            store: KnownRepositoryStore::new(store_path),
            generation: 0,
            content_generation: 0,
            last_click: None,
            sender,
            receiver,
        }
    }

    pub(crate) fn remember(&mut self, common_dir: &Path) -> Result<(), String> {
        self.store.extend_and_save(vec![common_dir.to_owned()])
    }

    pub(crate) fn open(
        &mut self,
        candidate_paths: Vec<PathBuf>,
        herdr_worktrees: Vec<(PathBuf, String)>,
        current_path: Option<PathBuf>,
        herdr_enabled: bool,
        herdr_verified: bool,
    ) -> Option<String> {
        self.query.clear();
        self.remove_dialog = None;
        self.current_path = current_path;
        self.candidate_paths = candidate_paths;
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
        candidate_paths: Vec<PathBuf>,
        herdr_worktrees: Vec<(PathBuf, String)>,
        verified: bool,
    ) -> bool {
        let candidates_changed = self.candidate_paths != candidate_paths;
        self.candidate_paths = candidate_paths;
        self.herdr_worktrees = herdr_worktrees;
        self.herdr_verified = verified;
        candidates_changed
    }

    pub(crate) fn start_refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let known = self.store.repositories.clone();
        let candidates = self.candidate_paths.clone();
        let sender = self.sender.clone();
        self.loading = true;
        thread::spawn(move || {
            let mut common_dirs = known;
            let mut seen = common_dirs.iter().cloned().collect::<HashSet<_>>();
            let mut discovered = Vec::new();
            for candidate in candidates {
                let Ok(common_dir) = git::common_git_dir(&candidate) else {
                    continue;
                };
                if seen.insert(common_dir.clone()) {
                    discovered.push(common_dir.clone());
                    common_dirs.push(common_dir);
                }
            }
            common_dirs.sort_by_cached_key(|path| path.to_string_lossy().to_lowercase());
            let repositories = common_dirs
                .into_iter()
                .map(|common_dir| match git::list_worktrees(&common_dir) {
                    Ok(worktrees) => WorktreeRepository {
                        label: repository_label(&common_dir, &worktrees),
                        common_dir,
                        worktrees,
                        error: None,
                    },
                    Err(error) => WorktreeRepository {
                        label: repository_label(&common_dir, &[]),
                        common_dir,
                        worktrees: Vec::new(),
                        error: Some(error.to_string()),
                    },
                })
                .collect();
            let _ = sender.send(Completion::Inventory(InventoryCompletion {
                generation,
                repositories,
                discovered,
            }));
        });
    }

    pub(crate) fn start_remove(&mut self, common_dir: PathBuf, path: PathBuf) -> bool {
        if self.remove_running {
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
                    self.restore_selection(selected_path.as_deref());
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
        if self.remove_dialog.is_some() {
            return self.handle_remove_dialog(key);
        }
        match key.code {
            KeyCode::Esc => Some(WorktreeManagerEffect::Close),
            KeyCode::Enter => self.activate_selected(),
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
            rows.push(WorktreeManagerRow::Repository(repository_index));
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

    pub(crate) fn remove_running(&self) -> bool {
        self.remove_running
    }

    fn activate_selected(&self) -> Option<WorktreeManagerEffect> {
        if self.remove_running {
            return Some(WorktreeManagerEffect::Notice(
                "Wait for the worktree removal to finish".to_owned(),
            ));
        }
        let (_, worktree) = self.selected_worktree()?;
        if worktree.is_bare {
            return Some(WorktreeManagerEffect::Notice(
                "Bare repositories cannot be opened as workspaces".to_owned(),
            ));
        }
        if worktree.prunable {
            return Some(WorktreeManagerEffect::Notice(
                "This worktree is missing and can only be pruned".to_owned(),
            ));
        }
        Some(WorktreeManagerEffect::Open(worktree.path.clone()))
    }

    fn begin_remove(&mut self) -> Option<WorktreeManagerEffect> {
        if self.remove_running {
            return Some(WorktreeManagerEffect::Notice(
                "A worktree removal is already running".to_owned(),
            ));
        }
        if self.herdr_enabled && !self.herdr_verified {
            return Some(WorktreeManagerEffect::Notice(
                "Waiting for Herdr to verify linked worktree ownership".to_owned(),
            ));
        }
        let (repository, worktree) = self.selected_worktree()?;
        if worktree.is_main {
            return Some(WorktreeManagerEffect::Notice(
                "The primary worktree cannot be removed".to_owned(),
            ));
        }
        if worktree.locked {
            return Some(WorktreeManagerEffect::Notice(
                worktree.locked_reason.as_ref().map_or_else(
                    || "Unlock this worktree before removing it".to_owned(),
                    |reason| format!("Worktree is locked: {reason}"),
                ),
            ));
        }
        if worktree.prunable {
            return Some(WorktreeManagerEffect::Notice(
                "This missing worktree requires repository metadata pruning".to_owned(),
            ));
        }
        if self
            .current_path
            .as_deref()
            .is_some_and(|current| same_path(current, &worktree.path))
        {
            return Some(WorktreeManagerEffect::Notice(
                "Open another worktree before removing the current one".to_owned(),
            ));
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

    fn selected_worktree(&self) -> Option<(&WorktreeRepository, &LinkedWorktree)> {
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

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

struct KnownRepositoryStore {
    path: Option<PathBuf>,
    repositories: Vec<PathBuf>,
    load_error: Option<String>,
}

impl KnownRepositoryStore {
    fn new(path: Option<PathBuf>) -> Self {
        let (repositories, load_error) = match path.as_deref().map(load_known) {
            Some(Ok(repositories)) => (repositories, None),
            Some(Err(error)) => (Vec::new(), Some(error)),
            None => (Vec::new(), None),
        };
        Self {
            path,
            repositories,
            load_error,
        }
    }

    fn insert(&mut self, common_dir: PathBuf) -> bool {
        if self.repositories.iter().any(|known| known == &common_dir) {
            return false;
        }
        self.repositories.push(common_dir);
        self.repositories
            .sort_by_cached_key(|path| path.to_string_lossy().to_lowercase());
        true
    }

    fn extend(&mut self, repositories: Vec<PathBuf>) {
        for repository in repositories {
            self.insert(repository);
        }
    }

    fn extend_and_save(&mut self, repositories: Vec<PathBuf>) -> Result<(), String> {
        let previous = self.repositories.clone();
        self.extend(repositories);
        if self.repositories == previous {
            return Ok(());
        }
        if let Err(error) = self.save() {
            self.repositories = previous;
            return Err(error);
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        if let Some(error) = self.load_error.as_deref() {
            return Err(format!(
                "{error}; refusing to overwrite the unreadable repository inventory"
            ));
        }
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create Hunkle config directory: {error}"))?;
        }
        let repositories = self
            .repositories
            .iter()
            .map(|common_dir| known_repository_value(common_dir))
            .collect::<Vec<_>>();
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "repositories": repositories,
        }))
        .map_err(|error| format!("Could not serialize known repositories: {error}"))?;
        atomic_write(path, format!("{content}\n").as_bytes())
            .map_err(|error| format!("Could not save known repositories: {error}"))
    }
}

fn load_known(path: &Path) -> Result<Vec<PathBuf>, String> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read known repositories: {error}")),
    };
    let value = serde_json::from_slice::<Value>(&content)
        .map_err(|error| format!("Could not parse known repositories: {error}"))?;
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Known repositories use an unsupported version".to_owned());
    }
    value
        .get("repositories")
        .and_then(Value::as_array)
        .ok_or_else(|| "Known repositories have no repository list".to_owned())?
        .iter()
        .map(known_repository_path)
        .collect()
}

fn known_repository_value(path: &Path) -> Value {
    if let Some(path) = path.to_str() {
        return serde_json::json!({ "common_dir": path });
    }
    #[cfg(unix)]
    {
        serde_json::json!({ "common_dir_bytes": path.as_os_str().as_bytes() })
    }
    #[cfg(windows)]
    {
        serde_json::json!({
            "common_dir_wide": path.as_os_str().encode_wide().collect::<Vec<_>>()
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        serde_json::json!({ "common_dir": path.to_string_lossy() })
    }
}

fn known_repository_path(value: &Value) -> Result<PathBuf, String> {
    if let Some(path) = value.get("common_dir").and_then(Value::as_str) {
        return Ok(PathBuf::from(path));
    }
    #[cfg(unix)]
    if let Some(bytes) = value.get("common_dir_bytes").and_then(Value::as_array) {
        let bytes = bytes
            .iter()
            .map(|byte| {
                byte.as_u64()
                    .and_then(|byte| u8::try_from(byte).ok())
                    .ok_or_else(|| "Known repository path contains an invalid byte".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
    }
    #[cfg(windows)]
    if let Some(wide) = value.get("common_dir_wide").and_then(Value::as_array) {
        let wide = wide
            .iter()
            .map(|unit| {
                unit.as_u64()
                    .and_then(|unit| u16::try_from(unit).ok())
                    .ok_or_else(|| "Known repository path contains an invalid unit".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)));
    }
    Err("Known repository entry has no path".to_owned())
}

#[cfg(test)]
mod tests {
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
    fn filters_branch_and_path_while_preserving_repository_header() {
        let mut manager = manager();
        manager.query = "modal".to_owned();
        manager.select_first();

        assert_eq!(
            manager.rows(),
            [
                WorktreeManagerRow::Repository(0),
                WorktreeManagerRow::Worktree {
                    repository: 0,
                    worktree: 1,
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
        assert_eq!(manager.state.selected(), Some(1));
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
        let paths = vec![PathBuf::from("/another-repository")];

        assert!(manager.update_herdr_inventory(paths.clone(), Vec::new(), true));
        assert!(!manager.update_herdr_inventory(paths, Vec::new(), true));
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
