use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use unicode_segmentation::UnicodeSegmentation;

use super::fuzzy::{fuzzy_text_score, fuzzy_text_score_lower};

const MAX_COMPLETION_SCAN: usize = 5_000;
const MAX_PREVIEW_ENTRIES: usize = 200;
const MAX_SURROUNDING_SIBLINGS: usize = 200;

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
    Navigate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExplorerHitTarget {
    Overlay,
    Path,
    SurroundingsPane,
    Surrounding { generation: u64, index: usize },
    EntriesPane,
    Entry { generation: u64, index: usize },
    MatchesPane,
    Match { generation: u64, index: usize },
    PreviewPane,
    Preview { generation: u64, index: usize },
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
    directory_index: Vec<IndexedDirectory>,
    index_roots: Vec<PathBuf>,
    index_rx: Option<Receiver<Vec<IndexedDirectory>>>,
    browse_rx: Option<Receiver<Result<BrowseResult, String>>>,
    content_generation: u64,
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

pub(super) enum PickerCommand {
    None,
    Close,
    Quit,
    Open(PathBuf),
}

impl Explorer {
    pub(super) fn new(directory: PathBuf) -> Self {
        let index_roots = search_roots(&directory);
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
            directory_index: Vec::new(),
            index_roots,
            index_rx: None,
            browse_rx: None,
            content_generation: 0,
        };
        picker.reload();
        picker
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, can_close: bool) -> PickerCommand {
        if self.editing_path {
            match key.code {
                KeyCode::Esc => {
                    self.invalidate_targets();
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
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_active_selection(1);
                PickerCommand::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_active_selection(-1);
                PickerCommand::None
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                self.go_parent();
                PickerCommand::None
            }
            KeyCode::Enter => self.activate_active(true),
            KeyCode::Right | KeyCode::Char('l') => {
                if self.surroundings_focused {
                    self.surroundings_focused = false;
                    PickerCommand::None
                } else {
                    self.activate_selected(false)
                }
            }
            KeyCode::Char('p') => {
                self.begin_search(Some(""));
                PickerCommand::None
            }
            KeyCode::Char('/') => {
                self.begin_search(Some(std::path::MAIN_SEPARATOR_STR));
                PickerCommand::None
            }
            KeyCode::Char('~') => {
                if let Some(home) = home_directory() {
                    self.navigate(home);
                }
                PickerCommand::None
            }
            KeyCode::Char('r') => {
                self.reload();
                PickerCommand::None
            }
            KeyCode::Char('q') if !can_close => PickerCommand::Quit,
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
        if self.editing_path {
            self.path_input.insert_str(self.path_cursor, text);
            self.path_cursor =
                boundary_at_or_after(&self.path_input, self.path_cursor + text.len());
            self.refresh_matches();
        }
    }

    pub(super) fn activate_selected(&mut self, open_repositories: bool) -> PickerCommand {
        let Some(entry) = self.selected().cloned() else {
            if open_repositories && self.loading {
                return PickerCommand::Open(self.directory.clone());
            }
            return PickerCommand::None;
        };
        if open_repositories && entry.action == PickerAction::Navigate && entry.is_repo {
            return PickerCommand::Open(entry.path);
        }
        match entry.action {
            PickerAction::Navigate => {
                self.navigate(entry.path);
                PickerCommand::None
            }
            PickerAction::Open => PickerCommand::Open(entry.path),
        }
    }

    pub(super) fn confirm_path(&mut self) -> PickerCommand {
        let exact_input = self.input_path();
        let path = if self.path_input.ends_with(['/', '\\']) && exact_input.is_dir() {
            exact_input
        } else {
            self.selected_match_path()
        };
        if !path.is_dir() {
            self.error = Some(format!("Directory not found: {}", path.display()));
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
        self.directory_index.clear();
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
            self.directory_index = index;
            self.index_rx = None;
            self.searching = false;
            if self.editing_path {
                self.refresh_matches();
            }
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
        let index_roots = search_roots(&path);
        if self.index_roots != index_roots {
            self.directory_index.clear();
            self.index_rx = None;
            self.index_roots = index_roots;
            self.searching = false;
        }
        self.directory = path;
        self.set_path_input(display_search_path(&self.directory));
        self.surroundings_focused = false;
        self.reload_directory();
    }

    pub(super) fn activate_surrounding(&mut self, index: usize) {
        if let Some(path) = self.surroundings.get(index).map(|entry| entry.path.clone()) {
            self.navigate(path);
        }
    }

    pub(super) fn accept_preview(&mut self, index: usize) {
        let Some(path) = self
            .preview_entries
            .get(index)
            .map(|entry| entry.path.clone())
        else {
            return;
        };
        self.set_path_input(completion_path(&path));
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

    pub(super) fn activate_target(&mut self, target: ExplorerHitTarget) -> PickerCommand {
        match target {
            ExplorerHitTarget::Path => {
                self.begin_search(None);
                PickerCommand::None
            }
            ExplorerHitTarget::Surrounding { generation, index }
                if generation == self.content_generation =>
            {
                self.activate_surrounding(index);
                PickerCommand::None
            }
            ExplorerHitTarget::Entry { generation, index }
                if generation == self.content_generation && index < self.entries.len() =>
            {
                self.state.select(Some(index));
                self.activate_selected(true)
            }
            ExplorerHitTarget::Match { generation, index }
                if generation == self.content_generation && index < self.matches.len() =>
            {
                self.match_state.select(Some(index));
                self.confirm_path()
            }
            ExplorerHitTarget::Preview { generation, index }
                if generation == self.content_generation =>
            {
                self.accept_preview(index);
                PickerCommand::None
            }
            ExplorerHitTarget::Overlay
            | ExplorerHitTarget::SurroundingsPane
            | ExplorerHitTarget::EntriesPane
            | ExplorerHitTarget::MatchesPane
            | ExplorerHitTarget::PreviewPane
            | ExplorerHitTarget::Surrounding { .. }
            | ExplorerHitTarget::Entry { .. }
            | ExplorerHitTarget::Match { .. }
            | ExplorerHitTarget::Preview { .. } => PickerCommand::None,
        }
    }

    fn invalidate_targets(&mut self) {
        self.content_generation = self.content_generation.wrapping_add(1);
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
        let query = self.path_input.trim();
        if query.is_empty() {
            self.searching = false;
            self.matches.clear();
            self.preview_entries.clear();
            self.match_state.select(None);
            return;
        }
        if query.contains(['/', '\\']) || query.starts_with('~') {
            self.searching = false;
            self.matches = path_completion_candidates(query, &self.directory);
            self.select_match(selected_path.as_deref());
            self.refresh_preview();
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
        self.searching = self.directory_index.is_empty() && self.index_rx.is_some();

        let query_lower = query.to_lowercase();
        let mut candidates = Vec::with_capacity(12);
        let compare =
            |(left_score, left_depth, left_index): &(u32, usize, usize),
             (right_score, right_depth, right_index): &(u32, usize, usize)| {
                right_score
                    .cmp(left_score)
                    .then_with(|| left_depth.cmp(right_depth))
                    .then_with(|| {
                        self.directory_index[*left_index]
                            .path
                            .cmp(&self.directory_index[*right_index].path)
                    })
            };
        for (index, directory) in self.directory_index.iter().enumerate() {
            let Some(score) = fuzzy_text_score_lower(&query_lower, &directory.name_lower) else {
                continue;
            };
            let candidate = (
                score + if directory.is_repo { 750 } else { 0 },
                directory.depth,
                index,
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
        self.matches = candidates
            .into_iter()
            .map(|(_, _, index)| PickerEntry {
                label: display_search_path(&self.directory_index[index].path),
                is_repo: self.directory_index[index].is_repo,
                path: self.directory_index[index].path.clone(),
                action: PickerAction::Navigate,
            })
            .collect();
        self.select_match(selected_path.as_deref());
        self.refresh_preview();
    }

    fn accept_completion(&mut self) {
        let Some(path) = self
            .match_state
            .selected()
            .and_then(|index| self.matches.get(index))
            .map(|entry| entry.path.clone())
        else {
            return;
        };
        self.set_path_input(completion_path(&path));
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
            .map(|entry| load_child_directories(&entry.path, MAX_PREVIEW_ENTRIES))
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
    entries.extend(load_child_directories(directory, usize::MAX));
    let (surroundings, selected_surrounding) = load_surroundings(directory);
    Ok(BrowseResult {
        entries,
        surroundings,
        selected_surrounding,
    })
}

fn load_child_directories(directory: &Path, limit: usize) -> Vec<PickerEntry> {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut directories: Vec<_> = read_dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            (file_type.is_dir() || file_type.is_symlink()).then_some(entry)
        })
        .filter(|entry| entry.file_name() != ".git")
        .take(limit)
        .map(|entry| {
            let path = entry.path();
            let is_repo = path.join(".git").exists();
            PickerEntry {
                label: format!("{}/", entry.file_name().to_string_lossy()),
                path,
                action: PickerAction::Navigate,
                is_repo,
            }
        })
        .collect();
    directories.sort_by_cached_key(|entry| entry.label.to_lowercase());
    directories
}

fn load_surroundings(directory: &Path) -> (Vec<SurroundingEntry>, Option<usize>) {
    let mut surroundings = Vec::new();
    let mut ancestors: Vec<_> = directory.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();
    ancestors.pop();
    for (depth, path) in ancestors.into_iter().enumerate() {
        surroundings.push(SurroundingEntry {
            label: path_label(&path),
            path,
            depth,
            current: false,
        });
    }

    let sibling_depth = surroundings.len();
    let mut siblings = directory
        .parent()
        .map(|parent| load_child_directories(parent, MAX_SURROUNDING_SIBLINGS))
        .unwrap_or_default();
    if !siblings.iter().any(|entry| entry.path == directory) {
        siblings.push(PickerEntry {
            label: path_label(directory),
            path: directory.to_path_buf(),
            action: PickerAction::Navigate,
            is_repo: is_repository_directory(directory),
        });
    }
    for sibling in siblings {
        surroundings.push(SurroundingEntry {
            label: sibling.label,
            current: sibling.path == directory,
            path: sibling.path,
            depth: sibling_depth,
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
    let mut candidates: Vec<_> = load_child_directories(&parent, MAX_COMPLETION_SCAN)
        .into_iter()
        .filter_map(|mut entry| {
            let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
            let score = if fragment_lower.is_empty() {
                0
            } else {
                fuzzy_text_score_lower(&fragment_lower, &name.to_lowercase())?
            };
            entry.label = display_search_path(&entry.path);
            Some((score, entry))
        })
        .collect();
    candidates.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates
        .into_iter()
        .take(12)
        .map(|(_, entry)| entry)
        .collect()
}

fn completion_path(path: &Path) -> String {
    let mut path = display_search_path(path);
    if !path.ends_with(std::path::MAIN_SEPARATOR) {
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
        picker.directory_index = index_directories(&[root.to_path_buf()]);
        picker.begin_search(Some("hnk"));
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
        picker.directory_index = index;
        picker.begin_search(Some("opencode"));
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
        assert_eq!(picker.matches[0].path, config);
        assert!(
            picker
                .preview_entries
                .iter()
                .any(|entry| entry.path == opencode)
        );

        picker.accept_completion();
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
        picker.directory_index.push(IndexedDirectory {
            path: first.join("stale"),
            name_lower: "stale".to_owned(),
            depth: 1,
            is_repo: false,
        });
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
        picker.directory_index.push(IndexedDirectory {
            path: temp.path().join("stale"),
            name_lower: "stale".to_owned(),
            depth: 1,
            is_repo: false,
        });
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
        assert_eq!(picker.directory, child);

        let generation = picker.content_generation;
        assert!(matches!(
            picker.activate_target(target),
            PickerCommand::None
        ));
        assert_eq!(picker.content_generation, generation);
    }

    #[test]
    fn fuzzy_search_keeps_only_the_best_twelve_matches() {
        let mut picker = Explorer::new(PathBuf::from("/"));
        picker.directory_index = (0..30)
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
            .collect();

        picker.begin_search(Some("needle"));

        assert_eq!(picker.matches.len(), 12);
        assert_eq!(picker.matches[0].path, Path::new("/needle"));
    }
}
