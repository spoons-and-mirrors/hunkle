use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use crate::git::{self, DiffSummary};

struct SummaryResult {
    generation: u64,
    root: PathBuf,
    requested: Vec<String>,
    result: Result<HashMap<String, DiffSummary>, String>,
}

struct SummaryRequest {
    generation: u64,
    root: PathBuf,
    requested: Vec<String>,
}

const MAX_BATCH_SIZE: usize = 32;
const MAX_CACHED_SUMMARIES: usize = 512;
const MAX_QUEUED_SUMMARIES: usize = 256;
const MAX_TRANSIENT_RETRIES: u8 = 2;

pub(crate) struct CommitSummaryCache {
    root: Option<PathBuf>,
    generation: u64,
    summaries: HashMap<String, DiffSummary>,
    summary_order: VecDeque<String>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    attempts: HashMap<String, u8>,
    queued: VecDeque<String>,
    running: bool,
    request_sender: Sender<SummaryRequest>,
    receiver: Receiver<SummaryResult>,
}

impl Default for CommitSummaryCache {
    fn default() -> Self {
        let (request_sender, request_receiver) = mpsc::channel::<SummaryRequest>();
        let (result_sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("hunkle-commit-summaries".to_owned())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let result = git::commit_summaries(&request.root, &request.requested)
                        .map_err(|error| error.to_string());
                    if result_sender
                        .send(SummaryResult {
                            generation: request.generation,
                            root: request.root,
                            requested: request.requested,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("could not start commit summary worker");
        Self {
            root: None,
            generation: 0,
            summaries: HashMap::new(),
            summary_order: VecDeque::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            attempts: HashMap::new(),
            queued: VecDeque::new(),
            running: false,
            request_sender,
            receiver,
        }
    }
}

impl CommitSummaryCache {
    pub(crate) fn get(&self, oid: &str) -> Option<&DiffSummary> {
        self.summaries.get(oid)
    }

    pub(crate) fn failed(&self, oid: &str) -> bool {
        self.failed.contains(oid)
    }

    pub(crate) fn request<'a>(&mut self, root: &Path, oids: impl IntoIterator<Item = &'a str>) {
        self.activate(root);
        let mut requested = Vec::new();
        let mut seen = HashSet::new();
        for oid in oids {
            if !self.summaries.contains_key(oid)
                && !self.pending.contains(oid)
                && !self.failed.contains(oid)
                && seen.insert(oid.to_owned())
            {
                requested.push(oid.to_owned());
            }
        }
        if requested.is_empty() {
            return;
        }
        self.pending.extend(requested.iter().cloned());
        for oid in requested.into_iter().rev() {
            self.queued.push_front(oid);
        }
        while self.queued.len() > MAX_QUEUED_SUMMARIES {
            if let Some(oid) = self.queued.pop_back() {
                self.pending.remove(&oid);
                self.attempts.remove(&oid);
            }
        }
        self.start_next_batch();
    }

    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.receiver.try_recv() {
            self.running = false;
            if done.generation != self.generation || self.root.as_deref() != Some(&done.root) {
                continue;
            }
            for oid in &done.requested {
                self.pending.remove(oid);
            }
            match done.result {
                Ok(summaries) => {
                    for oid in done
                        .requested
                        .iter()
                        .filter(|oid| !summaries.contains_key(*oid))
                    {
                        self.attempts.remove(oid);
                        self.failed.insert(oid.clone());
                    }
                    for (oid, summary) in summaries {
                        self.attempts.remove(&oid);
                        self.summary_order.push_back(oid.clone());
                        self.summaries.insert(oid, summary);
                    }
                    while self.summaries.len() > MAX_CACHED_SUMMARIES {
                        if let Some(oid) = self.summary_order.pop_front() {
                            self.summaries.remove(&oid);
                        }
                    }
                }
                Err(_) => {
                    for oid in done.requested.into_iter().rev() {
                        let attempts = self.attempts.entry(oid.clone()).or_default();
                        *attempts += 1;
                        if *attempts <= MAX_TRANSIENT_RETRIES {
                            self.pending.insert(oid.clone());
                            self.queued.push_back(oid);
                        } else {
                            self.attempts.remove(&oid);
                            self.failed.insert(oid);
                        }
                    }
                }
            }
            changed = true;
        }
        self.start_next_batch();
        changed
    }

    fn activate(&mut self, root: &Path) {
        if self.root.as_deref() == Some(root) {
            return;
        }
        self.root = Some(root.to_owned());
        self.generation = self.generation.wrapping_add(1);
        self.summaries.clear();
        self.summary_order.clear();
        self.pending.clear();
        self.failed.clear();
        self.attempts.clear();
        self.queued.clear();
    }

    fn start_next_batch(&mut self) {
        if self.running || self.queued.is_empty() {
            return;
        }
        let requested = self
            .queued
            .drain(..self.queued.len().min(MAX_BATCH_SIZE))
            .collect();
        let request = SummaryRequest {
            generation: self.generation,
            root: self.root.clone().expect("summary cache has an active root"),
            requested,
        };
        if self.request_sender.send(request).is_ok() {
            self.running = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> (
        CommitSummaryCache,
        Receiver<SummaryRequest>,
        Sender<SummaryResult>,
    ) {
        let (request_sender, request_receiver) = mpsc::channel();
        let (result_sender, receiver) = mpsc::channel();
        (
            CommitSummaryCache {
                root: None,
                generation: 0,
                summaries: HashMap::new(),
                summary_order: VecDeque::new(),
                pending: HashSet::new(),
                failed: HashSet::new(),
                attempts: HashMap::new(),
                queued: VecDeque::new(),
                running: false,
                request_sender,
                receiver,
            },
            request_receiver,
            result_sender,
        )
    }

    #[test]
    fn retries_transient_failures_before_caching_a_failure() {
        let (mut cache, requests, results) = cache();
        let root = Path::new("/repo");
        cache.request(root, ["abc"]);

        for attempt in 0..=MAX_TRANSIENT_RETRIES {
            let request = requests.recv().unwrap();
            results
                .send(SummaryResult {
                    generation: request.generation,
                    root: request.root,
                    requested: request.requested,
                    result: Err("temporary failure".to_owned()),
                })
                .unwrap();
            assert!(cache.poll());
            assert_eq!(cache.failed("abc"), attempt == MAX_TRANSIENT_RETRIES);
        }
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn prioritizes_visible_requests_and_bounds_stale_queued_work() {
        let (mut cache, _requests, _results) = cache();
        let root = Path::new("/repo");
        let stale = (0..400).map(|index| format!("stale-{index}"));
        let stale_refs = stale.collect::<Vec<_>>();
        cache.request(root, stale_refs.iter().map(String::as_str));
        cache.request(root, ["visible-a", "visible-b"]);

        assert!(cache.queued.len() <= MAX_QUEUED_SUMMARIES);
        assert_eq!(cache.queued.front().map(String::as_str), Some("visible-a"));
        assert_eq!(cache.queued.get(1).map(String::as_str), Some("visible-b"));
    }
}
