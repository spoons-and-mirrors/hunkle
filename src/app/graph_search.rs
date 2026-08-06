use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};

use crate::git::Commit;

use super::TextInput;

const ASYNC_CORPUS_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct GraphSearch {
    root: Option<PathBuf>,
    corpus_key: Option<GraphCorpusKey>,
    searchable_commits: Vec<String>,
    visible_indices: Vec<usize>,
    match_positions: Vec<usize>,
    terms: Vec<String>,
    selected_match: Option<usize>,
    worker: GraphCorpusWorker,
    pub(crate) input: TextInput,
}

#[derive(Debug, Clone)]
struct GraphCorpusKey {
    root: PathBuf,
    commits: Arc<Vec<Commit>>,
}

impl PartialEq for GraphCorpusKey {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && Arc::ptr_eq(&self.commits, &other.commits)
    }
}

impl Eq for GraphCorpusKey {}

impl Default for GraphSearch {
    fn default() -> Self {
        Self {
            root: None,
            corpus_key: None,
            searchable_commits: Vec::new(),
            visible_indices: Vec::new(),
            match_positions: Vec::new(),
            terms: Vec::new(),
            selected_match: None,
            worker: GraphCorpusWorker::new(),
            input: TextInput::default(),
        }
    }
}

impl GraphSearch {
    pub(crate) fn sync(
        &mut self,
        root: &Path,
        commits: &Arc<Vec<Commit>>,
        author_visible: &[usize],
    ) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_path_buf());
            self.input.clear();
        }
        let key = GraphCorpusKey {
            root: root.to_path_buf(),
            commits: Arc::clone(commits),
        };
        if self.corpus_key.as_ref() != Some(&key) {
            self.worker.cancel();
            self.searchable_commits.clear();
            self.match_positions.clear();
            self.terms.clear();
            let corpus_bytes = commits.iter().fold(0usize, |bytes, commit| {
                bytes
                    .saturating_add(commit.subject.len())
                    .saturating_add(commit.message.len())
                    .saturating_add(commit.refs.iter().map(String::len).sum::<usize>())
            });
            if corpus_bytes >= ASYNC_CORPUS_BYTES {
                self.worker.request(key);
                self.visible_indices.clear();
                self.visible_indices.extend_from_slice(author_visible);
                return;
            }
            self.searchable_commits = build_searchable_commits(commits, || false)
                .expect("synchronous graph corpus construction cannot be cancelled");
            self.corpus_key = Some(key);
        } else if self.visible_indices == author_visible {
            return;
        }
        self.apply(author_visible);
    }

    pub(crate) fn poll(&mut self, author_visible: &[usize]) -> bool {
        let mut changed = false;
        while let Some(completion) = self.worker.poll() {
            if completion.generation != self.worker.generation() {
                continue;
            }
            self.corpus_key = Some(completion.key);
            self.searchable_commits = completion.corpus;
            self.terms.clear();
            self.apply(author_visible);
            changed = true;
        }
        changed
    }

    pub(crate) fn apply(&mut self, author_visible: &[usize]) {
        let terms = self
            .input
            .text()
            .split_whitespace()
            .map(normalize_search_text)
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();
        let narrows_previous = self.visible_indices == author_visible
            && !self.terms.is_empty()
            && terms.len() >= self.terms.len()
            && self
                .terms
                .iter()
                .zip(&terms)
                .all(|(previous, current)| current.starts_with(previous));
        let candidates = narrows_previous.then(|| self.match_positions.clone());
        self.visible_indices.clear();
        self.visible_indices.extend_from_slice(author_visible);
        self.match_positions = if terms.is_empty() {
            Vec::new()
        } else if let Some(candidates) = candidates {
            candidates
                .into_iter()
                .filter(|position| {
                    author_visible
                        .get(*position)
                        .and_then(|index| self.searchable_commits.get(*index))
                        .is_some_and(|text| terms.iter().all(|term| text.contains(term.as_str())))
                })
                .collect()
        } else {
            author_visible
                .iter()
                .enumerate()
                .filter_map(|(position, index)| {
                    self.searchable_commits
                        .get(*index)
                        .is_some_and(|text| terms.iter().all(|term| text.contains(term.as_str())))
                        .then_some(position)
                })
                .collect()
        };
        self.terms = terms;
        self.selected_match = (!self.match_positions.is_empty()).then_some(0);
    }

    pub(crate) fn visible_indices(&self) -> &[usize] {
        &self.visible_indices
    }

    pub(crate) fn current_match_position(&self) -> Option<usize> {
        self.selected_match
            .and_then(|selected| self.match_positions.get(selected).copied())
    }

    pub(crate) fn match_status(&self) -> Option<(usize, usize)> {
        if self.input.is_empty() {
            return None;
        }
        Some((
            self.selected_match.map_or(0, |selected| selected + 1),
            self.match_positions.len(),
        ))
    }

    pub(crate) fn cycle_match(&mut self, forward: bool) -> Option<usize> {
        let count = self.match_positions.len();
        if count == 0 {
            return None;
        }
        let current = self.selected_match.unwrap_or(0);
        self.selected_match = Some(if forward {
            (current + 1) % count
        } else {
            current.checked_sub(1).unwrap_or(count - 1)
        });
        self.current_match_position()
    }
}

#[derive(Debug)]
struct GraphCorpusRequest {
    generation: u64,
    key: GraphCorpusKey,
}

#[derive(Debug)]
struct GraphCorpusCompletion {
    generation: u64,
    key: GraphCorpusKey,
    corpus: Vec<String>,
}

#[derive(Debug)]
struct GraphCorpusWorker {
    generation: Arc<AtomicU64>,
    pending: Arc<Mutex<Option<GraphCorpusRequest>>>,
    wake: Option<SyncSender<()>>,
    receiver: Receiver<GraphCorpusCompletion>,
    worker: Option<JoinHandle<()>>,
}

impl GraphCorpusWorker {
    fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(Mutex::new(None::<GraphCorpusRequest>));
        let worker_generation = Arc::clone(&generation);
        let worker_pending = Arc::clone(&pending);
        let (wake, wake_receiver) = mpsc::sync_channel(1);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while wake_receiver.recv().is_ok() {
                let Some(request) = worker_pending.lock().ok().and_then(|mut slot| slot.take())
                else {
                    continue;
                };
                let generation = request.generation;
                let Some(corpus) = build_searchable_commits(&request.key.commits, || {
                    worker_generation.load(Ordering::Acquire) != generation
                }) else {
                    continue;
                };
                if sender
                    .send(GraphCorpusCompletion {
                        generation,
                        key: request.key,
                        corpus,
                    })
                    .is_err()
                {
                    return;
                }
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
        self.generation.load(Ordering::Acquire)
    }

    fn request(&mut self, key: GraphCorpusKey) {
        self.cancel();
        let request = GraphCorpusRequest {
            generation: self.generation(),
            key,
        };
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(request);
        }
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
    }

    fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
    }

    fn poll(&mut self) -> Option<GraphCorpusCompletion> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for GraphCorpusWorker {
    fn drop(&mut self) {
        self.cancel();
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn build_searchable_commits(
    commits: &[Commit],
    mut cancelled: impl FnMut() -> bool,
) -> Option<Vec<String>> {
    let mut corpus = Vec::with_capacity(commits.len());
    for commit in commits {
        if cancelled() {
            return None;
        }
        corpus.push(searchable_commit_text(commit));
    }
    Some(corpus)
}

fn searchable_commit_text(commit: &Commit) -> String {
    normalize_search_text(&format!(
        "{} {} {} {} {} {}",
        commit.oid,
        commit.refs.join(" "),
        commit.subject,
        commit.message,
        commit.date,
        numeric_month(&commit.date).unwrap_or_default()
    ))
}

fn numeric_month(date: &str) -> Option<&'static str> {
    match date.split_whitespace().nth(1)? {
        "Jan" => Some("01"),
        "Feb" => Some("02"),
        "Mar" => Some("03"),
        "Apr" => Some("04"),
        "May" => Some("05"),
        "Jun" => Some("06"),
        "Jul" => Some("07"),
        "Aug" => Some("08"),
        "Sep" => Some("09"),
        "Oct" => Some("10"),
        "Nov" => Some("11"),
        "Dec" => Some("12"),
        _ => None,
    }
}

fn normalize_search_text(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(oid: &str, author: &str, date: &str, subject: &str, message: &str) -> Commit {
        Commit {
            oid: oid.to_owned(),
            parents: Vec::new(),
            refs: Vec::new(),
            author: author.to_owned(),
            date: date.to_owned(),
            subject: subject.to_owned(),
            message: message.to_owned(),
            graph: Vec::new(),
        }
    }

    #[test]
    fn finds_commit_content_and_date_without_filtering_the_graph() {
        let commits = Arc::new(vec![
            commit(
                "abc1234",
                "Ada Lovelace",
                "03 Aug 14:20",
                "Polish graph search",
                "Searches the complete commit message",
            ),
            commit(
                "def5678",
                "Grace Hopper",
                "02 Sep 09:10",
                "Improve navigation",
                "Keeps keyboard movement predictable",
            ),
        ]);
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[0, 1]);

        for query in ["abc", "graph", "complete message", "03Aug", "08"] {
            search.input.set(query);
            search.apply(&[0, 1]);
            assert_eq!(search.visible_indices(), &[0, 1], "query={query}");
            assert_eq!(search.current_match_position(), Some(0), "query={query}");
            assert_eq!(search.match_status(), Some((1, 1)), "query={query}");
        }

        search.input.set("Ada");
        search.apply(&[0, 1]);
        assert_eq!(search.visible_indices(), &[0, 1]);
        assert_eq!(search.current_match_position(), None);
        assert_eq!(search.match_status(), Some((0, 0)));
    }

    #[test]
    fn cycles_matches_within_the_author_filter() {
        let commits = Arc::new(vec![
            commit("abc", "Ada", "03 Aug", "First match", ""),
            commit("def", "Grace", "03 Aug", "Second match", ""),
            commit("ghi", "Linus", "02 Sep", "No match", ""),
        ]);
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[0, 1, 2]);
        search.input.set("03Aug");
        search.apply(&[0, 1, 2]);

        assert_eq!(search.visible_indices(), &[0, 1, 2]);
        assert_eq!(search.match_status(), Some((1, 2)));
        assert_eq!(search.cycle_match(true), Some(1));
        assert_eq!(search.match_status(), Some((2, 2)));
        assert_eq!(search.cycle_match(true), Some(0));
        assert_eq!(search.cycle_match(false), Some(1));

        search.apply(&[1, 2]);
        assert_eq!(search.visible_indices(), &[1, 2]);
        assert_eq!(search.current_match_position(), Some(0));
        assert_eq!(search.match_status(), Some((1, 1)));
    }

    #[test]
    fn unchanged_sync_reuses_the_normalized_corpus() {
        let commits = Arc::new(vec![commit(
            "abc",
            "Ada",
            "03 Aug",
            "Graph search",
            "needle",
        )]);
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[0]);
        let allocation = search.searchable_commits[0].as_ptr();

        search.sync(Path::new("/repo"), &commits, &[0]);

        assert_eq!(search.searchable_commits[0].as_ptr(), allocation);
    }

    #[test]
    fn extending_a_query_filters_the_previous_matches() {
        let commits = Arc::new(vec![
            commit("abc", "Ada", "03 Aug", "needle alpha", ""),
            commit("def", "Ada", "03 Aug", "needle beta", ""),
        ]);
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[0, 1]);
        search.input.set("need");
        search.apply(&[0, 1]);
        assert_eq!(search.match_status(), Some((1, 2)));

        search.input.set("needle alpha");
        search.apply(&[0, 1]);

        assert_eq!(search.match_status(), Some((1, 1)));
        assert_eq!(search.current_match_position(), Some(0));
    }

    #[test]
    fn builds_large_corpora_off_thread() {
        let commits = Arc::new(vec![commit(
            "abc",
            "Ada",
            "03 Aug",
            "Large history",
            &"needle".repeat(ASYNC_CORPUS_BYTES),
        )]);
        let mut search = GraphSearch::default();

        search.sync(Path::new("/repo"), &commits, &[0]);
        assert!(search.searchable_commits.is_empty());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !search.poll(&[0]) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(search.searchable_commits.len(), 1);
    }
}
