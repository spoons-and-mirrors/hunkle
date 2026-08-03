use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossterm::event::KeyEvent;
use ratatui::widgets::ListState;
use regex::{Regex, RegexBuilder};

use crate::{
    git::{self, RepositoryData},
    repo_path::RepoPath,
};

use super::{EditOutcome, TextInput, fuzzy::fuzzy_text_score_lower};

const MAX_FILE_RESULTS: usize = 40;
const MAX_ALL_FILE_RESULTS: usize = 6;
const MAX_TEXT_RESULTS: usize = 500;
const MAX_SEARCH_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MATCH_CONTEXT_BYTES: usize = 512;
const SEARCH_BATCH_SIZE: usize = 64;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
struct IndexedFile {
    path: RepoPath,
    path_lower: String,
    name_start: usize,
    ignored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchScope {
    All,
    Files,
    Text,
}

impl SearchScope {
    pub(crate) const ALL: [Self; 3] = [Self::All, Self::Files, Self::Text];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Files => "FILES",
            Self::Text => "TEXT",
        }
    }

    fn shifted(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|scope| *scope == self)
            .unwrap_or(0);
        let next = index
            .saturating_add_signed(delta)
            .min(Self::ALL.len().saturating_sub(1));
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SearchOptions {
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
    pub(crate) include_ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextSearchMatch {
    pub(crate) path: RepoPath,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) before: String,
    pub(crate) matched: String,
    pub(crate) after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileSearchRow {
    Header { label: &'static str, count: usize },
    File { path: RepoPath },
    Text(TextSearchMatch),
    Status(String),
}

impl FileSearchRow {
    pub(crate) fn selectable(&self) -> bool {
        matches!(self, Self::File { .. } | Self::Text(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchDestination {
    File(RepoPath),
    Text { path: RepoPath, line: usize },
}

#[derive(Debug, Clone)]
struct FileMatch {
    path: RepoPath,
    score: u32,
}

pub(crate) struct FileSearch {
    pub(crate) query: TextInput,
    pub(crate) rows: Vec<FileSearchRow>,
    pub(crate) state: ListState,
    pub(crate) match_count: usize,
    pub(crate) text_match_count: usize,
    pub(crate) scope: SearchScope,
    pub(crate) options: SearchOptions,
    pub(crate) searching: bool,
    pub(crate) search_truncated: bool,
    pub(crate) inventory_truncated: bool,
    pub(crate) error: Option<String>,
    index: Arc<Vec<IndexedFile>>,
    file_matches: Vec<FileMatch>,
    text_matches: Vec<TextSearchMatch>,
    files_fingerprint: Option<u64>,
    ignored_available: bool,
    rows_generation: u64,
    worker: SearchWorker,
    pub(crate) preview_path: Option<RepoPath>,
    pub(crate) preview_line: Option<usize>,
    pub(crate) preview_match: Option<(usize, usize)>,
    pub(crate) preview_content: String,
    pub(crate) preview_loading: bool,
    preview_worker: SearchPreviewWorker,
}

impl FileSearch {
    pub(crate) fn new(
        files: &[RepoPath],
        ignored_files: &[RepoPath],
        files_fingerprint: Option<u64>,
    ) -> Self {
        let mut search = Self {
            query: TextInput::default(),
            rows: Vec::new(),
            state: ListState::default(),
            match_count: 0,
            text_match_count: 0,
            scope: SearchScope::All,
            options: SearchOptions::default(),
            searching: false,
            search_truncated: false,
            inventory_truncated: false,
            error: None,
            index: Arc::new(Vec::new()),
            file_matches: Vec::new(),
            text_matches: Vec::new(),
            files_fingerprint: None,
            ignored_available: false,
            rows_generation: 0,
            worker: SearchWorker::new(),
            preview_path: None,
            preview_line: None,
            preview_match: None,
            preview_content: String::new(),
            preview_loading: false,
            preview_worker: SearchPreviewWorker::new(),
        };
        search.reindex(files, ignored_files, files_fingerprint);
        search
    }

    pub(crate) fn reindex(
        &mut self,
        files: &[RepoPath],
        ignored_files: &[RepoPath],
        files_fingerprint: Option<u64>,
    ) {
        if self.files_fingerprint == files_fingerprint {
            return;
        }
        self.worker.cancel();
        self.text_matches.clear();
        self.text_match_count = 0;
        self.searching = false;
        self.search_truncated = false;
        self.error = None;
        let _activity =
            crate::diagnostics::activity("index-repo-search", format!("files={}", files.len()));
        let index = files
            .iter()
            .map(|path| {
                let path_lower = path.display().to_lowercase();
                let name_start = path_lower.rfind('/').map_or(0, |index| index + 1);
                IndexedFile {
                    path: path.clone(),
                    path_lower,
                    name_start,
                    ignored: ignored_files.binary_search(path).is_ok(),
                }
            })
            .collect::<Vec<_>>();
        self.ignored_available = index.iter().any(|file| file.ignored);
        let previous = std::mem::replace(&mut self.index, Arc::new(index));
        if previous.len() >= 10_000 {
            crate::diagnostics::drop_in_background("repo-search-index", previous);
        }
        self.files_fingerprint = files_fingerprint;
        self.refresh_file_matches();
    }

    pub(crate) fn repository_refreshed(&mut self, repository: &RepositoryData) {
        self.clear_preview();
        self.inventory_truncated = repository.inventory_truncated;
        self.reindex(
            &repository.files,
            &repository.ignored_files,
            Some(repository.files_fingerprint),
        );
        if !self.query.text().trim().is_empty() {
            self.refresh(repository);
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.worker.cancel();
        self.options.include_ignored = false;
        let previous = std::mem::replace(&mut self.index, Arc::new(Vec::new()));
        if previous.len() >= 10_000 {
            crate::diagnostics::drop_in_background("repo-search-index", previous);
        }
        self.files_fingerprint = None;
        self.open(false);
    }

    pub(crate) fn open(&mut self, inventory_truncated: bool) {
        self.worker.cancel();
        self.clear_preview();
        self.query.clear();
        self.query.focus();
        self.file_matches.clear();
        self.text_matches.clear();
        self.match_count = 0;
        self.text_match_count = 0;
        self.searching = false;
        self.search_truncated = false;
        self.inventory_truncated = inventory_truncated;
        self.error = None;
        self.rows.clear();
        self.state = ListState::default();
        self.bump_rows_generation();
    }

    pub(crate) fn close(&mut self) {
        self.worker.cancel();
        self.clear_preview();
        self.searching = false;
    }

    pub(crate) fn clear_preview(&mut self) {
        self.preview_worker.cancel();
        self.preview_path = None;
        self.preview_line = None;
        self.preview_match = None;
        self.preview_content.clear();
        self.preview_loading = false;
    }

    pub(crate) fn ensure_preview(&mut self, root: &Path) {
        let Some(row) = self.state.selected().and_then(|index| self.rows.get(index)) else {
            self.clear_preview();
            return;
        };
        let (path, line, matched) = match row {
            FileSearchRow::File { path } => (path.clone(), None, None),
            FileSearchRow::Text(result) => (
                result.path.clone(),
                Some(result.line),
                Some((
                    result.column.saturating_sub(1),
                    result.matched.chars().count(),
                )),
            ),
            FileSearchRow::Header { .. } | FileSearchRow::Status(_) => {
                self.clear_preview();
                return;
            }
        };
        self.preview_line = line;
        self.preview_match = matched;
        if self.preview_path.as_ref() == Some(&path) {
            return;
        }
        self.preview_path = Some(path.clone());
        self.preview_content.clear();
        self.preview_loading = true;
        self.preview_worker.request(root.to_path_buf(), path);
    }

    pub(crate) fn paste(&mut self, text: &str, repository: &RepositoryData) {
        self.query.insert_single_line(text);
        self.refresh(repository);
    }

    pub(crate) fn clear(&mut self, repository: &RepositoryData) {
        self.query.clear();
        self.refresh(repository);
    }

    pub(crate) fn delete_word(&mut self, repository: &RepositoryData) {
        self.query.delete_word();
        self.refresh(repository);
    }

    pub(crate) fn handle_edit_key(
        &mut self,
        key: KeyEvent,
        repository: &RepositoryData,
    ) -> EditOutcome {
        let outcome = self.query.handle_edit_key(key);
        if outcome == EditOutcome::Edited {
            self.refresh(repository);
        }
        outcome
    }

    pub(crate) fn set_scope(&mut self, scope: SearchScope, repository: &RepositoryData) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.refresh(repository);
    }

    pub(crate) fn move_scope(&mut self, delta: isize, repository: &RepositoryData) {
        self.set_scope(self.scope.shifted(delta), repository);
    }

    pub(crate) fn toggle_case(&mut self, repository: &RepositoryData) {
        if self.scope == SearchScope::Files {
            return;
        }
        self.options.case_sensitive = !self.options.case_sensitive;
        self.refresh(repository);
    }

    pub(crate) fn toggle_whole_word(&mut self, repository: &RepositoryData) {
        if self.scope == SearchScope::Files {
            return;
        }
        self.options.whole_word = !self.options.whole_word;
        self.refresh(repository);
    }

    pub(crate) fn toggle_regex(&mut self, repository: &RepositoryData) {
        if self.scope == SearchScope::Files {
            return;
        }
        self.options.regex = !self.options.regex;
        self.refresh(repository);
    }

    pub(crate) fn toggle_ignored(&mut self, repository: &RepositoryData) {
        if !self.ignored_available {
            return;
        }
        self.options.include_ignored = !self.options.include_ignored;
        self.refresh(repository);
    }

    pub(crate) fn ignored_available(&self) -> bool {
        self.ignored_available
    }

    pub(crate) fn total_files(&self) -> usize {
        self.index.len()
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let selectable = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.selectable().then_some(index))
            .collect::<Vec<_>>();
        if selectable.is_empty() {
            self.state.select(None);
            return;
        }
        let current = self
            .state
            .selected()
            .and_then(|selected| selectable.iter().position(|index| *index == selected))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(selectable.len().saturating_sub(1));
        self.state.select(Some(selectable[next]));
    }

    pub(crate) fn select_first(&mut self) {
        self.state
            .select(self.rows.iter().position(FileSearchRow::selectable));
    }

    pub(crate) fn select_last(&mut self) {
        self.state
            .select(self.rows.iter().rposition(FileSearchRow::selectable));
    }

    pub(crate) fn select(&mut self, generation: u64, row: usize) -> bool {
        if generation != self.rows_generation
            || !self.rows.get(row).is_some_and(FileSearchRow::selectable)
        {
            return false;
        }
        self.state.select(Some(row));
        true
    }

    pub(crate) fn target_generation(&self) -> u64 {
        self.rows_generation
    }

    pub(crate) fn selected_destination(&self) -> Option<SearchDestination> {
        match self
            .state
            .selected()
            .and_then(|index| self.rows.get(index))?
        {
            FileSearchRow::File { path } => Some(SearchDestination::File(path.clone())),
            FileSearchRow::Text(result) => Some(SearchDestination::Text {
                path: result.path.clone(),
                line: result.line,
            }),
            FileSearchRow::Header { .. } | FileSearchRow::Status(_) => None,
        }
    }

    pub(crate) fn poll(&mut self, repository: Option<&RepositoryData>) -> bool {
        let mut changed = false;
        while let Some(event) = self.worker.poll() {
            let Some(repository) = repository else {
                continue;
            };
            if event.generation() != self.worker.generation()
                || event.root() != repository.root
                || event.fingerprint() != repository.files_fingerprint
            {
                continue;
            }
            let selected = self.selected_destination();
            match event {
                SearchEvent::Batch { matches, .. } => {
                    self.text_matches.extend(matches);
                    self.text_matches.sort_unstable_by(|left, right| {
                        left.path
                            .cmp(&right.path)
                            .then_with(|| left.line.cmp(&right.line))
                            .then_with(|| left.column.cmp(&right.column))
                    });
                    self.text_matches.dedup_by(|left, right| {
                        left.path == right.path
                            && left.line == right.line
                            && left.column == right.column
                    });
                    self.text_match_count = self.text_matches.len();
                    self.rebuild_rows(selected.as_ref());
                    changed = true;
                }
                SearchEvent::Complete { truncated, .. } => {
                    self.searching = false;
                    self.search_truncated = truncated;
                    self.text_match_count = self.text_matches.len();
                    self.rebuild_rows(selected.as_ref());
                    changed = true;
                }
                SearchEvent::Error { message, .. } => {
                    self.searching = false;
                    self.error = Some(message.split_whitespace().collect::<Vec<_>>().join(" "));
                    self.rebuild_rows(selected.as_ref());
                    changed = true;
                }
            }
        }
        if let Some(result) = self.preview_worker.poll()
            && self.preview_path.as_ref() == Some(&result.path)
        {
            self.preview_content = result.content;
            self.preview_loading = false;
            changed = true;
        }
        changed
    }

    pub(crate) fn shutdown(&mut self) {
        self.worker.shutdown();
        self.preview_worker.shutdown();
    }

    fn refresh(&mut self, repository: &RepositoryData) {
        self.worker.cancel();
        self.text_matches.clear();
        self.text_match_count = 0;
        self.search_truncated = false;
        self.error = None;
        self.refresh_file_matches();
        if self.query.text().trim().is_empty() || self.scope == SearchScope::Files {
            self.searching = false;
            self.rebuild_rows(None);
            return;
        }
        self.searching = true;
        self.worker.request(SearchRequest {
            generation: self.worker.generation(),
            root: repository.root.clone(),
            fingerprint: repository.files_fingerprint,
            query: self.query.text().to_owned(),
            options: self.options,
            index: Arc::clone(&self.index),
        });
        self.rebuild_rows(None);
    }

    fn refresh_file_matches(&mut self) {
        self.file_matches.clear();
        self.match_count = 0;
        let query = self.query.text().trim().to_lowercase();
        let terms = query.split_whitespace().collect::<Vec<_>>();
        if terms.is_empty() || self.scope == SearchScope::Text {
            self.rebuild_rows(None);
            return;
        }
        for file in self.index.iter() {
            if file.ignored && !self.options.include_ignored {
                continue;
            }
            let Some(score) = file_score(&terms, file) else {
                continue;
            };
            self.match_count += 1;
            let candidate = FileMatch {
                path: file.path.clone(),
                score,
            };
            if self.file_matches.len() < MAX_FILE_RESULTS {
                self.file_matches.push(candidate);
            } else if let Some((worst, _)) = self
                .file_matches
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| file_result_order(left, right))
                && file_result_order(&candidate, &self.file_matches[worst]).is_lt()
            {
                self.file_matches[worst] = candidate;
            }
        }
        self.file_matches.sort_by(file_result_order);
        self.rebuild_rows(None);
    }

    fn rebuild_rows(&mut self, selected: Option<&SearchDestination>) {
        self.rows.clear();
        if self.query.text().trim().is_empty() {
            self.state.select(None);
            self.bump_rows_generation();
            return;
        }
        if matches!(self.scope, SearchScope::All | SearchScope::Files) {
            self.rows.push(FileSearchRow::Header {
                label: "FILES",
                count: self.match_count,
            });
            let limit = if self.scope == SearchScope::All {
                MAX_ALL_FILE_RESULTS
            } else {
                MAX_FILE_RESULTS
            };
            self.rows
                .extend(
                    self.file_matches
                        .iter()
                        .take(limit)
                        .map(|result| FileSearchRow::File {
                            path: result.path.clone(),
                        }),
                );
        }
        if matches!(self.scope, SearchScope::All | SearchScope::Text) {
            self.rows.push(FileSearchRow::Header {
                label: "TEXT",
                count: self.text_match_count,
            });
            self.rows
                .extend(self.text_matches.iter().cloned().map(FileSearchRow::Text));
            if self.searching && self.text_matches.is_empty() {
                self.rows
                    .push(FileSearchRow::Status("Searching file contents…".to_owned()));
            } else if !self.searching && self.text_matches.is_empty() && self.error.is_none() {
                self.rows
                    .push(FileSearchRow::Status("No text matches".to_owned()));
            }
        }
        let selected_row = selected.and_then(|selected| {
            self.rows.iter().position(|row| match (selected, row) {
                (SearchDestination::File(selected), FileSearchRow::File { path }) => {
                    selected == path
                }
                (SearchDestination::Text { path, line }, FileSearchRow::Text(result)) => {
                    path == &result.path && line == &result.line
                }
                _ => false,
            })
        });
        let first = self.rows.iter().position(FileSearchRow::selectable);
        self.state.select(selected_row.or(first));
        self.bump_rows_generation();
    }

    fn bump_rows_generation(&mut self) {
        self.rows_generation = self.rows_generation.wrapping_add(1);
    }
}

struct SearchPreviewResult {
    path: RepoPath,
    content: String,
}

struct SearchPreviewWorker {
    pending: Arc<Mutex<Option<(u64, PathBuf, RepoPath)>>>,
    wake: Option<SyncSender<()>>,
    results: Receiver<(u64, SearchPreviewResult)>,
    generation: u64,
    join: Option<JoinHandle<()>>,
}

impl SearchPreviewWorker {
    fn new() -> Self {
        let pending: Arc<Mutex<Option<(u64, PathBuf, RepoPath)>>> = Arc::new(Mutex::new(None));
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::channel();
        let worker_pending = Arc::clone(&pending);
        let join = thread::spawn(move || {
            while wake_rx.recv().is_ok() {
                let request = worker_pending.lock().ok().and_then(|mut slot| slot.take());
                let Some((generation, root, path)) = request else {
                    continue;
                };
                let content = git::file_content(&root, &path)
                    .unwrap_or_else(|error| format!("Unable to preview file: {error:#}"));
                if result_tx
                    .send((generation, SearchPreviewResult { path, content }))
                    .is_err()
                {
                    return;
                }
            }
        });
        Self {
            pending,
            wake: Some(wake_tx),
            results: result_rx,
            generation: 0,
            join: Some(join),
        }
    }

    fn request(&mut self, root: PathBuf, path: RepoPath) {
        self.generation = self.generation.wrapping_add(1);
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some((self.generation, root, path));
        }
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
    }

    fn poll(&mut self) -> Option<SearchPreviewResult> {
        let mut latest = None;
        while let Ok((generation, result)) = self.results.try_recv() {
            if generation == self.generation {
                latest = Some(result);
            }
        }
        latest
    }

    fn shutdown(&mut self) {
        self.wake.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for FileSearch {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn file_score(terms: &[&str], file: &IndexedFile) -> Option<u32> {
    terms.iter().try_fold(0_u32, |total, term| {
        let path_score = fuzzy_text_score_lower(term, &file.path_lower);
        let name_score = fuzzy_text_score_lower(term, &file.path_lower[file.name_start..])
            .map(|score| score.saturating_add(1_500));
        path_score
            .into_iter()
            .chain(name_score)
            .max()
            .map(|score| total.saturating_add(score))
    })
}

fn file_result_order(left: &FileMatch, right: &FileMatch) -> Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.path.cmp(&right.path))
}

struct SearchWorker {
    generation: Arc<AtomicU64>,
    pending: Arc<Mutex<Option<SearchRequest>>>,
    wake: Option<SyncSender<()>>,
    receiver: Receiver<SearchEvent>,
    worker: Option<JoinHandle<()>>,
}

impl SearchWorker {
    fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(Mutex::new(None::<SearchRequest>));
        let worker_pending = Arc::clone(&pending);
        let worker_generation = Arc::clone(&generation);
        let (wake, wake_rx) = mpsc::sync_channel::<()>(1);
        let (result_tx, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while wake_rx.recv().is_ok() {
                loop {
                    match wake_rx.recv_timeout(SEARCH_DEBOUNCE) {
                        Ok(()) => {}
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                let Some(request) = worker_pending.lock().ok().and_then(|mut slot| slot.take())
                else {
                    continue;
                };
                if request.generation != worker_generation.load(AtomicOrdering::Acquire) {
                    continue;
                }
                run_search(request, &worker_generation, &result_tx);
            }
        });
        Self {
            generation,
            pending,
            wake: Some(wake),
            receiver,
            worker: Some(worker),
        }
    }

    fn generation(&self) -> u64 {
        self.generation.load(AtomicOrdering::Acquire)
    }

    fn cancel(&mut self) {
        self.generation.fetch_add(1, AtomicOrdering::AcqRel);
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
    }

    fn request(&mut self, mut request: SearchRequest) {
        request.generation = self.generation();
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(request);
            if let Some(wake) = &self.wake {
                let _ = wake.try_send(());
            }
        }
    }

    fn poll(&mut self) -> Option<SearchEvent> {
        self.receiver.try_recv().ok()
    }

    fn shutdown(&mut self) {
        self.cancel();
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct SearchRequest {
    generation: u64,
    root: PathBuf,
    fingerprint: u64,
    query: String,
    options: SearchOptions,
    index: Arc<Vec<IndexedFile>>,
}

enum SearchEvent {
    Batch {
        generation: u64,
        root: PathBuf,
        fingerprint: u64,
        matches: Vec<TextSearchMatch>,
    },
    Complete {
        generation: u64,
        root: PathBuf,
        fingerprint: u64,
        truncated: bool,
    },
    Error {
        generation: u64,
        root: PathBuf,
        fingerprint: u64,
        message: String,
    },
}

impl SearchEvent {
    fn generation(&self) -> u64 {
        match self {
            Self::Batch { generation, .. }
            | Self::Complete { generation, .. }
            | Self::Error { generation, .. } => *generation,
        }
    }

    fn root(&self) -> &Path {
        match self {
            Self::Batch { root, .. } | Self::Complete { root, .. } | Self::Error { root, .. } => {
                root
            }
        }
    }

    fn fingerprint(&self) -> u64 {
        match self {
            Self::Batch { fingerprint, .. }
            | Self::Complete { fingerprint, .. }
            | Self::Error { fingerprint, .. } => *fingerprint,
        }
    }
}

fn run_search(
    request: SearchRequest,
    active_generation: &AtomicU64,
    sender: &mpsc::Sender<SearchEvent>,
) {
    let matcher = match build_matcher(&request.query, request.options) {
        Ok(matcher) => matcher,
        Err(error) => {
            let _ = sender.send(SearchEvent::Error {
                generation: request.generation,
                root: request.root,
                fingerprint: request.fingerprint,
                message: format!("Invalid regular expression: {error}"),
            });
            return;
        }
    };
    let next_file = AtomicUsize::new(0);
    let result_count = AtomicUsize::new(0);
    let worker_count = thread::available_parallelism()
        .map_or(1, usize::from)
        .saturating_sub(1)
        .clamp(1, 8);
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let request = &request;
            let matcher = &matcher;
            let next_file = &next_file;
            let result_count = &result_count;
            let sender = sender.clone();
            scope.spawn(move || {
                let mut batch = Vec::with_capacity(SEARCH_BATCH_SIZE);
                let context = ScanContext {
                    request,
                    active_generation,
                    result_count,
                    sender: &sender,
                };
                loop {
                    if active_generation.load(AtomicOrdering::Acquire) != request.generation
                        || result_count.load(AtomicOrdering::Acquire) >= MAX_TEXT_RESULTS
                    {
                        break;
                    }
                    let index = next_file.fetch_add(1, AtomicOrdering::Relaxed);
                    let Some(file) = request.index.get(index) else {
                        break;
                    };
                    if file.ignored && !request.options.include_ignored {
                        continue;
                    }
                    if !scan_file(&request.root, file, matcher, &mut batch, &context) {
                        break;
                    }
                }
                if !batch.is_empty() {
                    let _ = send_batch(request, &sender, batch);
                }
            });
        }
    });
    if active_generation.load(AtomicOrdering::Acquire) != request.generation {
        return;
    }
    let _ = sender.send(SearchEvent::Complete {
        generation: request.generation,
        root: request.root,
        fingerprint: request.fingerprint,
        truncated: result_count.load(AtomicOrdering::Acquire) >= MAX_TEXT_RESULTS,
    });
}

fn send_batch(
    request: &SearchRequest,
    sender: &mpsc::Sender<SearchEvent>,
    matches: Vec<TextSearchMatch>,
) -> Result<(), mpsc::SendError<SearchEvent>> {
    sender.send(SearchEvent::Batch {
        generation: request.generation,
        root: request.root.clone(),
        fingerprint: request.fingerprint,
        matches,
    })
}

fn build_matcher(query: &str, options: SearchOptions) -> Result<Regex, regex::Error> {
    let pattern = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let pattern = if options.whole_word {
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .unicode(true)
        .build()
}

struct ScanContext<'a> {
    request: &'a SearchRequest,
    active_generation: &'a AtomicU64,
    result_count: &'a AtomicUsize,
    sender: &'a mpsc::Sender<SearchEvent>,
}

fn scan_file(
    root: &Path,
    file: &IndexedFile,
    matcher: &Regex,
    batch: &mut Vec<TextSearchMatch>,
    context: &ScanContext<'_>,
) -> bool {
    let path = root.join(file.path.as_path());
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return true;
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_SEARCH_FILE_BYTES
    {
        return true;
    }
    let Ok(bytes) = fs::read(path) else {
        return true;
    };
    if bytes.contains(&0) {
        return true;
    }
    let Ok(content) = std::str::from_utf8(&bytes) else {
        return true;
    };
    for (line_index, line) in content.lines().enumerate() {
        if context.active_generation.load(AtomicOrdering::Acquire) != context.request.generation {
            return false;
        }
        for found in matcher.find_iter(line) {
            let position = context.result_count.fetch_add(1, AtomicOrdering::AcqRel);
            if position >= MAX_TEXT_RESULTS {
                return false;
            }
            batch.push(text_match(
                &file.path,
                line_index + 1,
                line,
                found.start(),
                found.end(),
            ));
            if batch.len() >= SEARCH_BATCH_SIZE
                && send_batch(context.request, context.sender, std::mem::take(batch)).is_err()
            {
                return false;
            }
        }
    }
    true
}

fn text_match(
    path: &RepoPath,
    line_number: usize,
    line: &str,
    match_start: usize,
    match_end: usize,
) -> TextSearchMatch {
    let mut visible_match_end = match_end.min(match_start.saturating_add(MAX_MATCH_CONTEXT_BYTES));
    while visible_match_end > match_start && !line.is_char_boundary(visible_match_end) {
        visible_match_end -= 1;
    }
    let context =
        MAX_MATCH_CONTEXT_BYTES.saturating_sub(visible_match_end.saturating_sub(match_start));
    let mut start = match_start.saturating_sub(context / 2);
    while start < match_start && !line.is_char_boundary(start) {
        start += 1;
    }
    let mut end = if visible_match_end < match_end {
        visible_match_end
    } else {
        visible_match_end
            .saturating_add(context / 2)
            .min(line.len())
    };
    while end > visible_match_end && !line.is_char_boundary(end) {
        end -= 1;
    }
    let leading = if start > 0 { "..." } else { "" };
    let trailing = if end < line.len() { "..." } else { "" };
    TextSearchMatch {
        path: path.clone(),
        line: line_number,
        column: line[..match_start].chars().count() + 1,
        before: format!("{leading}{}", clean_context(&line[start..match_start])),
        matched: clean_context(&line[match_start..visible_match_end]),
        after: format!("{}{trailing}", clean_context(&line[visible_match_end..end])),
    }
}

fn clean_context(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\t' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use crate::{git, repo_path::RepoPath};

    use super::{
        FileSearch, FileSearchRow, MAX_MATCH_CONTEXT_BYTES, SearchDestination, SearchOptions,
        SearchScope, build_matcher, text_match,
    };

    #[test]
    fn favors_basenames_and_matches_multiple_terms() {
        let files = vec![
            RepoPath::from("src/application.rs"),
            RepoPath::from("docs/app-notes.md"),
            RepoPath::from("src/ui/app_view.rs"),
        ];
        let mut search = FileSearch::new(&files, &[], Some(1));
        search.query.set("app view");
        search.scope = SearchScope::Files;
        search.refresh_file_matches();

        assert_eq!(search.match_count, 1);
        assert_eq!(
            search.selected_destination(),
            Some(SearchDestination::File(RepoPath::from(
                "src/ui/app_view.rs"
            )))
        );
    }

    #[test]
    fn keeps_only_the_best_file_results() {
        let files = (0..100)
            .map(|index| RepoPath::from(format!("src/file-{index:03}.rs")))
            .collect::<Vec<_>>();
        let mut search = FileSearch::new(&files, &[], Some(1));
        search.query.set("f");
        search.scope = SearchScope::Files;
        search.refresh_file_matches();

        assert_eq!(
            search
                .rows
                .iter()
                .filter(|row| matches!(row, FileSearchRow::File { .. }))
                .count(),
            40
        );
        assert_eq!(search.match_count, files.len());
    }

    #[test]
    fn scans_text_and_filters_ignored_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap();
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(root.join("visible.rs"), "alpha\nNeedle value\n").unwrap();
        fs::write(root.join("ignored.rs"), "Needle hidden\n").unwrap();
        let repository = git::load(root).unwrap();
        let mut search = FileSearch::new(
            &repository.files,
            &repository.ignored_files,
            Some(repository.files_fingerprint),
        );

        search.query.set("Needle");
        search.refresh(&repository);
        wait_for_search(&mut search, &repository);

        assert_eq!(search.text_match_count, 1);
        assert!(
            search
                .text_matches
                .iter()
                .all(|result| result.path == "visible.rs")
        );

        search.toggle_ignored(&repository);
        wait_for_search(&mut search, &repository);
        assert_eq!(search.text_match_count, 2);

        search.scope = SearchScope::Files;
        search.toggle_case(&repository);
        search.toggle_whole_word(&repository);
        search.toggle_regex(&repository);
        assert!(!search.options.case_sensitive);
        assert!(!search.options.whole_word);
        assert!(!search.options.regex);
    }

    #[test]
    fn rejects_regexes_that_can_split_utf8_characters() {
        let options = SearchOptions {
            regex: true,
            ..SearchOptions::default()
        };

        assert!(build_matcher("(?-u:.)", options).is_err());
        let matcher = build_matcher(".", options).unwrap();
        let found = matcher.find("é").unwrap();
        assert_eq!((found.start(), found.end()), (0, "é".len()));
    }

    #[test]
    fn bounds_the_complete_rendered_match_context() {
        let line = "x".repeat(MAX_MATCH_CONTEXT_BYTES * 4);
        let result = text_match(&RepoPath::from("minified.js"), 1, &line, 0, line.len());

        assert!(result.matched.len() <= MAX_MATCH_CONTEXT_BYTES);
        assert_eq!(result.after, "...");
        assert!(result.before.len() + result.matched.len() + result.after.len() <= 515);
    }

    #[test]
    fn refreshes_an_active_query_after_file_contents_change() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap();
        fs::write(root.join("first.rs"), "Needle one\n").unwrap();
        let repository = git::load(root).unwrap();
        let mut search = FileSearch::new(
            &repository.files,
            &repository.ignored_files,
            Some(repository.files_fingerprint),
        );
        search.query.set("Needle");
        search.refresh(&repository);
        wait_for_search(&mut search, &repository);
        assert_eq!(search.text_match_count, 1);

        fs::write(root.join("first.rs"), "Needle one\nNeedle two\n").unwrap();
        let mut refreshed = git::load(root).unwrap();
        refreshed.inventory_truncated = true;
        assert_eq!(
            refreshed.files_fingerprint, repository.files_fingerprint,
            "content edits should not change the inventory fingerprint"
        );
        search.repository_refreshed(&refreshed);
        wait_for_search(&mut search, &refreshed);

        assert_eq!(search.text_match_count, 2);
        assert!(search.inventory_truncated);
    }

    #[test]
    fn bounds_matches_while_scanning_one_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap();
        fs::write(root.join("dense.txt"), "a".repeat(2_000)).unwrap();
        let repository = git::load(root).unwrap();
        let mut search = FileSearch::new(
            &repository.files,
            &repository.ignored_files,
            Some(repository.files_fingerprint),
        );
        search.scope = SearchScope::Text;
        search.query.set("a");
        search.refresh(&repository);
        wait_for_search(&mut search, &repository);

        assert_eq!(search.text_match_count, 500);
        assert!(search.search_truncated);
    }

    fn wait_for_search(search: &mut FileSearch, repository: &git::RepositoryData) {
        for _ in 0..100 {
            let _ = search.poll(Some(repository));
            if !search.searching {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("search did not complete");
    }
}
