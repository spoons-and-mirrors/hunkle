use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
use std::sync::Arc;

use crate::{
    filesystem::same_path,
    git::{self, Branch, LinkedWorktree, WorktreeSignature},
};

mod known_repositories;

use known_repositories::KnownRepositoryStore;

const MIN_STATS_INTERVAL: Duration = Duration::from_secs(5);
const MAX_STATS_INTERVAL: Duration = Duration::from_secs(60);

type ChangeStats = (u64, u64);
type StatsLoadResult = Result<(WorktreeSignature, Option<ChangeStats>), String>;

#[cfg(test)]
type TestStatsLoader =
    Arc<dyn Fn(&Path, Option<WorktreeSignature>) -> StatsLoadResult + Send + Sync>;

#[derive(Debug, Clone)]
pub(crate) struct LinkedWorktreeRepository {
    pub(crate) common_dir: PathBuf,
    pub(crate) label: String,
    pub(crate) worktrees: Vec<LinkedWorktree>,
    pub(crate) branches: Vec<Branch>,
    pub(crate) branch_error: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RepositoryPickerItem {
    pub(crate) root: PathBuf,
    pub(crate) label: String,
    pub(crate) stats: Option<(u64, u64)>,
    pub(crate) branch: Option<String>,
}

pub(crate) struct AgentDestinationMetadata<'a> {
    repository: &'a LinkedWorktreeRepository,
    worktree: &'a LinkedWorktree,
}

impl<'a> AgentDestinationMetadata<'a> {
    pub(crate) fn repository(&self) -> &'a str {
        &self.repository.label
    }

    pub(crate) fn repository_root(&self) -> &'a Path {
        self.repository
            .worktrees
            .iter()
            .find(|worktree| worktree.is_main)
            .unwrap_or(self.worktree)
            .path
            .as_path()
    }

    pub(crate) fn branch(&self) -> &'a str {
        self.worktree
            .branch
            .as_deref()
            .map(|branch| branch.strip_prefix("refs/heads/").unwrap_or(branch))
            .unwrap_or("detached HEAD")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedWorktreeCandidate {
    pub(crate) path: PathBuf,
}

pub(crate) struct LinkedWorktreeObservation {
    pub(crate) candidates: Vec<LinkedWorktreeCandidate>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LinkedWorktreeCatalogSnapshot {
    pub(crate) loading: bool,
    pub(crate) repositories: Vec<LinkedWorktreeRepository>,
}

impl LinkedWorktreeCatalogSnapshot {
    pub(crate) fn repository(&self, common_dir: &Path) -> Option<&LinkedWorktreeRepository> {
        self.repositories
            .iter()
            .find(|repository| same_path(&repository.common_dir, common_dir))
    }

    pub(crate) fn worktree_name(&self, path: &Path) -> Option<String> {
        let worktree = self
            .repositories
            .iter()
            .flat_map(|repository| &repository.worktrees)
            .find(|worktree| same_path(&worktree.path, path))?;
        if worktree.is_main {
            return None;
        }
        worktree
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
    }

    fn agent_destination(&self, path: &Path) -> Option<AgentDestinationMetadata<'_>> {
        self.repositories.iter().find_map(|repository| {
            repository
                .worktrees
                .iter()
                .find(|worktree| same_path(&worktree.path, path))
                .map(|worktree| AgentDestinationMetadata {
                    repository,
                    worktree,
                })
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(repositories: Vec<LinkedWorktreeRepository>) -> Self {
        Self {
            loading: false,
            repositories,
        }
    }
}

struct InventoryCompletion {
    generation: u64,
    repositories: Vec<LinkedWorktreeRepository>,
    discovered: Vec<PathBuf>,
    pruned: Vec<PathBuf>,
    relevant: Vec<PathBuf>,
}

struct RepositoryStatsCompletion {
    generation: u64,
    results: Vec<RepositoryStatsResult>,
}

struct RepositoryStatsResult {
    root: PathBuf,
    revision: u64,
    result: StatsLoadResult,
}

struct RepositoryStatsRequest {
    root: PathBuf,
    revision: u64,
    previous_signature: Option<WorktreeSignature>,
}

#[derive(Clone, Copy)]
struct RepositoryStatsInterest {
    revision: u64,
    standalone: bool,
    refresh_generation: Option<u64>,
}

impl RepositoryStatsInterest {
    fn merge(&mut self, other: Self) {
        debug_assert_eq!(self.revision, other.revision);
        self.standalone |= other.standalone;
        if other.refresh_generation.is_some() {
            self.refresh_generation = other.refresh_generation;
        }
    }
}

struct PendingRepositoryStats {
    root: PathBuf,
    interest: RepositoryStatsInterest,
}

struct RepositoryStatsEntry {
    counts: Option<ChangeStats>,
    signature: Option<WorktreeSignature>,
    checked_at: Option<Instant>,
    recheck_interval: Duration,
    revision: u64,
}

impl RepositoryStatsEntry {
    fn new(counts: Option<ChangeStats>) -> Self {
        Self {
            counts,
            signature: None,
            checked_at: None,
            recheck_interval: MIN_STATS_INTERVAL,
            revision: 0,
        }
    }

    fn due(&self, now: Instant) -> bool {
        self.checked_at
            .is_none_or(|checked_at| now >= checked_at + self.recheck_interval)
    }

    fn back_off(&mut self) {
        self.recheck_interval = self
            .recheck_interval
            .saturating_mul(2)
            .min(MAX_STATS_INTERVAL);
    }
}

struct PersistenceCompletion {
    generation: u64,
    result: Result<(), String>,
}

#[derive(Clone, PartialEq, Eq)]
struct CatalogRefreshKey {
    known: Vec<PathBuf>,
    candidates: Vec<LinkedWorktreeCandidate>,
    stats_roots: Vec<PathBuf>,
    topology_epoch: u64,
}

#[derive(Clone)]
struct CatalogRefreshRequest {
    key: CatalogRefreshKey,
    prioritized_stats_roots: Vec<PathBuf>,
}

struct CatalogRefreshFlight {
    key: CatalogRefreshKey,
    generation: u64,
    inventory_pending: bool,
    stats_pending: bool,
}

impl CatalogRefreshFlight {
    fn settled(&self) -> bool {
        !self.inventory_pending && !self.stats_pending
    }
}

#[derive(Default)]
pub(crate) struct LinkedWorktreeCatalogPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
    pub(crate) worktree_creation: Option<Result<PathBuf, String>>,
}

pub(crate) struct LinkedWorktreeCatalog {
    snapshot: LinkedWorktreeCatalogSnapshot,
    candidates: Vec<LinkedWorktreeCandidate>,
    relevant_common_dirs: Vec<PathBuf>,
    store: KnownRepositoryStore,
    generation: u64,
    topology_epoch: u64,
    active_refresh: Option<CatalogRefreshFlight>,
    pending_refresh: Option<CatalogRefreshRequest>,
    sender: Sender<InventoryCompletion>,
    receiver: Receiver<InventoryCompletion>,
    stats_sender: Sender<RepositoryStatsCompletion>,
    stats_receiver: Receiver<RepositoryStatsCompletion>,
    stats: HashMap<PathBuf, RepositoryStatsEntry>,
    active_stats_root: Option<PathBuf>,
    stats_generation: u64,
    active_stats_generation: Option<u64>,
    stats_in_flight: HashMap<PathBuf, RepositoryStatsInterest>,
    pending_stats: Vec<PendingRepositoryStats>,
    persistence_sender: Sender<PersistenceCompletion>,
    persistence_receiver: Receiver<PersistenceCompletion>,
    persistence_generation: u64,
    active_persistence_generation: Option<u64>,
    pending_persistence: Option<(u64, known_repositories::PersistenceRequest)>,
    deferred_notice: Option<String>,
    worktree_sender: Sender<Result<PathBuf, String>>,
    worktree_receiver: Receiver<Result<PathBuf, String>>,
    creating_worktree: bool,
    #[cfg(test)]
    stats_loader: TestStatsLoader,
}

impl LinkedWorktreeCatalog {
    pub(crate) fn new(store_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (stats_sender, stats_receiver) = mpsc::channel();
        let (persistence_sender, persistence_receiver) = mpsc::channel();
        let (worktree_sender, worktree_receiver) = mpsc::channel();
        let store = KnownRepositoryStore::new(store_path);
        let stats = store
            .recent
            .iter()
            .filter_map(|recent| {
                recent
                    .stats
                    .map(|stats| (recent.root.clone(), RepositoryStatsEntry::new(Some(stats))))
            })
            .collect();
        Self {
            snapshot: LinkedWorktreeCatalogSnapshot::default(),
            candidates: Vec::new(),
            relevant_common_dirs: Vec::new(),
            store,
            generation: 0,
            topology_epoch: 0,
            active_refresh: None,
            pending_refresh: None,
            sender,
            receiver,
            stats_sender,
            stats_receiver,
            stats,
            active_stats_root: None,
            stats_generation: 0,
            active_stats_generation: None,
            stats_in_flight: HashMap::new(),
            pending_stats: Vec::new(),
            persistence_sender,
            persistence_receiver,
            persistence_generation: 0,
            active_persistence_generation: None,
            pending_persistence: None,
            deferred_notice: None,
            worktree_sender,
            worktree_receiver,
            creating_worktree: false,
            #[cfg(test)]
            stats_loader: Arc::new(|root, previous| {
                git::load_change_line_counts(root, previous).map_err(|error| error.to_string())
            }),
        }
    }

    pub(crate) fn repository(&self, common_dir: &Path) -> Option<&LinkedWorktreeRepository> {
        self.snapshot.repository(common_dir)
    }

    pub(crate) fn snapshot(&self) -> &LinkedWorktreeCatalogSnapshot {
        &self.snapshot
    }

    pub(crate) fn change_stats(&self, root: &Path) -> Option<ChangeStats> {
        self.stats.get(root).and_then(|entry| entry.counts)
    }

    pub(crate) fn active_repository_stats_are_current(
        &self,
        root: &Path,
        signature: WorktreeSignature,
    ) -> bool {
        self.active_stats_root.as_deref() == Some(root)
            && self
                .stats
                .get(root)
                .is_some_and(|entry| entry.counts.is_some() && entry.signature == Some(signature))
    }

    pub(crate) fn request_stats(&mut self, roots: impl IntoIterator<Item = PathBuf>) {
        self.request_stats_at(roots, Instant::now());
    }

    pub(crate) fn request_recent_stats(&mut self) {
        let roots = self
            .store
            .recent
            .iter()
            .filter(|recent| recent.common_dir.is_some())
            .map(|recent| recent.root.clone())
            .collect::<Vec<_>>();
        self.request_stats(roots);
    }

    pub(crate) fn observe_active_repository(
        &mut self,
        observation: Option<(PathBuf, ChangeStats, WorktreeSignature)>,
    ) -> bool {
        let Some((root, counts, signature)) = observation else {
            self.active_stats_root = None;
            return false;
        };
        let newly_active = self.active_stats_root.as_ref() != Some(&root);
        self.active_stats_root = Some(root.clone());
        self.pending_stats.retain(|pending| pending.root != root);

        let entry = self
            .stats
            .entry(root.clone())
            .or_insert_with(|| RepositoryStatsEntry::new(None));
        let changed = entry.counts != Some(counts);
        if newly_active || entry.signature != Some(signature) || changed {
            entry.revision = entry.revision.wrapping_add(1);
        }
        entry.counts = Some(counts);
        entry.signature = Some(signature);
        entry.checked_at = Some(Instant::now());
        entry.recheck_interval = MAX_STATS_INTERVAL;

        if changed {
            match self.store.update_stats(&[(root, counts)]) {
                Ok(true) => {
                    if let Err(error) = self.queue_store_persistence() {
                        self.deferred_notice = Some(error);
                    }
                }
                Ok(false) => {}
                Err(error) => self.deferred_notice = Some(error),
            }
        }
        self.update_active_refresh_stats_pending();
        changed
    }

    pub(crate) fn worktree_name(&self, path: &Path) -> Option<String> {
        self.snapshot.worktree_name(path)
    }

    pub(crate) fn agent_destination(&self, path: &Path) -> Option<AgentDestinationMetadata<'_>> {
        self.snapshot.agent_destination(path)
    }

    pub(crate) fn recent_repository_picker_items(&self) -> Vec<RepositoryPickerItem> {
        let repositories = self
            .snapshot
            .repositories
            .iter()
            .map(|repository| (repository.common_dir.as_path(), repository))
            .collect::<HashMap<_, _>>();
        self.store
            .recent
            .iter()
            .map(|recent| {
                let repository = recent
                    .common_dir
                    .as_deref()
                    .and_then(|common_dir| repositories.get(common_dir).copied());
                let label = repository
                    .map(|repository| repository.label.clone())
                    .unwrap_or_else(|| workspace_label(&recent.root));
                let branch = repository
                    .and_then(|repository| {
                        repository
                            .worktrees
                            .iter()
                            .find(|worktree| worktree.path == recent.root)
                    })
                    .and_then(|worktree| worktree.branch.as_deref())
                    .map(|branch| {
                        branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(branch)
                            .to_owned()
                    });
                RepositoryPickerItem {
                    root: recent.root.clone(),
                    label,
                    stats: self.change_stats(&recent.root).or(recent.stats),
                    branch,
                }
            })
            .collect()
    }

    pub(crate) fn remember_workspace(
        &mut self,
        common_dir: Option<&Path>,
        root: &Path,
    ) -> Result<(), String> {
        self.store.remember_and_save(
            common_dir.map(Path::to_owned),
            root.to_owned(),
            &self.relevant_common_dirs,
        )
    }

    pub(crate) fn observe_herdr(&mut self, observation: LinkedWorktreeObservation) -> bool {
        let candidates_changed = self.candidates != observation.candidates;
        self.candidates = observation.candidates;
        candidates_changed
    }

    pub(crate) fn create_worktree_for_branch(
        &mut self,
        repository: PathBuf,
        branch: String,
        remote: bool,
    ) -> Result<(), String> {
        if self.creating_worktree {
            return Err("A linked worktree is already being created".to_owned());
        }
        self.creating_worktree = true;
        let sender = self.worktree_sender.clone();
        thread::spawn(move || {
            let result = git::create_worktree_for_branch(&repository, &branch, remote)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        Ok(())
    }

    pub(crate) fn refresh(&mut self) {
        let request = self.refresh_request();
        if let Some(active) = &self.active_refresh {
            if active.key != request.key || self.pending_refresh.is_some() {
                self.pending_refresh = Some(request);
            }
            return;
        }
        self.start_refresh(request);
    }

    pub(crate) fn refresh_after_topology_change(&mut self) {
        self.topology_epoch = self.topology_epoch.wrapping_add(1);
        self.refresh();
    }

    fn refresh_request(&self) -> CatalogRefreshRequest {
        let prioritized_stats_roots = recent_git_roots(self.store.recent.clone());
        let mut stats_roots = prioritized_stats_roots.clone();
        stats_roots.sort_unstable();
        CatalogRefreshRequest {
            key: CatalogRefreshKey {
                known: self.store.repositories.clone(),
                candidates: self.candidates.clone(),
                stats_roots,
                topology_epoch: self.topology_epoch,
            },
            prioritized_stats_roots,
        }
    }

    fn start_refresh(&mut self, request: CatalogRefreshRequest) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let known = request.key.known.clone();
        let candidates = request.key.candidates.clone();
        let stats_roots = request.prioritized_stats_roots;
        self.active_refresh = Some(CatalogRefreshFlight {
            key: request.key,
            generation,
            inventory_pending: true,
            stats_pending: false,
        });
        let sender = self.sender.clone();
        self.snapshot.loading = true;
        thread::spawn(move || {
            let mut common_dirs = known;
            let mut seen = common_dirs.iter().cloned().collect::<HashSet<_>>();
            let mut candidate_ranks = HashMap::new();
            let mut relevant = Vec::new();
            let mut discovered = Vec::new();
            for (rank, candidate) in candidates.into_iter().enumerate() {
                let Ok(common_dir) = git::common_git_dir(&candidate.path) else {
                    continue;
                };
                if !candidate_ranks.contains_key(&common_dir) {
                    candidate_ranks.insert(common_dir.clone(), rank);
                    relevant.push(common_dir.clone());
                }
                if seen.insert(common_dir.clone()) {
                    discovered.push(common_dir.clone());
                    common_dirs.push(common_dir);
                }
            }
            common_dirs.sort_by_cached_key(|path| {
                (
                    !candidate_ranks.contains_key(path),
                    candidate_ranks.get(path).copied().unwrap_or(usize::MAX),
                    path.to_string_lossy().to_lowercase(),
                )
            });
            let mut pruned = Vec::new();
            let repositories = common_dirs
                .into_iter()
                .filter_map(|common_dir| {
                    let is_candidate = candidate_ranks.contains_key(&common_dir);
                    match git::list_worktrees(&common_dir) {
                        Ok(worktrees) => {
                            let branch_root = worktrees
                                .iter()
                                .find(|worktree| worktree.is_main && !worktree.is_bare)
                                .or_else(|| worktrees.iter().find(|worktree| !worktree.is_bare));
                            let (branches, branch_error) = branch_root.map_or_else(
                                || (Vec::new(), Some("No usable worktree".to_owned())),
                                |worktree| match git::repository_branches(&worktree.path) {
                                    Ok(branches) => (branches, None),
                                    Err(error) => (Vec::new(), Some(error.to_string())),
                                },
                            );
                            Some(LinkedWorktreeRepository {
                                label: repository_label(&common_dir, &worktrees),
                                common_dir,
                                worktrees,
                                branches,
                                branch_error,
                                error: None,
                            })
                        }
                        Err(_) if !is_candidate && !common_dir.exists() => {
                            pruned.push(common_dir);
                            None
                        }
                        Err(error) => Some(LinkedWorktreeRepository {
                            label: repository_label(&common_dir, &[]),
                            common_dir,
                            worktrees: Vec::new(),
                            branches: Vec::new(),
                            branch_error: None,
                            error: Some(error.to_string()),
                        }),
                    }
                })
                .collect();
            let _ = sender.send(InventoryCompletion {
                generation,
                repositories,
                discovered,
                pruned,
                relevant,
            });
        });
        self.request_stats_for_refresh(stats_roots, generation, Instant::now());
        self.update_active_refresh_stats_pending();
    }

    pub(crate) fn poll(&mut self) -> LinkedWorktreeCatalogPoll {
        let mut result = LinkedWorktreeCatalogPoll {
            notice: self.deferred_notice.take(),
            ..LinkedWorktreeCatalogPoll::default()
        };
        while let Ok(completion) = self.receiver.try_recv() {
            if self
                .active_refresh
                .as_ref()
                .is_none_or(|active| active.generation != completion.generation)
            {
                continue;
            }
            self.active_refresh
                .as_mut()
                .expect("matched active catalog refresh")
                .inventory_pending = false;
            if self.pending_refresh.is_some() {
                continue;
            }
            self.snapshot.repositories = completion.repositories;
            self.relevant_common_dirs = completion.relevant;
            match self.store.reconcile_and_save(
                completion.discovered,
                &completion.pruned,
                &self.relevant_common_dirs,
            ) {
                Ok(()) => {}
                Err(error) => result.notice = Some(error),
            }
            result.changed = true;
        }
        while let Ok(completion) = self.stats_receiver.try_recv() {
            if self.active_stats_generation != Some(completion.generation) {
                continue;
            }
            self.active_stats_generation = None;
            let now = Instant::now();
            let mut persisted = Vec::new();
            for completion in completion.results {
                let Some(interest) = self
                    .stats_in_flight
                    .remove(&completion.root)
                    .filter(|interest| interest.revision == completion.revision)
                else {
                    continue;
                };
                let refresh_is_current = interest.refresh_generation.is_some_and(|generation| {
                    self.pending_refresh.is_none()
                        && self
                            .active_refresh
                            .as_ref()
                            .is_some_and(|active| active.generation == generation)
                });
                if !interest.standalone && !refresh_is_current {
                    continue;
                }
                let Some(entry) = self.stats.get_mut(&completion.root) else {
                    continue;
                };
                if entry.revision != completion.revision {
                    continue;
                }
                entry.checked_at = Some(now);
                match completion.result {
                    Ok((signature, Some(counts))) => {
                        let changed = entry.counts != Some(counts);
                        entry.counts = Some(counts);
                        entry.signature = Some(signature);
                        entry.recheck_interval = MIN_STATS_INTERVAL;
                        if changed {
                            result.changed = true;
                            persisted.push((completion.root, counts));
                        }
                    }
                    Ok((signature, None)) => {
                        entry.signature = Some(signature);
                        entry.back_off();
                    }
                    Err(_) => entry.back_off(),
                }
            }
            if !persisted.is_empty() {
                match self.store.update_stats(&persisted) {
                    Ok(true) => {
                        if let Err(error) = self.queue_store_persistence() {
                            result.notice = Some(error);
                        }
                    }
                    Ok(false) => {}
                    Err(error) => result.notice = Some(error),
                }
            }
            self.start_pending_stats();
        }
        while let Ok(completion) = self.persistence_receiver.try_recv() {
            if self.active_persistence_generation != Some(completion.generation) {
                continue;
            }
            self.active_persistence_generation = None;
            if let Err(error) = completion.result {
                result.notice = Some(error);
            }
            if let Some((generation, request)) = self.pending_persistence.take() {
                self.start_persistence(generation, request);
            }
        }
        self.update_active_refresh_stats_pending();
        if self
            .active_refresh
            .as_ref()
            .is_some_and(CatalogRefreshFlight::settled)
        {
            self.active_refresh = None;
            if let Some(request) = self.pending_refresh.take() {
                self.start_refresh(request);
            } else {
                self.snapshot.loading = false;
            }
        }
        if let Ok(completion) = self.worktree_receiver.try_recv() {
            self.creating_worktree = false;
            result.worktree_creation = Some(completion);
        }
        result
    }

    fn request_stats_at(&mut self, roots: impl IntoIterator<Item = PathBuf>, now: Instant) {
        self.request_stats_with_interest(roots, now, true, None);
    }

    fn request_stats_for_refresh(
        &mut self,
        roots: impl IntoIterator<Item = PathBuf>,
        generation: u64,
        now: Instant,
    ) {
        self.request_stats_with_interest(roots, now, false, Some(generation));
    }

    fn request_stats_with_interest(
        &mut self,
        roots: impl IntoIterator<Item = PathBuf>,
        now: Instant,
        standalone: bool,
        refresh_generation: Option<u64>,
    ) {
        for root in roots {
            if self.active_stats_root.as_ref() == Some(&root) {
                continue;
            }
            let entry = self
                .stats
                .entry(root.clone())
                .or_insert_with(|| RepositoryStatsEntry::new(None));
            let interest = RepositoryStatsInterest {
                revision: entry.revision,
                standalone,
                refresh_generation,
            };
            if let Some(in_flight) = self.stats_in_flight.get_mut(&root)
                && in_flight.revision == entry.revision
            {
                in_flight.merge(interest);
                continue;
            }
            if let Some(pending) = self
                .pending_stats
                .iter_mut()
                .find(|pending| pending.root == root && pending.interest.revision == entry.revision)
            {
                pending.interest.merge(interest);
                continue;
            }
            if entry.due(now) {
                self.pending_stats.retain(|pending| pending.root != root);
                self.pending_stats
                    .push(PendingRepositoryStats { root, interest });
            }
        }
        self.start_pending_stats();
        self.update_active_refresh_stats_pending();
    }

    fn start_pending_stats(&mut self) {
        if self.active_stats_generation.is_some() || self.pending_stats.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_stats);
        let requests = pending
            .into_iter()
            .map(|pending| {
                let root = pending.root;
                let entry = self
                    .stats
                    .entry(root.clone())
                    .or_insert_with(|| RepositoryStatsEntry::new(None));
                debug_assert_eq!(pending.interest.revision, entry.revision);
                self.stats_in_flight.insert(root.clone(), pending.interest);
                RepositoryStatsRequest {
                    root,
                    revision: entry.revision,
                    previous_signature: entry.signature,
                }
            })
            .collect::<Vec<_>>();
        self.stats_generation = self.stats_generation.wrapping_add(1);
        let generation = self.stats_generation;
        self.active_stats_generation = Some(generation);
        let sender = self.stats_sender.clone();
        #[cfg(test)]
        let loader = Arc::clone(&self.stats_loader);
        thread::spawn(move || {
            #[cfg(test)]
            let results = load_repository_stats(requests, |root, previous| loader(root, previous));
            #[cfg(not(test))]
            let results = load_repository_stats(requests, |root, previous| {
                git::load_change_line_counts(root, previous).map_err(|error| error.to_string())
            });
            let _ = sender.send(RepositoryStatsCompletion {
                generation,
                results,
            });
        });
    }

    fn update_active_refresh_stats_pending(&mut self) {
        let stats_pending = self.active_refresh.as_ref().is_some_and(|active| {
            active.key.stats_roots.iter().any(|root| {
                self.stats_in_flight
                    .get(root)
                    .is_some_and(|interest| interest.refresh_generation == Some(active.generation))
                    || self.pending_stats.iter().any(|pending| {
                        pending.root == *root
                            && pending.interest.refresh_generation == Some(active.generation)
                    })
            })
        });
        if let Some(active) = self.active_refresh.as_mut() {
            active.stats_pending = stats_pending;
        }
    }

    fn queue_store_persistence(&mut self) -> Result<(), String> {
        let Some(request) = self.store.persistence_request()? else {
            return Ok(());
        };
        self.persistence_generation = self.persistence_generation.wrapping_add(1);
        let generation = self.persistence_generation;
        if self.active_persistence_generation.is_some() {
            self.pending_persistence = Some((generation, request));
        } else {
            self.start_persistence(generation, request);
        }
        Ok(())
    }

    fn start_persistence(
        &mut self,
        generation: u64,
        request: known_repositories::PersistenceRequest,
    ) {
        self.active_persistence_generation = Some(generation);
        let sender = self.persistence_sender.clone();
        thread::spawn(move || {
            let result = known_repositories::persist(request);
            let _ = sender.send(PersistenceCompletion { generation, result });
        });
    }

    #[cfg(test)]
    fn new_with_stats_loader(store_path: Option<PathBuf>, loader: TestStatsLoader) -> Self {
        let mut catalog = Self::new(store_path);
        catalog.stats_loader = loader;
        catalog
    }

    #[cfg(test)]
    pub(crate) fn set_change_stats_for_test(&mut self, root: PathBuf, counts: ChangeStats) {
        let entry = self
            .stats
            .entry(root)
            .or_insert_with(|| RepositoryStatsEntry::new(None));
        entry.counts = Some(counts);
        entry.checked_at = Some(Instant::now());
        entry.recheck_interval = MAX_STATS_INTERVAL;
    }
}

fn load_repository_stats(
    requests: Vec<RepositoryStatsRequest>,
    load: impl Fn(&Path, Option<WorktreeSignature>) -> StatsLoadResult + Sync,
) -> Vec<RepositoryStatsResult> {
    let worker_count = requests.len().min(
        thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(4),
    );
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let next = &next;
            let requests = &requests;
            let load = &load;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(request) = requests.get(index) else {
                        break;
                    };
                    let result = load(&request.root, request.previous_signature);
                    let _ = sender.send((
                        index,
                        RepositoryStatsResult {
                            root: request.root.clone(),
                            revision: request.revision,
                            result,
                        },
                    ));
                }
            });
        }
    });
    drop(sender);
    let mut stats = receiver.into_iter().collect::<Vec<_>>();
    stats.sort_by_key(|(index, _)| *index);
    stats.into_iter().map(|(_, result)| result).collect()
}

fn recent_git_roots(recent: Vec<known_repositories::RecentRepository>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    recent
        .into_iter()
        .filter(|recent| recent.common_dir.is_some())
        .map(|recent| recent.root)
        .filter(|root| seen.insert(root.clone()))
        .collect()
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

fn workspace_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

#[cfg(test)]
mod tests;
