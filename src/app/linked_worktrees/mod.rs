use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use crate::{
    filesystem::same_path,
    git::{self, LinkedWorktree},
};

mod known_repositories;

use known_repositories::KnownRepositoryStore;

#[derive(Debug, Clone)]
pub(crate) struct LinkedWorktreeRepository {
    pub(crate) common_dir: PathBuf,
    pub(crate) label: String,
    pub(crate) worktrees: Vec<LinkedWorktree>,
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
    stats: Vec<(PathBuf, (u64, u64))>,
}

#[derive(Default)]
pub(crate) struct LinkedWorktreeCatalogPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
}

pub(crate) struct LinkedWorktreeCatalog {
    snapshot: LinkedWorktreeCatalogSnapshot,
    candidates: Vec<LinkedWorktreeCandidate>,
    relevant_common_dirs: Vec<PathBuf>,
    store: KnownRepositoryStore,
    generation: u64,
    sender: Sender<InventoryCompletion>,
    receiver: Receiver<InventoryCompletion>,
    stats_sender: Sender<RepositoryStatsCompletion>,
    stats_receiver: Receiver<RepositoryStatsCompletion>,
}

impl LinkedWorktreeCatalog {
    pub(crate) fn new(store_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (stats_sender, stats_receiver) = mpsc::channel();
        Self {
            snapshot: LinkedWorktreeCatalogSnapshot::default(),
            candidates: Vec::new(),
            relevant_common_dirs: Vec::new(),
            store: KnownRepositoryStore::new(store_path),
            generation: 0,
            sender,
            receiver,
            stats_sender,
            stats_receiver,
        }
    }

    pub(crate) fn repository(&self, common_dir: &Path) -> Option<&LinkedWorktreeRepository> {
        self.snapshot.repository(common_dir)
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
                    stats: recent.stats,
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

    pub(crate) fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let known = self.store.repositories.clone();
        let recent = self.store.recent.clone();
        let candidates = self.candidates.clone();
        let sender = self.sender.clone();
        let stats_sender = self.stats_sender.clone();
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
                        Ok(worktrees) => Some(LinkedWorktreeRepository {
                            label: repository_label(&common_dir, &worktrees),
                            common_dir,
                            worktrees,
                            error: None,
                        }),
                        Err(_) if !is_candidate && !common_dir.exists() => {
                            pruned.push(common_dir);
                            None
                        }
                        Err(error) => Some(LinkedWorktreeRepository {
                            label: repository_label(&common_dir, &[]),
                            common_dir,
                            worktrees: Vec::new(),
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
        if !recent.is_empty() {
            thread::spawn(move || {
                let stats = load_repository_stats(recent);
                let _ = stats_sender.send(RepositoryStatsCompletion { generation, stats });
            });
        }
    }

    pub(crate) fn poll(&mut self) -> LinkedWorktreeCatalogPoll {
        let mut result = LinkedWorktreeCatalogPoll::default();
        while let Ok(completion) = self.receiver.try_recv() {
            if completion.generation != self.generation {
                continue;
            }
            self.snapshot.loading = false;
            self.snapshot.repositories = completion.repositories;
            self.relevant_common_dirs = completion.relevant;
            if let Err(error) = self.store.reconcile_and_save(
                completion.discovered,
                &completion.pruned,
                &self.relevant_common_dirs,
            ) {
                result.notice = Some(error);
            }
            result.changed = true;
        }
        while let Ok(completion) = self.stats_receiver.try_recv() {
            if completion.generation != self.generation {
                continue;
            }
            match self.store.update_stats_and_save(&completion.stats) {
                Ok(true) => {
                    result.changed = true;
                }
                Ok(false) => {}
                Err(error) => result.notice = Some(error),
            }
        }
        result
    }
}

fn load_repository_stats(
    recent: Vec<known_repositories::RecentRepository>,
) -> Vec<(PathBuf, (u64, u64))> {
    let roots = recent_git_roots(recent);
    let worker_count = roots.len().min(
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
            let roots = &roots;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(root) = roots.get(index) else {
                        break;
                    };
                    if let Ok(stats) = git::load_change_line_counts(root) {
                        let _ = sender.send((index, root.clone(), stats));
                    }
                }
            });
        }
    });
    drop(sender);
    let mut stats = receiver.into_iter().collect::<Vec<_>>();
    stats.sort_by_key(|(index, _, _)| *index);
    stats
        .into_iter()
        .map(|(_, root, stats)| (root, stats))
        .collect()
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
