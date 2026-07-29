use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    sync::{Arc, Mutex},
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
    directory_index: Arc<Vec<IndexedDirectory>>,
    index_roots: Vec<PathBuf>,
    index_rx: Option<Receiver<Vec<IndexedDirectory>>>,
    match_generation: u64,
    match_pending: Arc<Mutex<Option<MatchRequest>>>,
    match_wake: Option<SyncSender<()>>,
    match_rx: Receiver<MatchResult>,
    match_worker: Option<JoinHandle<()>>,
    browse_rx: Option<Receiver<Result<BrowseResult, String>>>,
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

#[derive(Debug, Clone)]
struct IndexedDirectory {
    path: PathBuf,
    name_lower: String,
    depth: usize,
    is_repo: bool,
}

#[derive(Debug)]
struct MatchRequest {
    generation: u64,
    query: String,
    directory: PathBuf,
    index: Arc<Vec<IndexedDirectory>>,
    selected_path: Option<PathBuf>,
}

#[derive(Debug)]
struct MatchResult {
    generation: u64,
    matches: Vec<PickerEntry>,
    selected_path: Option<PathBuf>,
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
            directory_index: Arc::new(Vec::new()),
            index_roots,
            index_rx: None,
            match_generation: 0,
            match_pending,
            match_wake: Some(match_wake),
            match_rx,
            match_worker: Some(match_worker),
            browse_rx: None,
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
        self.directory_index = Arc::new(Vec::new());
        self.index_rx = None;
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
        let directory = self.directory.clone();
        let (sender, receiver) = mpsc::channel();
        self.browse_rx = Some(receiver);
        thread::spawn(move || {
            let _ = sender.send(load_directory_entries(&directory));
        });
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
        if let Some(index) = self
            .index_rx
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.directory_index = Arc::new(index);
            self.index_rx = None;
            self.searching = false;
            if self.editing_path {
                self.refresh_matches();
            }
            changed = true;
        }
        while let Ok(result) = self.match_rx.try_recv() {
            if result.generation != self.match_generation {
                continue;
            }
            self.matches = result.matches;
            self.select_match(result.selected_path.as_deref());
            self.refresh_preview();
            self.searching = self.index_rx.is_some();
            changed = true;
        }
        if let Some(result) = self
            .browse_rx
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.invalidate_targets();
            self.browse_rx = None;
            self.loading = false;
            match result {
                Ok(result) => {
                    self.entries = result.entries;
                    self.surroundings = result.surroundings;
                    self.surroundings_state.select(result.selected_surrounding);
                    self.state.select((!self.entries.is_empty()).then_some(0));
                }
                Err(error) => self.error = Some(error),
            }
            changed = true;
        }
        changed
    }

    pub(super) fn navigate(&mut self, path: PathBuf) {
        self.cancel_match_search();
        let index_roots = search_roots(&path);
        if self.index_roots != index_roots {
            self.directory_index = Arc::new(Vec::new());
            self.index_rx = None;
            self.index_roots = index_roots;
            self.searching = false;
        }
        self.directory = path;
        self.set_path_input(display_search_path(&self.directory));
        self.surroundings_focused = false;
        self.reload_directory();
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
        if !query.contains(['/', '\\'])
            && self.directory_index.is_empty()
            && self.index_rx.is_none()
        {
            self.searching = true;
            let (sender, receiver) = mpsc::channel();
            self.index_rx = Some(receiver);
            let roots = self.index_roots.clone();
            thread::spawn(move || {
                let _ = sender.send(index_directories(&roots));
            });
        }
        if self.directory_index.is_empty() {
            self.searching = self.index_rx.is_some();
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

    fn cancel_match_search(&mut self) {
        self.match_generation = self.match_generation.wrapping_add(1);
        if let Ok(mut pending) = self.match_pending.lock() {
            *pending = None;
        }
        self.searching = false;
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
        self.preview_entries = self
            .match_state
            .selected()
            .and_then(|index| self.matches.get(index))
            .map(|entry| load_child_directories(&entry.path, MAX_PREVIEW_ENTRIES, true))
            .unwrap_or_default();
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
    }
}

impl Drop for Explorer {
    fn drop(&mut self) {
        self.shutdown();
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
    let Ok(read_dir) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for entry in read_dir.filter_map(Result::ok) {
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
    directories.sort_by_cached_key(|entry| entry.label.to_lowercase());
    files.sort_by_cached_key(|entry| entry.label.to_lowercase());
    directories.extend(files);
    directories.truncate(limit);
    directories
}

fn indexed_directory_matches(query: &str, index: &[IndexedDirectory]) -> Vec<PickerEntry> {
    let query_lower = query.to_lowercase();
    let mut candidates = Vec::with_capacity(12);
    let compare =
        |(left_score, left_depth, left_index): &(u32, usize, usize),
         (right_score, right_depth, right_index): &(u32, usize, usize)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_depth.cmp(right_depth))
                .then_with(|| index[*left_index].path.cmp(&index[*right_index].path))
        };
    for (directory_index, directory) in index.iter().enumerate() {
        let Some(score) = fuzzy_text_score_lower(&query_lower, &directory.name_lower) else {
            continue;
        };
        let candidate = (
            score + if directory.is_repo { 750 } else { 0 },
            directory.depth,
            directory_index,
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
        .map(|(_, _, directory_index)| PickerEntry {
            label: display_search_path(&index[directory_index].path),
            is_repo: index[directory_index].is_repo,
            path: index[directory_index].path.clone(),
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

fn index_directories(roots: &[PathBuf]) -> Vec<IndexedDirectory> {
    const MAX_DIRECTORIES: usize = 25_000;
    const MAX_DEPTH: usize = 7;
    let mut directories = Vec::new();
    let mut queue: VecDeque<_> = roots.iter().cloned().map(|path| (path, 0)).collect();
    let mut seen = HashSet::new();
    while let Some((directory, depth)) = queue.pop_front() {
        if directories.len() >= MAX_DIRECTORIES || !seen.insert(directory.clone()) {
            continue;
        }
        directories.push(IndexedDirectory {
            name_lower: directory
                .file_name()
                .unwrap_or_else(|| directory.as_os_str())
                .to_string_lossy()
                .to_lowercase(),
            depth: path_depth(&directory),
            is_repo: is_repository_directory(&directory)
                || is_bare_repository_directory(&directory),
            path: directory.clone(),
        });
        if depth >= MAX_DEPTH || is_bare_repository_directory(&directory) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
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
            queue.push_back((entry.path(), depth + 1));
        }
    }
    directories
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
mod tests {
    use super::*;

    #[test]
    fn shutdown_joins_the_match_worker_once() {
        let directory = tempfile::tempdir().unwrap();
        let mut explorer = Explorer::new(directory.path().to_path_buf());
        explorer.shutdown();
        explorer.shutdown();
        assert!(explorer.match_worker.is_none());
        assert!(explorer.match_wake.is_none());
    }

    fn wait_for_matches(picker: &mut Explorer) {
        for _ in 0..100 {
            picker.poll_index();
            if !picker.searching {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("Explorer search did not finish");
    }

    fn wait_for_browse(picker: &mut Explorer) {
        for _ in 0..100 {
            picker.poll_index();
            if !picker.loading {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("Explorer browse did not finish");
    }

    #[test]
    fn fuzzy_repository_paths_resolve_and_complete() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let code = root.join("code");
        let hunkle = code.join("hunkle");
        let gitlab = code.join("gitlab-runner");
        fs::create_dir_all(hunkle.join(".git")).unwrap();
        fs::create_dir_all(&gitlab).unwrap();

        assert_eq!(resolve_fuzzy_path("cod/hunk", root), Some(hunkle.clone()));

        let mut picker = Explorer::new(root.to_path_buf());
        picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
        picker.begin_search(Some("hnk"));
        wait_for_matches(&mut picker);
        assert_eq!(picker.matches[0].path, hunkle);
        assert!(picker.matches[0].is_repo);
        assert!(fuzzy_text_score("hunkle", "go-genai-streamed-function-args").is_none());

        let completed = picker.matches[0].path.clone();
        picker.accept_completion();
        assert_eq!(PathBuf::from(&picker.path_input), completed);
        assert!(picker.path_input.ends_with(std::path::MAIN_SEPARATOR));
    }

    #[test]
    fn directory_index_skips_build_trees() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("projects/hunkle")).unwrap();
        fs::create_dir_all(root.join("target/debug/deps")).unwrap();
        fs::create_dir_all(root.join("archive.git/objects/pack")).unwrap();
        fs::create_dir_all(root.join("archive.git/refs")).unwrap();
        fs::write(root.join("archive.git/HEAD"), "ref: refs/heads/main\n").unwrap();

        let index = index_directories(&[root.to_path_buf()]);
        let paths: Vec<_> = index.iter().map(|entry| &entry.path).collect();
        assert!(paths.contains(&&root.join("projects/hunkle")));
        assert!(!paths.contains(&&root.join("target")));
        assert!(paths.contains(&&root.join("archive.git")));
        assert!(!paths.contains(&&root.join("archive.git/objects")));
    }

    #[test]
    fn includes_config_directories_in_browsing_and_global_search() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let opencode = root.join(".config/opencode");
        fs::create_dir_all(opencode.join("themes")).unwrap();
        fs::create_dir_all(root.join(".cache/ignored")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();

        let browse = load_directory_entries(root).unwrap();
        assert!(
            browse
                .entries
                .iter()
                .any(|entry| entry.path == root.join(".config"))
        );
        assert!(
            !browse
                .entries
                .iter()
                .any(|entry| entry.path == root.join(".git"))
        );

        let index = index_directories(&[root.to_path_buf()]);
        let paths: Vec<_> = index.iter().map(|entry| &entry.path).collect();
        assert!(paths.contains(&&opencode));
        assert!(!paths.contains(&&root.join(".cache")));

        let mut picker = Explorer::new(root.to_path_buf());
        picker.directory_index = Arc::new(index);
        picker.begin_search(Some("opencode"));
        wait_for_matches(&mut picker);
        assert_eq!(picker.matches[0].path, opencode);
    }

    #[test]
    fn path_completion_adds_a_separator_and_immediately_lists_children() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let config = root.join(".config");
        let opencode = config.join("opencode");
        fs::create_dir_all(opencode.join("themes")).unwrap();
        fs::create_dir_all(config.join("other")).unwrap();

        let mut picker = Explorer::new(root.to_path_buf());
        picker.begin_search(Some(&format!("{}/.conf", root.display())));
        wait_for_matches(&mut picker);
        assert_eq!(picker.matches[0].path, config);
        assert!(
            picker
                .preview_entries
                .iter()
                .any(|entry| entry.path == opencode)
        );

        picker.accept_completion();
        wait_for_matches(&mut picker);
        assert!(
            picker
                .path_input
                .ends_with(&format!(".config{}", std::path::MAIN_SEPARATOR))
        );
        assert!(picker.matches.iter().any(|entry| entry.path == opencode));

        assert!(matches!(picker.confirm_path(), PickerCommand::None));
        assert_eq!(picker.directory, config);
    }

    #[test]
    fn edits_paths_at_the_cursor_and_deletes_previous_segments() {
        let temp = tempfile::tempdir().unwrap();
        let mut picker = Explorer::new(temp.path().to_path_buf());
        picker.begin_search(Some("~/projects/alpha/"));

        picker.handle_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
            true,
        );
        assert_eq!(picker.path_input, "~/projects/");
        picker.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT), true);
        assert_eq!(picker.path_input, "~/");

        picker.begin_search(Some("~/foo/bar"));
        picker.path_cursor = "~/foo".len();
        picker.handle_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
            true,
        );
        assert_eq!(picker.path_input, "~/bar");

        picker.begin_search(Some("/foo bar/"));
        picker.handle_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
            true,
        );
        assert_eq!(picker.path_input, "/");

        picker.begin_search(Some("cafe\u{301}"));
        picker.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), true);
        assert_eq!(picker.path_input, "caf");

        picker.begin_search(Some("👩👩"));
        picker.path_cursor = "👩".len();
        picker.handle_key(
            KeyEvent::new(KeyCode::Char('\u{200d}'), KeyModifiers::NONE),
            true,
        );
        assert_eq!(picker.path_cursor, picker.path_input.len());
        picker.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), true);
        assert!(picker.path_input.is_empty());

        picker.begin_search(Some("ac"));
        picker.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), true);
        picker.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE), true);
        assert_eq!(picker.path_input, "abc");
        picker.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), true);
        picker.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE), true);
        assert_eq!(picker.path_input, "bc");

        picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true);
        assert_eq!(picker.path_input, display_search_path(temp.path()));
        assert_eq!(picker.path_cursor, picker.path_input.len());
    }

    #[test]
    fn invalidates_fuzzy_index_when_roaming_to_another_root() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        let mut picker = Explorer::new(first.clone());
        picker.directory_index = Arc::new(vec![IndexedDirectory {
            path: first.join("stale"),
            name_lower: "stale".to_owned(),
            depth: 1,
            is_repo: false,
        }]);
        let (_, receiver) = mpsc::channel();
        picker.index_rx = Some(receiver);

        picker.navigate(second);

        assert!(picker.directory_index.is_empty());
        assert!(picker.index_rx.is_none());
    }

    #[test]
    fn explicit_reload_invalidates_the_fuzzy_index_for_the_same_root() {
        let temp = tempfile::tempdir().unwrap();
        let mut picker = Explorer::new(temp.path().to_path_buf());
        picker.directory_index = Arc::new(vec![IndexedDirectory {
            path: temp.path().join("stale"),
            name_lower: "stale".to_owned(),
            depth: 1,
            is_repo: false,
        }]);
        let (_, receiver) = mpsc::channel();
        picker.index_rx = Some(receiver);

        picker.reload();

        assert!(picker.directory_index.is_empty());
        assert!(picker.index_rx.is_none());
    }

    #[test]
    fn enter_opens_the_current_directory_while_its_rows_load() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("workspace");
        fs::create_dir(&directory).unwrap();
        let mut picker = Explorer::new(temp.path().to_path_buf());
        picker.navigate(directory.clone());

        let PickerCommand::Open(opened) = picker.activate_selected(true) else {
            panic!("Enter should open the directory being browsed");
        };
        assert_eq!(opened, directory);
    }

    #[test]
    fn semantic_targets_activate_exact_entries_and_reject_stale_rows() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        let mut picker = Explorer::new(temp.path().to_path_buf());
        picker.entries = vec![PickerEntry {
            label: "child/".to_owned(),
            path: child.clone(),
            action: PickerAction::Navigate,
            is_repo: false,
        }];
        picker.state.select(Some(0));
        let target = picker.entry_target(0);

        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, temp.path());
        assert_eq!(picker.state.selected(), Some(0));

        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, child);

        let generation = picker.content_generation;
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.content_generation, generation);
    }

    #[test]
    fn single_clicks_select_and_double_clicks_traverse_repository_folders() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let child = root.join("child");
        let grandchild = child.join("grandchild");
        fs::create_dir_all(&grandchild).unwrap();
        fs::create_dir(child.join(".git")).unwrap();
        fs::write(root.join("note.txt"), "x\n").unwrap();
        let mut picker = Explorer::new(root.to_path_buf());
        wait_for_browse(&mut picker);

        let child_row = picker
            .entries
            .iter()
            .position(|entry| entry.path == child)
            .unwrap();
        let child_target = picker.entry_target(child_row);
        assert!(matches!(
            picker.activate_target(child_target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, root);
        assert_eq!(picker.state.selected(), Some(child_row));

        picker.activate_target(child_target);
        assert_eq!(picker.directory, child);
        wait_for_browse(&mut picker);

        let grandchild_row = picker
            .entries
            .iter()
            .position(|entry| entry.path == grandchild)
            .unwrap();
        let parent_row = picker
            .entries
            .iter()
            .position(|entry| entry.label == "..")
            .unwrap();
        picker.activate_target(picker.entry_target(grandchild_row));
        assert_eq!(picker.directory, child);
        let parent_target = picker.entry_target(parent_row);
        picker.activate_target(parent_target);
        assert_eq!(picker.directory, child);
        picker.activate_target(parent_target);
        assert_eq!(picker.directory, root);
        wait_for_browse(&mut picker);

        let file = root.join("note.txt");
        let file_row = picker
            .entries
            .iter()
            .position(|entry| entry.path == file)
            .unwrap();
        let file_target = picker.entry_target(file_row);
        assert!(matches!(
            picker.activate_target(file_target),
            PickerCommand::None
        ));
        let PickerCommand::OpenFile(opened) = picker.activate_target(file_target) else {
            panic!("double-clicking a file entry should open it");
        };
        assert_eq!(opened, file);
    }

    #[test]
    fn single_click_on_a_match_previews_and_double_click_confirms() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let child = root.join("child");
        fs::create_dir_all(child.join("inside")).unwrap();
        let mut picker = Explorer::new(root.to_path_buf());
        picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
        picker.begin_search(Some("child"));
        wait_for_matches(&mut picker);

        let target = picker.match_target(0);
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, root);
        assert_eq!(picker.match_state.selected(), Some(0));
        assert!(
            picker
                .preview_entries
                .iter()
                .any(|entry| entry.path == child.join("inside"))
        );

        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, child);
    }

    #[test]
    fn surrounding_tree_can_navigate_up_and_back_down() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let spoon = home.join("spoon");
        let code = spoon.join("code");
        fs::create_dir_all(&code).unwrap();
        let mut picker = Explorer::new(code.clone());
        wait_for_browse(&mut picker);

        let home_row = picker
            .surroundings
            .iter()
            .position(|entry| entry.path == home)
            .unwrap();
        let target = picker.surrounding_target(home_row);
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, code);
        assert_eq!(picker.surroundings_state.selected(), Some(home_row));
        assert!(picker.surroundings_focused);
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, home);
        wait_for_browse(&mut picker);

        let spoon_row = picker
            .surroundings
            .iter()
            .position(|entry| entry.path == spoon)
            .expect("the current directory's child should remain in the left tree");
        let target = picker.surrounding_target(spoon_row);
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, spoon);
        wait_for_browse(&mut picker);
        assert!(picker.surroundings.iter().any(|entry| entry.path == code));
    }

    #[test]
    fn fuzzy_search_keeps_only_the_best_twelve_matches() {
        let mut picker = Explorer::new(PathBuf::from("/"));
        picker.directory_index = Arc::new(
            (0..30)
                .map(|index| {
                    let name = if index == 29 {
                        "needle".to_owned()
                    } else {
                        format!("needle-{index:02}")
                    };
                    IndexedDirectory {
                        path: PathBuf::from("/").join(&name),
                        name_lower: name,
                        depth: 1,
                        is_repo: false,
                    }
                })
                .collect(),
        );

        picker.begin_search(Some("needle"));
        wait_for_matches(&mut picker);

        assert_eq!(picker.matches.len(), 12);
        assert_eq!(picker.matches[0].path, Path::new("/needle"));
    }

    #[test]
    fn every_unmodified_character_starts_path_input_instead_of_a_browse_command() {
        let temp = tempfile::tempdir().unwrap();
        for character in ['h', 'j', 'k', 'l', 'p', 'q', 'r', '/', '~'] {
            let mut picker = Explorer::new(temp.path().to_path_buf());
            assert!(matches!(
                picker.handle_key(
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                    false,
                ),
                PickerCommand::None
            ));
            assert!(picker.editing_path);
            assert_eq!(picker.path_input, character.to_string());
            assert_eq!(picker.directory, temp.path());
        }
    }

    #[test]
    fn paste_starts_path_input_from_browse_mode() {
        let temp = tempfile::tempdir().unwrap();
        let mut picker = Explorer::new(temp.path().to_path_buf());

        picker.paste("~/shared");

        assert!(picker.editing_path);
        assert_eq!(picker.path_input, "~/shared");
    }

    #[test]
    fn favorites_persist_navigate_and_toggle_from_the_active_directory() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let favorites_path = temp.path().join("explorer-favorites.json");
        let mut picker = Explorer::with_favorites(first.clone(), Some(favorites_path.clone()));

        picker.handle_key(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            true,
        );
        assert!(picker.naming_favorite);
        for character in "Projects".chars() {
            picker.handle_key(
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                true,
            );
        }
        picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true);
        assert!(!picker.naming_favorite);
        assert_eq!(picker.favorites.len(), 1);
        assert_eq!(picker.favorites[0].name, "Projects");
        drop(picker);

        let mut picker = Explorer::with_favorites(second.clone(), Some(favorites_path.clone()));
        assert_eq!(picker.favorites.len(), 1);
        let target = picker.favorite_target(0);
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.directory, first);

        picker.handle_key(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            true,
        );
        assert!(picker.favorites.is_empty());
        drop(picker);

        let picker = Explorer::with_favorites(second, Some(favorites_path));
        assert!(picker.favorites.is_empty());
    }

    #[test]
    fn path_completion_finds_files_and_enter_opens_them() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let file = root.join("auth.json");
        fs::write(&file, "{}\n").unwrap();
        fs::create_dir_all(root.join("themes")).unwrap();

        let mut picker = Explorer::new(root.to_path_buf());
        picker.begin_search(Some(&format!("{}/au", root.display())));
        wait_for_matches(&mut picker);

        assert_eq!(picker.matches[0].path, file);
        assert_eq!(picker.matches[0].action, PickerAction::OpenFile);

        picker.accept_completion();
        assert!(picker.path_input.ends_with("auth.json"));
        assert!(!picker.path_input.ends_with(std::path::MAIN_SEPARATOR));

        let PickerCommand::OpenFile(opened) = picker.confirm_path() else {
            panic!("Enter should open the completed file");
        };
        assert_eq!(opened, root.join("auth.json"));
    }

    #[test]
    fn enter_opens_an_exact_file_in_the_current_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("auth.json"), "{}\n").unwrap();

        let mut picker = Explorer::new(root.to_path_buf());
        picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
        picker.begin_search(Some("auth.json"));
        wait_for_matches(&mut picker);

        let PickerCommand::OpenFile(opened) = picker.confirm_path() else {
            panic!("Enter should open the exact file path");
        };
        assert_eq!(opened, root.join("auth.json"));
    }

    #[test]
    fn enter_reports_paths_that_do_not_exist() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let mut picker = Explorer::new(root.to_path_buf());
        picker.directory_index = Arc::new(index_directories(&[root.to_path_buf()]));
        picker.begin_search(Some("missing.json"));
        wait_for_matches(&mut picker);

        assert!(matches!(picker.confirm_path(), PickerCommand::None));
        assert!(
            picker
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("Path not found: "))
        );
    }

    #[test]
    fn browsing_a_directory_lists_everything_directories_first() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for index in 0..15 {
            fs::write(root.join(format!("file-{index:02}.txt")), "x\n").unwrap();
        }
        for index in 0..5 {
            fs::create_dir_all(root.join(format!("dir-{index:02}"))).unwrap();
        }

        let mut picker = Explorer::new(root.to_path_buf());
        picker.begin_search(Some(&format!("{}/", root.display())));
        wait_for_matches(&mut picker);

        assert_eq!(picker.matches.len(), 20);
        assert!(
            picker
                .matches
                .iter()
                .take(5)
                .all(|entry| entry.action == PickerAction::Navigate)
        );
        assert!(
            picker
                .matches
                .iter()
                .skip(5)
                .all(|entry| entry.action == PickerAction::OpenFile)
        );
        assert_eq!(picker.matches[0].path, root.join("dir-00"));
        assert_eq!(picker.matches[5].path, root.join("file-00.txt"));
    }

    #[test]
    fn fuzzy_fragments_keep_only_the_best_twelve_completions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for index in 0..20 {
            fs::create_dir_all(root.join(format!("needle-{index:02}"))).unwrap();
        }

        let mut picker = Explorer::new(root.to_path_buf());
        picker.begin_search(Some(&format!("{}/need", root.display())));
        wait_for_matches(&mut picker);

        assert_eq!(picker.matches.len(), 12);
        assert!(
            picker
                .matches
                .iter()
                .all(|entry| entry.path.starts_with(root))
        );
    }

    #[test]
    fn browsing_lists_directories_before_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("zeta")).unwrap();
        fs::write(root.join("auth.json"), "{}\n").unwrap();

        let browse = load_directory_entries(root).unwrap();
        let directory = browse
            .entries
            .iter()
            .position(|entry| entry.path == root.join("zeta"))
            .unwrap();
        let file = browse
            .entries
            .iter()
            .position(|entry| entry.path == root.join("auth.json"))
            .unwrap();
        assert!(directory < file);
        assert_eq!(browse.entries[file].action, PickerAction::OpenFile);
        assert_eq!(browse.entries[file].label, "auth.json");

        let mut picker = Explorer::new(root.to_path_buf());
        picker.entries = browse.entries;
        picker.state.select(Some(file));
        let PickerCommand::OpenFile(opened) = picker.activate_selected(true) else {
            panic!("activating a file entry should open it");
        };
        assert_eq!(opened, root.join("auth.json"));
    }
}
