use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use unicode_segmentation::UnicodeSegmentation;

use crate::filesystem::same_path;

use super::fuzzy::{fuzzy_text_score, fuzzy_text_score_lower};

mod favorites;
pub(crate) use favorites::ExplorerFavorite;
use favorites::FavoriteStore;

const MAX_PREVIEW_ENTRIES: usize = 200;
const MAX_SURROUNDING_CHILDREN: usize = 200;
const INDEX_BATCH_SIZE: usize = 512;
const DOUBLE_CLICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);
pub(super) const MINIMUM_EXPLORER_PANE_WIDTH: u16 = 16;

#[derive(Debug, Clone)]
pub struct PickerEntry {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
    pub(crate) action: PickerAction,
    pub(crate) is_repo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Open,
    OpenFile,
    Navigate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExplorerHitTarget {
    Overlay,
    Path,
    Splitter,
    SurroundingsPane,
    Surrounding { generation: u64, index: usize },
    EntriesPane,
    Entry { generation: u64, index: usize },
    MatchesPane,
    Match { generation: u64, index: usize },
    PreviewPane,
    Preview { generation: u64, index: usize },
    Favorite { generation: u64, index: usize },
}

#[derive(Debug)]
pub struct Explorer {
    pub(crate) directory: PathBuf,
    pub(crate) path_input: String,
    pub(crate) path_cursor: usize,
    pub(crate) editing_path: bool,
    pub(crate) entries: Vec<PickerEntry>,
    pub(crate) state: ListState,
    pub(crate) matches: Vec<PickerEntry>,
    pub(crate) match_state: ListState,
    pub(crate) searching: bool,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) surroundings: Vec<SurroundingEntry>,
    pub(crate) surroundings_state: ListState,
    pub(crate) surroundings_focused: bool,
    pub(crate) preview_entries: Vec<PickerEntry>,
    directory_index: Arc<DirectoryIndex>,
    index_roots: Vec<PathBuf>,
    prewarmed_index_roots: Option<Vec<PathBuf>>,
    index_generation: u64,
    index_active_generation: Arc<AtomicU64>,
    pub(crate) index_loading: bool,
    index_pending: Arc<Mutex<Option<IndexRequest>>>,
    index_wake: Option<SyncSender<()>>,
    index_rx: Receiver<IndexCompletion>,
    match_generation: u64,
    match_pending: Arc<Mutex<Option<MatchRequest>>>,
    match_wake: Option<SyncSender<()>>,
    match_rx: Receiver<MatchResult>,
    match_worker: Option<JoinHandle<()>>,
    preview_generation: u64,
    preview_active_generation: Arc<AtomicU64>,
    pub(crate) preview_loading: bool,
    preview_directory: Option<PathBuf>,
    preview_pending: Arc<Mutex<Option<PreviewRequest>>>,
    preview_wake: Option<SyncSender<()>>,
    preview_rx: Receiver<PreviewCompletion>,
    browse_generation: u64,
    browse_pending: Arc<Mutex<Option<BrowseRequest>>>,
    browse_wake: Option<SyncSender<()>>,
    browse_rx: Receiver<BrowseCompletion>,
    content_generation: u64,
    last_row_click: Option<(PathBuf, Instant)>,
    pub(crate) left_pane_width: Option<u16>,
    pub(crate) dragging_splitter: bool,
    pub(crate) favorites: Vec<ExplorerFavorite>,
    pub(crate) favorite_name: String,
    pub(crate) naming_favorite: bool,
    favorite_store: FavoriteStore,
    favorite_generation: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct SurroundingEntry {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
    pub(crate) depth: usize,
    pub(crate) current: bool,
}

struct BrowseResult {
    entries: Vec<PickerEntry>,
    surroundings: Vec<SurroundingEntry>,
    selected_surrounding: Option<usize>,
}

#[derive(Debug)]
struct BrowseRequest {
    generation: u64,
    directory: PathBuf,
}

struct BrowseCompletion {
    generation: u64,
    result: Result<BrowseResult, String>,
}

#[derive(Debug, Clone)]
struct IndexedDirectory {
    path: PathBuf,
    name_lower: String,
    depth: usize,
    is_repo: bool,
}

#[derive(Debug, Clone, Default)]
struct DirectoryIndex {
    chunks: Vec<Arc<[IndexedDirectory]>>,
}

impl DirectoryIndex {
    fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    fn push(&mut self, entries: Vec<IndexedDirectory>) {
        self.chunks.push(entries.into());
    }

    fn iter(&self) -> impl Iterator<Item = &IndexedDirectory> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }
}

impl From<Vec<IndexedDirectory>> for DirectoryIndex {
    fn from(entries: Vec<IndexedDirectory>) -> Self {
        let mut index = Self::default();
        if !entries.is_empty() {
            index.push(entries);
        }
        index
    }
}

#[derive(Debug)]
struct IndexRequest {
    generation: u64,
    roots: Vec<PathBuf>,
}

struct IndexCompletion {
    generation: u64,
    entries: Vec<IndexedDirectory>,
    complete: bool,
}

#[derive(Debug)]
struct MatchRequest {
    generation: u64,
    query: String,
    directory: PathBuf,
    index: Arc<DirectoryIndex>,
    selected_path: Option<PathBuf>,
}

#[derive(Debug)]
struct MatchResult {
    generation: u64,
    matches: Vec<PickerEntry>,
    selected_path: Option<PathBuf>,
}

#[derive(Debug)]
struct PreviewRequest {
    generation: u64,
    directory: PathBuf,
}

struct PreviewCompletion {
    generation: u64,
    entries: Vec<PickerEntry>,
}

pub(super) enum PickerCommand {
    None,
    Close,
    Open(PathBuf),
    OpenFile(PathBuf),
}

impl Explorer {
    #[cfg(test)]
    pub(super) fn new(directory: PathBuf) -> Self {
        Self::with_favorites(directory, None)
    }

    pub(super) fn with_favorites(directory: PathBuf, favorites_path: Option<PathBuf>) -> Self {
        let (favorite_store, favorites) = FavoriteStore::new(favorites_path);
        let favorite_load_error = favorite_store.load_error().map(str::to_owned);
        let index_roots = search_roots(&directory);
        let match_pending = Arc::new(Mutex::new(None::<MatchRequest>));
        let worker_pending = Arc::clone(&match_pending);
        let (match_wake, wake_rx) = mpsc::sync_channel::<()>(1);
        let (match_tx, match_rx) = mpsc::channel();
        let match_worker = thread::spawn(move || {
            while wake_rx.recv().is_ok() {
                let Some(request) = worker_pending.lock().ok().and_then(|mut slot| slot.take())
                else {
                    continue;
                };
                let matches =
                    if request.query.contains(['/', '\\']) || request.query.starts_with('~') {
                        path_completion_candidates(&request.query, &request.directory)
                    } else {
                        indexed_directory_matches(&request.query, &request.index)
                    };
                let _ = match_tx.send(MatchResult {
                    generation: request.generation,
                    matches,
                    selected_path: request.selected_path,
                });
            }
        });
        let preview_pending = Arc::new(Mutex::new(None::<PreviewRequest>));
        let worker_pending = Arc::clone(&preview_pending);
        let preview_active_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&preview_active_generation);
        let (preview_wake, wake_rx) = mpsc::sync_channel::<()>(1);
        let (preview_tx, preview_rx) = mpsc::channel();
        thread::spawn(move || {
            run_preview_worker(worker_pending, worker_generation, wake_rx, preview_tx);
        });
        let index_pending = Arc::new(Mutex::new(None::<IndexRequest>));
        let worker_pending = Arc::clone(&index_pending);
        let index_active_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&index_active_generation);
        let (index_wake, wake_rx) = mpsc::sync_channel::<()>(1);
        let (index_tx, index_rx) = mpsc::channel();
        thread::spawn(move || {
            run_index_worker(
                worker_pending,
                worker_generation,
                wake_rx,
                index_tx,
                index_directories_progressively,
            );
        });
        let browse_pending = Arc::new(Mutex::new(None::<BrowseRequest>));
        let worker_pending = Arc::clone(&browse_pending);
        let (browse_wake, wake_rx) = mpsc::sync_channel::<()>(1);
        let (browse_tx, browse_rx) = mpsc::channel();
        thread::spawn(move || {
            run_browse_worker(worker_pending, wake_rx, browse_tx, load_directory_entries);
        });
        let mut picker = Self {
            path_input: display_search_path(&directory),
            path_cursor: display_search_path(&directory).len(),
            directory,
            editing_path: false,
            entries: Vec::new(),
            state: ListState::default(),
            matches: Vec::new(),
            match_state: ListState::default(),
            searching: false,
            loading: false,
            error: None,
            surroundings: Vec::new(),
            surroundings_state: ListState::default(),
            surroundings_focused: false,
            preview_entries: Vec::new(),
            directory_index: Arc::new(DirectoryIndex::default()),
            index_roots,
            prewarmed_index_roots: None,
            index_generation: 0,
            index_active_generation,
            index_loading: false,
            index_pending,
            index_wake: Some(index_wake),
            index_rx,
            match_generation: 0,
            match_pending,
            match_wake: Some(match_wake),
            match_rx,
            match_worker: Some(match_worker),
            preview_generation: 0,
            preview_active_generation,
            preview_loading: false,
            preview_directory: None,
            preview_pending,
            preview_wake: Some(preview_wake),
            preview_rx,
            browse_generation: 0,
            browse_pending,
            browse_wake: Some(browse_wake),
            browse_rx,
            content_generation: 0,
            last_row_click: None,
            left_pane_width: None,
            dragging_splitter: false,
            favorites,
            favorite_name: String::new(),
            naming_favorite: false,
            favorite_store,
            favorite_generation: 0,
        };
        picker.reload();
        picker.error = favorite_load_error;
        picker
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, can_close: bool) -> PickerCommand {
        if key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.toggle_current_favorite();
            return PickerCommand::None;
        }
        if self.naming_favorite {
            match key.code {
                KeyCode::Esc => self.cancel_favorite_name(),
                KeyCode::Enter => self.save_favorite_name(),
                KeyCode::Backspace => {
                    let mut cursor = self.favorite_name.len();
                    delete_previous_character(&mut self.favorite_name, &mut cursor);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.favorite_name.clear();
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.favorite_name.push(character);
                    self.error = None;
                }
                _ => {}
            }
            return PickerCommand::None;
        }
        if self.editing_path {
            match key.code {
                KeyCode::Esc => {
                    self.invalidate_targets();
                    self.cancel_match_search();
                    self.editing_path = false;
                    self.set_path_input(display_search_path(&self.directory));
                    self.matches.clear();
                    self.preview_entries.clear();
                    self.error = None;
                }
                KeyCode::Enter => return self.confirm_path(),
                KeyCode::Tab => self.accept_completion(),
                KeyCode::Down => self.move_match_selection(1),
                KeyCode::Up => self.move_match_selection(-1),
                KeyCode::Backspace
                    if key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    delete_previous_path_segment(&mut self.path_input, &mut self.path_cursor);
                    self.refresh_matches();
                }
                KeyCode::Backspace => {
                    delete_previous_character(&mut self.path_input, &mut self.path_cursor);
                    self.refresh_matches();
                }
                KeyCode::Delete => {
                    delete_next_character(&mut self.path_input, self.path_cursor);
                    self.refresh_matches();
                }
                KeyCode::Left => {
                    self.path_cursor = previous_boundary(&self.path_input, self.path_cursor);
                }
                KeyCode::Right => {
                    self.path_cursor = next_boundary(&self.path_input, self.path_cursor);
                }
                KeyCode::Home => self.path_cursor = 0,
                KeyCode::End => self.path_cursor = self.path_input.len(),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.set_path_input(String::new());
                    self.refresh_matches();
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.path_input.insert(self.path_cursor, character);
                    self.path_cursor = boundary_at_or_after(
                        &self.path_input,
                        self.path_cursor + character.len_utf8(),
                    );
                    self.refresh_matches();
                }
                _ => {}
            }
            return PickerCommand::None;
        }
        match key.code {
            KeyCode::Esc if can_close => PickerCommand::Close,
            KeyCode::Tab | KeyCode::BackTab => {
                self.surroundings_focused = !self.surroundings_focused;
                PickerCommand::None
            }
            KeyCode::Down => {
                self.move_active_selection(1);
                PickerCommand::None
            }
            KeyCode::Up => {
                self.move_active_selection(-1);
                PickerCommand::None
            }
            KeyCode::Backspace | KeyCode::Left => {
                self.go_parent();
                PickerCommand::None
            }
            KeyCode::Enter => self.activate_active(true),
            KeyCode::Right => {
                if self.surroundings_focused {
                    self.surroundings_focused = false;
                    PickerCommand::None
                } else {
                    self.activate_selected(false)
                }
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.begin_search(Some(&character.to_string()));
                PickerCommand::None
            }
            _ => PickerCommand::None,
        }
    }

    pub(super) fn paste(&mut self, text: &str) {
        if self.naming_favorite {
            self.favorite_name
                .extend(text.chars().filter(|character| !character.is_control()));
            self.error = None;
            return;
        }
        if !self.editing_path {
            self.begin_search(Some(""));
        }
        self.path_input.insert_str(self.path_cursor, text);
        self.path_cursor = boundary_at_or_after(&self.path_input, self.path_cursor + text.len());
        self.refresh_matches();
    }

    pub(super) fn activate_selected(&mut self, open_repositories: bool) -> PickerCommand {
        let Some(entry) = self.selected().cloned() else {
            if open_repositories && self.loading {
                return PickerCommand::Open(self.directory.clone());
            }
            return PickerCommand::None;
        };
        match entry.action {
            PickerAction::Navigate => {
                self.navigate(entry.path);
                PickerCommand::None
            }
            PickerAction::Open => PickerCommand::Open(entry.path),
            PickerAction::OpenFile => PickerCommand::OpenFile(entry.path),
        }
    }

    pub(super) fn confirm_path(&mut self) -> PickerCommand {
        let exact_input = self.input_path();
        if exact_input.is_file() {
            return PickerCommand::OpenFile(exact_input);
        }
        let path = if self.path_input.ends_with(['/', '\\']) && exact_input.is_dir() {
            exact_input
        } else {
            self.selected_match_path()
        };
        if path.is_file() {
            return PickerCommand::OpenFile(path);
        }
        if !path.is_dir() {
            self.error = Some(format!("Path not found: {}", path.display()));
            return PickerCommand::None;
        }
        if is_repository_directory(&path) {
            PickerCommand::Open(path)
        } else {
            self.navigate(path);
            self.editing_path = false;
            self.matches.clear();
            PickerCommand::None
        }
    }

    pub(super) fn reload(&mut self) {
        self.cancel_match_search();
        self.directory_index = Arc::new(DirectoryIndex::default());
        self.cancel_index();
        self.searching = false;
        self.reload_directory();
    }

    fn reload_directory(&mut self) {
        self.invalidate_targets();
        self.error = None;
        self.loading = true;
        self.entries.clear();
        self.state.select(None);
        self.surroundings.clear();
        self.surroundings_state.select(None);
        self.browse_generation = self.browse_generation.wrapping_add(1);
        let request = BrowseRequest {
            generation: self.browse_generation,
            directory: self.directory.clone(),
        };
        if let Ok(mut pending) = self.browse_pending.lock() {
            *pending = Some(request);
            if let Some(wake) = &self.browse_wake {
                let _ = wake.try_send(());
            }
        }
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        move_list(&mut self.state, self.entries.len(), delta);
    }

    pub(super) fn move_surrounding_selection(&mut self, delta: isize) {
        move_list(&mut self.surroundings_state, self.surroundings.len(), delta);
    }

    fn move_active_selection(&mut self, delta: isize) {
        if self.surroundings_focused {
            self.move_surrounding_selection(delta);
        } else {
            self.move_selection(delta);
        }
    }

    pub(super) fn begin_search(&mut self, initial: Option<&str>) {
        self.editing_path = true;
        self.error = None;
        if let Some(initial) = initial {
            self.set_path_input(initial.to_owned());
        } else {
            self.path_cursor = self.path_input.len();
        }
        self.refresh_matches();
    }

    pub(super) fn poll_index(&mut self) -> bool {
        let mut changed = false;
        while let Ok(completion) = self.index_rx.try_recv() {
            changed |= self.apply_index_completion(completion);
        }
        while let Ok(result) = self.match_rx.try_recv() {
            if result.generation != self.match_generation {
                continue;
            }
            self.matches = result.matches;
            self.select_match(result.selected_path.as_deref());
            self.refresh_preview();
            self.searching = self.index_loading;
            changed = true;
        }
        while let Ok(completion) = self.preview_rx.try_recv() {
            if completion.generation != self.preview_generation {
                continue;
            }
            self.preview_entries = completion.entries;
            self.preview_loading = false;
            changed = true;
        }
        while let Ok(completion) = self.browse_rx.try_recv() {
            changed |= self.apply_browse_completion(completion);
        }
        changed
    }

    pub(super) fn navigate(&mut self, path: PathBuf) {
        self.cancel_match_search();
        let index_roots = search_roots(&path);
        if self.index_roots != index_roots {
            self.directory_index = Arc::new(DirectoryIndex::default());
            self.cancel_index();
            self.index_roots = index_roots;
            self.searching = false;
        }
        self.directory = path;
        self.set_path_input(display_search_path(&self.directory));
        self.surroundings_focused = false;
        self.reload_directory();
    }

    #[cfg(not(test))]
    pub(super) fn prewarm_index(&mut self, directory: &Path) {
        self.prewarm_index_roots(search_roots(directory));
    }

    fn prewarm_index_roots(&mut self, roots: Vec<PathBuf>) {
        if self.prewarmed_index_roots.as_ref() == Some(&roots) {
            return;
        }
        self.prewarmed_index_roots = Some(roots.clone());
        if self.index_roots != roots {
            self.cancel_match_search();
            self.directory_index = Arc::new(DirectoryIndex::default());
            self.cancel_index();
            self.index_roots = roots;
        }
        self.start_index();
    }

    pub(super) fn accept_preview(&mut self, index: usize) {
        let Some(entry) = self.preview_entries.get(index).cloned() else {
            return;
        };
        self.set_path_input(completion_path(
            &entry.path,
            entry.action != PickerAction::OpenFile,
        ));
        self.refresh_matches();
    }

    pub(crate) fn surrounding_target(&self, index: usize) -> ExplorerHitTarget {
        ExplorerHitTarget::Surrounding {
            generation: self.content_generation,
            index,
        }
    }

    pub(crate) fn entry_target(&self, index: usize) -> ExplorerHitTarget {
        ExplorerHitTarget::Entry {
            generation: self.content_generation,
            index,
        }
    }

    pub(crate) fn match_target(&self, index: usize) -> ExplorerHitTarget {
        ExplorerHitTarget::Match {
            generation: self.content_generation,
            index,
        }
    }

    pub(crate) fn preview_target(&self, index: usize) -> ExplorerHitTarget {
        ExplorerHitTarget::Preview {
            generation: self.content_generation,
            index,
        }
    }

    pub(crate) fn favorite_target(&self, index: usize) -> ExplorerHitTarget {
        ExplorerHitTarget::Favorite {
            generation: self.favorite_generation,
            index,
        }
    }

    pub(crate) fn favorite_is_current(&self, index: usize) -> bool {
        self.favorites
            .get(index)
            .is_some_and(|favorite| favorite.path == self.favorite_directory())
    }

    pub(super) fn activate_target(&mut self, target: ExplorerHitTarget) -> PickerCommand {
        match target {
            ExplorerHitTarget::Path => {
                self.begin_search(None);
                PickerCommand::None
            }
            ExplorerHitTarget::Surrounding { generation, index }
                if generation == self.content_generation && index < self.surroundings.len() =>
            {
                self.surroundings_focused = true;
                self.surroundings_state.select(Some(index));
                let path = self.surroundings[index].path.clone();
                if self.register_row_click(path.clone()) {
                    self.navigate(path);
                }
                PickerCommand::None
            }
            ExplorerHitTarget::Entry { generation, index }
                if generation == self.content_generation && index < self.entries.len() =>
            {
                self.surroundings_focused = false;
                self.state.select(Some(index));
                let entry = self.entries[index].clone();
                if self.register_row_click(entry.path.clone()) {
                    return self.activate_selected(true);
                }
                PickerCommand::None
            }
            ExplorerHitTarget::Match { generation, index }
                if generation == self.content_generation && index < self.matches.len() =>
            {
                self.match_state.select(Some(index));
                self.refresh_preview();
                let path = self.matches[index].path.clone();
                if self.register_row_click(path) {
                    return self.confirm_path();
                }
                PickerCommand::None
            }
            ExplorerHitTarget::Preview { generation, index }
                if generation == self.content_generation =>
            {
                self.accept_preview(index);
                PickerCommand::None
            }
            ExplorerHitTarget::Favorite { generation, index }
                if generation == self.favorite_generation && index < self.favorites.len() =>
            {
                let path = self.favorites[index].path.clone();
                if !path.is_dir() {
                    self.error = Some(format!("Favorite path not found: {}", path.display()));
                    return PickerCommand::None;
                }
                self.editing_path = false;
                self.naming_favorite = false;
                self.matches.clear();
                self.preview_entries.clear();
                self.navigate(path);
                PickerCommand::None
            }
            ExplorerHitTarget::Overlay
            | ExplorerHitTarget::Splitter
            | ExplorerHitTarget::SurroundingsPane
            | ExplorerHitTarget::EntriesPane
            | ExplorerHitTarget::MatchesPane
            | ExplorerHitTarget::PreviewPane
            | ExplorerHitTarget::Surrounding { .. }
            | ExplorerHitTarget::Entry { .. }
            | ExplorerHitTarget::Match { .. }
            | ExplorerHitTarget::Preview { .. }
            | ExplorerHitTarget::Favorite { .. } => PickerCommand::None,
        }
    }

    pub(crate) fn pane_width(&self, total_width: u16) -> u16 {
        let available = total_width.saturating_sub(2);
        let minimum = MINIMUM_EXPLORER_PANE_WIDTH.min(available / 2);
        let maximum = available.saturating_sub(minimum);
        self.left_pane_width
            .unwrap_or_else(|| total_width.saturating_mul(38) / 100)
            .clamp(minimum, maximum)
    }

    pub(crate) fn resize_panes(&mut self, column: u16, start: u16, total_width: u16) {
        self.left_pane_width =
            Some(column.saturating_sub(start).saturating_sub(1).clamp(
                MINIMUM_EXPLORER_PANE_WIDTH.min(total_width.saturating_sub(2) / 2),
                total_width.saturating_sub(2).saturating_sub(
                    MINIMUM_EXPLORER_PANE_WIDTH.min(total_width.saturating_sub(2) / 2),
                ),
            ));
    }

    fn register_row_click(&mut self, path: PathBuf) -> bool {
        let double_click = self.last_row_click.as_ref().is_some_and(|(previous, at)| {
            *previous == path && at.elapsed() <= DOUBLE_CLICK_INTERVAL
        });
        self.last_row_click = (!double_click).then(|| (path, Instant::now()));
        double_click
    }

    fn invalidate_targets(&mut self) {
        self.content_generation = self.content_generation.wrapping_add(1);
    }

    fn apply_index_completion(&mut self, completion: IndexCompletion) -> bool {
        if completion.generation != self.index_generation {
            return false;
        }
        let changed = !completion.entries.is_empty();
        if changed {
            Arc::make_mut(&mut self.directory_index).push(completion.entries);
        }
        if completion.complete {
            self.index_loading = false;
        }
        let query = self.path_input.trim();
        let uses_index =
            !query.is_empty() && !query.contains(['/', '\\']) && !query.starts_with('~');
        if self.editing_path && uses_index && (changed || completion.complete) {
            self.refresh_matches();
        }
        changed || completion.complete
    }

    fn apply_browse_completion(&mut self, completion: BrowseCompletion) -> bool {
        if completion.generation != self.browse_generation {
            return false;
        }
        self.invalidate_targets();
        self.loading = false;
        match completion.result {
            Ok(result) => {
                self.entries = result.entries;
                self.surroundings = result.surroundings;
                self.surroundings_state.select(result.selected_surrounding);
                self.state.select((!self.entries.is_empty()).then_some(0));
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    fn favorite_directory(&self) -> PathBuf {
        if self.directory.is_absolute() {
            self.directory.clone()
        } else {
            std::env::current_dir()
                .map(|current| current.join(&self.directory))
                .unwrap_or_else(|_| self.directory.clone())
        }
    }

    fn toggle_current_favorite(&mut self) {
        if self.naming_favorite {
            self.cancel_favorite_name();
            return;
        }
        let path = self.favorite_directory();
        if let Some(index) = self
            .favorites
            .iter()
            .position(|favorite| same_path(&favorite.path, &path))
        {
            let previous = self.favorites.clone();
            self.favorites.remove(index);
            if let Err(error) = self.favorite_store.save(&self.favorites) {
                self.favorites = previous;
                self.error = Some(error);
                return;
            }
            self.favorite_generation = self.favorite_generation.wrapping_add(1);
            self.error = None;
            return;
        }
        self.cancel_match_search();
        self.editing_path = false;
        self.set_path_input(display_search_path(&self.directory));
        self.matches.clear();
        self.preview_entries.clear();
        self.favorite_name.clear();
        self.naming_favorite = true;
        self.error = None;
    }

    fn cancel_favorite_name(&mut self) {
        self.naming_favorite = false;
        self.favorite_name.clear();
        self.error = None;
    }

    fn save_favorite_name(&mut self) {
        let name = self.favorite_name.trim();
        if name.is_empty() {
            self.error = Some("Favorite name cannot be empty".to_owned());
            return;
        }
        let previous = self.favorites.clone();
        self.favorites.push(ExplorerFavorite {
            name: name.to_owned(),
            path: self.favorite_directory(),
        });
        if let Err(error) = self.favorite_store.save(&self.favorites) {
            self.favorites = previous;
            self.error = Some(error);
            return;
        }
        self.favorite_generation = self.favorite_generation.wrapping_add(1);
        self.favorite_name.clear();
        self.naming_favorite = false;
        self.error = None;
    }

    fn selected(&self) -> Option<&PickerEntry> {
        self.state
            .selected()
            .and_then(|index| self.entries.get(index))
    }

    pub(super) fn move_match_selection(&mut self, delta: isize) {
        move_list(&mut self.match_state, self.matches.len(), delta);
        self.refresh_preview();
    }

    fn activate_active(&mut self, open_repositories: bool) -> PickerCommand {
        if self.surroundings_focused {
            if let Some(path) = self
                .surroundings_state
                .selected()
                .and_then(|index| self.surroundings.get(index))
                .map(|entry| entry.path.clone())
            {
                self.navigate(path);
            }
            PickerCommand::None
        } else {
            self.activate_selected(open_repositories)
        }
    }

    fn refresh_matches(&mut self) {
        self.invalidate_targets();
        self.error = None;
        let selected_path = self
            .match_state
            .selected()
            .and_then(|index| self.matches.get(index))
            .map(|entry| entry.path.clone());
        let query = self.path_input.trim().to_owned();
        if query.is_empty() {
            self.cancel_match_search();
            self.matches.clear();
            self.preview_entries.clear();
            self.match_state.select(None);
            return;
        }
        if query.contains(['/', '\\']) || query.starts_with('~') {
            self.start_match_search(query, selected_path);
            return;
        }
        self.start_index();
        if self.directory_index.is_empty() {
            self.searching = self.index_loading;
            self.matches.clear();
            self.preview_entries.clear();
            self.match_state.select(None);
            return;
        }
        self.start_match_search(query, selected_path);
    }

    fn start_match_search(&mut self, query: String, selected_path: Option<PathBuf>) {
        self.match_generation = self.match_generation.wrapping_add(1);
        self.searching = true;
        let request = MatchRequest {
            generation: self.match_generation,
            query,
            directory: self.directory.clone(),
            index: Arc::clone(&self.directory_index),
            selected_path,
        };
        if let Ok(mut pending) = self.match_pending.lock() {
            *pending = Some(request);
            if let Some(match_wake) = &self.match_wake {
                let _ = match_wake.try_send(());
            }
        }
    }

    fn start_index(&mut self) {
        if !self.directory_index.is_empty() || self.index_loading {
            return;
        }
        self.index_loading = true;
        self.index_generation = self.index_generation.wrapping_add(1);
        self.index_active_generation
            .store(self.index_generation, Ordering::Relaxed);
        let request = IndexRequest {
            generation: self.index_generation,
            roots: self.index_roots.clone(),
        };
        if let Ok(mut pending) = self.index_pending.lock() {
            *pending = Some(request);
            if let Some(wake) = &self.index_wake {
                let _ = wake.try_send(());
            }
        }
    }

    fn cancel_match_search(&mut self) {
        self.match_generation = self.match_generation.wrapping_add(1);
        if let Ok(mut pending) = self.match_pending.lock() {
            *pending = None;
        }
        self.cancel_preview();
        self.searching = false;
    }

    fn cancel_preview(&mut self) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_active_generation
            .store(self.preview_generation, Ordering::Relaxed);
        self.preview_loading = false;
        self.preview_directory = None;
        if let Ok(mut pending) = self.preview_pending.lock() {
            *pending = None;
        }
    }

    fn cancel_index(&mut self) {
        self.index_generation = self.index_generation.wrapping_add(1);
        self.index_active_generation
            .store(self.index_generation, Ordering::Relaxed);
        self.index_loading = false;
        if let Ok(mut pending) = self.index_pending.lock() {
            *pending = None;
        }
    }

    fn accept_completion(&mut self) {
        let Some(entry) = self
            .match_state
            .selected()
            .and_then(|index| self.matches.get(index))
            .cloned()
        else {
            return;
        };
        self.set_path_input(completion_path(
            &entry.path,
            entry.action != PickerAction::OpenFile,
        ));
        self.refresh_matches();
    }

    fn select_match(&mut self, previous_path: Option<&Path>) {
        let selected = previous_path
            .and_then(|path| self.matches.iter().position(|entry| entry.path == path))
            .or((!self.matches.is_empty()).then_some(0));
        self.match_state.select(selected);
    }

    fn refresh_preview(&mut self) {
        let directory = self
            .match_state
            .selected()
            .and_then(|index| self.matches.get(index))
            .map(|entry| entry.path.clone());
        if self.preview_directory == directory {
            return;
        }
        self.cancel_preview();
        self.preview_entries.clear();
        let Some(directory) = directory else {
            return;
        };
        self.preview_loading = true;
        self.preview_directory = Some(directory.clone());
        let request = PreviewRequest {
            generation: self.preview_generation,
            directory,
        };
        if let Ok(mut pending) = self.preview_pending.lock() {
            *pending = Some(request);
            if let Some(wake) = &self.preview_wake {
                let _ = wake.try_send(());
            }
        }
    }

    fn selected_match_path(&self) -> PathBuf {
        self.match_state
            .selected()
            .and_then(|index| self.matches.get(index))
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| self.input_path())
    }

    fn go_parent(&mut self) {
        if let Some(parent) = self.directory.parent() {
            self.navigate(parent.to_path_buf());
        }
    }

    fn input_path(&self) -> PathBuf {
        let expanded = expand_search_path(self.path_input.trim());
        if expanded.is_absolute() {
            expanded
        } else {
            self.directory.join(expanded)
        }
    }

    fn set_path_input(&mut self, input: String) {
        self.path_input = input;
        self.path_cursor = self.path_input.len();
    }

    pub(super) fn shutdown(&mut self) {
        self.cancel_match_search();
        self.match_wake.take();
        if let Some(worker) = self.match_worker.take() {
            let _ = worker.join();
        }
        self.preview_wake.take();
        self.cancel_index();
        self.index_wake.take();
        self.browse_generation = self.browse_generation.wrapping_add(1);
        self.loading = false;
        if let Ok(mut pending) = self.browse_pending.lock() {
            *pending = None;
        }
        self.browse_wake.take();
    }
}

impl Drop for Explorer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_browse_worker(
    pending: Arc<Mutex<Option<BrowseRequest>>>,
    wake_rx: Receiver<()>,
    result_tx: Sender<BrowseCompletion>,
    load: impl Fn(&Path) -> Result<BrowseResult, String>,
) {
    while wake_rx.recv().is_ok() {
        let Some(request) = pending.lock().ok().and_then(|mut slot| slot.take()) else {
            continue;
        };
        let result = load(&request.directory);
        if result_tx
            .send(BrowseCompletion {
                generation: request.generation,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

fn run_preview_worker(
    pending: Arc<Mutex<Option<PreviewRequest>>>,
    active_generation: Arc<AtomicU64>,
    wake_rx: Receiver<()>,
    result_tx: Sender<PreviewCompletion>,
) {
    while wake_rx.recv().is_ok() {
        let Some(request) = pending.lock().ok().and_then(|mut slot| slot.take()) else {
            continue;
        };
        let cancelled = || active_generation.load(Ordering::Relaxed) != request.generation;
        let entries =
            load_child_directories_until(&request.directory, MAX_PREVIEW_ENTRIES, true, &cancelled);
        if cancelled() {
            continue;
        }
        if result_tx
            .send(PreviewCompletion {
                generation: request.generation,
                entries,
            })
            .is_err()
        {
            break;
        }
    }
}

fn run_index_worker(
    pending: Arc<Mutex<Option<IndexRequest>>>,
    active_generation: Arc<AtomicU64>,
    wake_rx: Receiver<()>,
    result_tx: Sender<IndexCompletion>,
    load: impl Fn(&[PathBuf], &mut dyn FnMut(Vec<IndexedDirectory>, bool) -> bool, &dyn Fn() -> bool),
) {
    while wake_rx.recv().is_ok() {
        let Some(request) = pending.lock().ok().and_then(|mut slot| slot.take()) else {
            continue;
        };
        let generation = request.generation;
        let cancelled = || active_generation.load(Ordering::Relaxed) != generation;
        let mut publish = |entries, complete| {
            if cancelled() {
                return false;
            }
            result_tx
                .send(IndexCompletion {
                    generation,
                    entries,
                    complete,
                })
                .is_ok()
        };
        load(&request.roots, &mut publish, &cancelled);
    }
}

fn delete_previous_path_segment(input: &mut String, cursor: &mut usize) {
    while *cursor > 0 {
        let previous = previous_boundary(input, *cursor);
        let character = input[previous..*cursor]
            .chars()
            .next()
            .expect("character boundary");
        if character != '/' && character != '\\' {
            break;
        }
        input.drain(previous..*cursor);
        *cursor = previous;
    }
    while *cursor > 0 {
        let previous = previous_boundary(input, *cursor);
        let character = input[previous..*cursor]
            .chars()
            .next()
            .expect("character boundary");
        if character == '/' || character == '\\' {
            break;
        }
        input.drain(previous..*cursor);
        *cursor = previous;
    }
    if *cursor < input.len() {
        let next = next_boundary(input, *cursor);
        let next_is_separator = input[*cursor..next]
            .chars()
            .next()
            .is_some_and(|character| character == '/' || character == '\\');
        let previous_is_separator = *cursor == 0
            || input[..*cursor]
                .chars()
                .next_back()
                .is_some_and(|character| character == '/' || character == '\\');
        if next_is_separator && previous_is_separator {
            input.drain(*cursor..next);
        }
    }
}

fn delete_previous_character(input: &mut String, cursor: &mut usize) {
    let previous = previous_boundary(input, *cursor);
    input.drain(previous..*cursor);
    *cursor = previous;
}

fn delete_next_character(input: &mut String, cursor: usize) {
    let next = next_boundary(input, cursor);
    input.drain(cursor..next);
}

fn previous_boundary(input: &str, cursor: usize) -> usize {
    input[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(input: &str, cursor: usize) -> usize {
    input[cursor..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(input.len(), |(index, _)| cursor + index)
}

fn boundary_at_or_after(input: &str, cursor: usize) -> usize {
    input
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(input.len()))
        .find(|index| *index >= cursor)
        .unwrap_or(input.len())
}

fn load_directory_entries(directory: &Path) -> Result<BrowseResult, String> {
    let current_is_repo = is_repository_directory(directory);
    let mut entries = vec![PickerEntry {
        label: if current_is_repo {
            "Open current repository".to_owned()
        } else {
            "Open current location".to_owned()
        },
        path: directory.to_path_buf(),
        action: PickerAction::Open,
        is_repo: current_is_repo,
    }];
    if let Some(parent) = directory.parent() {
        entries.push(PickerEntry {
            label: "..".to_owned(),
            path: parent.to_path_buf(),
            action: PickerAction::Navigate,
            is_repo: false,
        });
    }
    fs::read_dir(directory).map_err(|error| error.to_string())?;
    entries.extend(load_child_directories(directory, usize::MAX, true));
    let (surroundings, selected_surrounding) = load_surroundings(directory);
    Ok(BrowseResult {
        entries,
        surroundings,
        selected_surrounding,
    })
}

fn load_child_directories(directory: &Path, limit: usize, include_files: bool) -> Vec<PickerEntry> {
    load_child_directories_until(directory, limit, include_files, &|| false)
}

fn load_child_directories_until(
    directory: &Path,
    limit: usize,
    include_files: bool,
    cancelled: &dyn Fn() -> bool,
) -> Vec<PickerEntry> {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for entry in read_dir.filter_map(Result::ok) {
        if cancelled() {
            return Vec::new();
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if entry.file_name() == ".git" {
            continue;
        }
        let is_directory = file_type.is_dir() || file_type.is_symlink();
        let path = entry.path();
        if is_directory {
            let is_repo = path.join(".git").exists();
            directories.push(PickerEntry {
                label: format!("{}/", entry.file_name().to_string_lossy()),
                path,
                action: PickerAction::Navigate,
                is_repo,
            });
        } else if include_files && file_type.is_file() {
            files.push(PickerEntry {
                label: entry.file_name().to_string_lossy().into_owned(),
                path,
                action: PickerAction::OpenFile,
                is_repo: false,
            });
        }
    }
    if cancelled() {
        return Vec::new();
    }
    directories.sort_by_cached_key(|entry| entry.label.to_lowercase());
    files.sort_by_cached_key(|entry| entry.label.to_lowercase());
    directories.extend(files);
    directories.truncate(limit);
    directories
}

fn indexed_directory_matches(query: &str, index: &DirectoryIndex) -> Vec<PickerEntry> {
    let query_lower = query.to_lowercase();
    let mut candidates = Vec::with_capacity(12);
    let compare =
        |(left_score, left_depth, left): &(u32, usize, &IndexedDirectory),
         (right_score, right_depth, right): &(u32, usize, &IndexedDirectory)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_depth.cmp(right_depth))
                .then_with(|| left.path.cmp(&right.path))
        };
    for directory in index.iter() {
        let Some(score) = fuzzy_text_score_lower(&query_lower, &directory.name_lower) else {
            continue;
        };
        let candidate = (
            score + if directory.is_repo { 750 } else { 0 },
            directory.depth,
            directory,
        );
        if candidates.len() < 12 {
            candidates.push(candidate);
        } else if let Some((worst, _)) = candidates
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| compare(left, right))
            && compare(&candidate, &candidates[worst]).is_lt()
        {
            candidates[worst] = candidate;
        }
    }
    candidates.sort_by(compare);
    candidates
        .into_iter()
        .map(|(_, _, directory)| PickerEntry {
            label: display_search_path(&directory.path),
            is_repo: directory.is_repo,
            path: directory.path.clone(),
            action: PickerAction::Navigate,
        })
        .collect()
}

fn load_surroundings(directory: &Path) -> (Vec<SurroundingEntry>, Option<usize>) {
    let mut surroundings = Vec::new();
    let mut ancestors: Vec<_> = directory.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();
    for (depth, path) in ancestors.into_iter().enumerate() {
        surroundings.push(SurroundingEntry {
            label: path_label(&path),
            current: path == directory,
            path,
            depth,
        });
    }

    let child_depth = surroundings.len();
    for child in load_child_directories(directory, MAX_SURROUNDING_CHILDREN, false) {
        surroundings.push(SurroundingEntry {
            label: child.label,
            current: false,
            path: child.path,
            depth: child_depth,
        });
    }
    let selected = surroundings.iter().position(|entry| entry.current);
    (surroundings, selected)
}

fn search_roots(current: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home_directory() {
        roots.push(home);
    }
    if !roots.iter().any(|root| current.starts_with(root)) {
        roots.push(current.to_path_buf());
    }
    for path in ["/workspace", "/workspaces", "/projects", "/mnt", "/media"] {
        let path = PathBuf::from(path);
        if path.is_dir() {
            roots.push(path);
        }
    }
    roots
}

#[cfg(test)]
fn index_directories(roots: &[PathBuf]) -> Vec<IndexedDirectory> {
    let mut index = Vec::new();
    index_directories_progressively(
        roots,
        &mut |entries, _| {
            index.extend(entries);
            true
        },
        &|| false,
    );
    index
}

fn index_directories_progressively(
    roots: &[PathBuf],
    publish: &mut dyn FnMut(Vec<IndexedDirectory>, bool) -> bool,
    cancelled: &dyn Fn() -> bool,
) {
    const MAX_DIRECTORIES: usize = 25_000;
    const MAX_DEPTH: usize = 7;
    let mut directories = Vec::with_capacity(INDEX_BATCH_SIZE);
    let mut indexed = 0;
    let mut queue: VecDeque<_> = roots
        .iter()
        .cloned()
        .map(|path| {
            let absolute_depth = path_depth(&path);
            (path, 0, absolute_depth)
        })
        .collect();
    let mut seen = HashSet::new();
    while let Some((directory, depth, absolute_depth)) = queue.pop_front() {
        if cancelled() {
            return;
        }
        if indexed >= MAX_DIRECTORIES {
            break;
        }
        if !seen.insert(directory.clone()) {
            continue;
        }
        let is_bare_repo = is_bare_repository_directory(&directory);
        directories.push(IndexedDirectory {
            name_lower: directory
                .file_name()
                .unwrap_or_else(|| directory.as_os_str())
                .to_string_lossy()
                .to_lowercase(),
            depth: absolute_depth,
            is_repo: is_repository_directory(&directory) || is_bare_repo,
            path: directory.clone(),
        });
        indexed += 1;
        if directories.len() == INDEX_BATCH_SIZE
            && !publish(std::mem::take(&mut directories), false)
        {
            return;
        }
        if depth >= MAX_DEPTH || is_bare_repo {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if cancelled() {
                return;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if should_skip_index_directory(&name) {
                continue;
            }
            queue.push_back((entry.path(), depth + 1, absolute_depth + 1));
        }
    }
    let _ = publish(directories, true);
}

fn should_skip_index_directory(name: &str) -> bool {
    (name.starts_with('.') && name != ".config")
        || matches!(
            name,
            "node_modules" | "target" | "vendor" | "dist" | "build" | "__pycache__"
        )
}

fn expand_search_path(input: &str) -> PathBuf {
    if input == "~" {
        home_directory().unwrap_or_else(|| PathBuf::from(input))
    } else if let Some(rest) = input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("~\\"))
    {
        home_directory()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(input))
    } else {
        PathBuf::from(input)
    }
}

fn path_completion_candidates(input: &str, base: &Path) -> Vec<PickerEntry> {
    let expanded = expand_search_path(input);
    let trailing_separator = input.ends_with(['/', '\\']);
    let (parent, fragment) = if trailing_separator {
        (expanded, String::new())
    } else {
        (
            expanded.parent().map(Path::to_path_buf).unwrap_or_default(),
            expanded
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    };
    let parent = if parent.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        let parent_input = parent.to_string_lossy();
        let Some(parent) = resolve_fuzzy_path(&parent_input, base) else {
            return Vec::new();
        };
        parent
    };
    let fragment_lower = fragment.to_lowercase();
    let browsing = fragment_lower.is_empty();
    let compare = |(left_score, left): &(u32, PickerEntry),
                   (right_score, right): &(u32, PickerEntry)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.path.cmp(&right.path))
    };
    let mut candidates = Vec::new();
    let Ok(entries) = fs::read_dir(&parent) else {
        return Vec::new();
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.file_name() == ".git" {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let is_directory = kind.is_dir() || kind.is_symlink();
        if !is_directory && !kind.is_file() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        let Some(score) = (if browsing {
            Some(0)
        } else {
            fuzzy_text_score_lower(&fragment_lower, &name.to_string_lossy().to_lowercase())
        }) else {
            continue;
        };
        let candidate = (
            score,
            PickerEntry {
                label: display_search_path(&path),
                is_repo: is_directory && path.join(".git").exists(),
                path,
                action: if is_directory {
                    PickerAction::Navigate
                } else {
                    PickerAction::OpenFile
                },
            },
        );
        if browsing || candidates.len() < 12 {
            candidates.push(candidate);
        } else if let Some((worst, _)) = candidates
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| compare(left, right))
            && compare(&candidate, &candidates[worst]).is_lt()
        {
            candidates[worst] = candidate;
        }
    }
    if browsing {
        candidates.sort_by(|(_, left), (_, right)| {
            (left.action == PickerAction::OpenFile)
                .cmp(&(right.action == PickerAction::OpenFile))
                .then_with(|| left.path.cmp(&right.path))
        });
    } else {
        candidates.sort_by(compare);
    }
    candidates.into_iter().map(|(_, entry)| entry).collect()
}

fn completion_path(path: &Path, is_directory: bool) -> String {
    let mut path = display_search_path(path);
    if is_directory && !path.ends_with(std::path::MAIN_SEPARATOR) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    path
}

fn resolve_fuzzy_path(input: &str, base: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let expanded = expand_search_path(input);
    let mut resolved = if expanded.is_absolute() {
        PathBuf::new()
    } else {
        base.to_path_buf()
    };
    for component in expanded.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                let exact = resolved.join(name);
                if exact.is_dir() {
                    resolved = exact;
                    continue;
                }
                let query = name.to_string_lossy();
                let entries = fs::read_dir(&resolved).ok()?;
                let best = entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .filter_map(|entry| {
                        let score = fuzzy_text_score(&query, &entry.file_name().to_string_lossy())?;
                        Some((score, entry.path()))
                    })
                    .max_by(|(left_score, left), (right_score, right)| {
                        left_score.cmp(right_score).then_with(|| right.cmp(left))
                    })?;
                resolved = best.1;
            }
        }
    }
    resolved.is_dir().then_some(resolved)
}

fn is_repository_directory(path: &Path) -> bool {
    path.join(".git").exists()
}

fn is_bare_repository_directory(path: &Path) -> bool {
    path.join("HEAD").is_file() && path.join("objects").is_dir() && path.join("refs").is_dir()
}

fn display_search_path(path: &Path) -> String {
    if let Some(home) = home_directory()
        && let Ok(relative) = path.strip_prefix(home)
    {
        return if relative.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", relative.display())
        };
    }
    path.display().to_string()
}

fn path_label(path: &Path) -> String {
    path.file_name()
        .map(|name| format!("{}/", name.to_string_lossy()))
        .unwrap_or_else(|| path.display().to_string())
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn move_list(state: &mut ListState, len: usize, delta: isize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let current = state.selected().unwrap_or(0);
    let next = (current as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
    state.select(Some(next));
}

#[cfg(test)]
mod tests;
