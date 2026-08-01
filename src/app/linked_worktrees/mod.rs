use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
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
    pub(crate) group: Option<String>,
    pub(crate) worktrees: Vec<LinkedWorktree>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedWorktreeCandidate {
    pub(crate) path: PathBuf,
    pub(crate) group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrOwnedWorktree {
    pub(crate) path: PathBuf,
    pub(crate) workspace_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum HerdrOwnership {
    #[default]
    Disabled,
    Unverified,
    Verified(Vec<HerdrOwnedWorktree>),
}

pub(crate) struct LinkedWorktreeObservation {
    pub(crate) candidates: Vec<LinkedWorktreeCandidate>,
    pub(crate) ownership: HerdrOwnership,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkedWorktreeRemovalPlan {
    Native { common_dir: PathBuf, path: PathBuf },
    Herdr { workspace_id: String, path: PathBuf },
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LinkedWorktreeCatalogSnapshot {
    revision: u64,
    pub(crate) loading: bool,
    pub(crate) repositories: Vec<LinkedWorktreeRepository>,
    active_path: Option<PathBuf>,
    ownership: HerdrOwnership,
}

impl LinkedWorktreeCatalogSnapshot {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn is_active(&self, path: &Path) -> bool {
        self.active_path
            .as_deref()
            .is_some_and(|active| same_path(active, path))
    }

    pub(crate) fn is_herdr_owned(&self, path: &Path) -> bool {
        matches!(
            &self.ownership,
            HerdrOwnership::Verified(worktrees)
                if worktrees.iter().any(|worktree| same_path(&worktree.path, path))
        )
    }

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

    pub(crate) fn removal_plan(&self, path: &Path) -> Result<LinkedWorktreeRemovalPlan, String> {
        let (repository, worktree) = self
            .repositories
            .iter()
            .find_map(|repository| {
                repository
                    .worktrees
                    .iter()
                    .find(|worktree| same_path(&worktree.path, path))
                    .map(|worktree| (repository, worktree))
            })
            .ok_or_else(|| "This worktree is no longer in the Git inventory".to_owned())?;
        if worktree.is_main {
            return Err("The primary worktree cannot be removed".to_owned());
        }
        if worktree.locked {
            return Err(worktree.locked_reason.as_ref().map_or_else(
                || "Unlock this worktree before removing it".to_owned(),
                |reason| format!("Worktree is locked: {reason}"),
            ));
        }
        if worktree.prunable {
            return Err("This missing worktree requires repository metadata pruning".to_owned());
        }
        if self.is_active(path) {
            return Err("Open another worktree before removing the current one".to_owned());
        }
        match &self.ownership {
            HerdrOwnership::Unverified => {
                Err("Waiting for Herdr to verify linked worktree ownership".to_owned())
            }
            HerdrOwnership::Verified(worktrees) => worktrees
                .iter()
                .find(|worktree| same_path(&worktree.path, path))
                .map_or_else(
                    || {
                        Ok(LinkedWorktreeRemovalPlan::Native {
                            common_dir: repository.common_dir.clone(),
                            path: worktree.path.clone(),
                        })
                    },
                    |owned| {
                        Ok(LinkedWorktreeRemovalPlan::Herdr {
                            workspace_id: owned.workspace_id.clone(),
                            path: worktree.path.clone(),
                        })
                    },
                ),
            HerdrOwnership::Disabled => Ok(LinkedWorktreeRemovalPlan::Native {
                common_dir: repository.common_dir.clone(),
                path: worktree.path.clone(),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        repositories: Vec<LinkedWorktreeRepository>,
        active_path: Option<PathBuf>,
        ownership: HerdrOwnership,
    ) -> Self {
        Self {
            revision: 1,
            loading: false,
            repositories,
            active_path,
            ownership,
        }
    }
}

struct InventoryCompletion {
    generation: u64,
    repositories: Vec<LinkedWorktreeRepository>,
    discovered: Vec<PathBuf>,
    pruned: Vec<PathBuf>,
}

#[derive(Default)]
pub(crate) struct LinkedWorktreeCatalogPoll {
    pub(crate) changed: bool,
    pub(crate) notice: Option<String>,
}

pub(crate) struct LinkedWorktreeCatalog {
    snapshot: LinkedWorktreeCatalogSnapshot,
    candidates: Vec<LinkedWorktreeCandidate>,
    store: KnownRepositoryStore,
    generation: u64,
    sender: Sender<InventoryCompletion>,
    receiver: Receiver<InventoryCompletion>,
}

impl LinkedWorktreeCatalog {
    pub(crate) fn new(store_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            snapshot: LinkedWorktreeCatalogSnapshot::default(),
            candidates: Vec::new(),
            store: KnownRepositoryStore::new(store_path),
            generation: 0,
            sender,
            receiver,
        }
    }

    pub(crate) fn snapshot(&self) -> LinkedWorktreeCatalogSnapshot {
        self.snapshot.clone()
    }

    pub(crate) fn repository(&self, common_dir: &Path) -> Option<&LinkedWorktreeRepository> {
        self.snapshot.repository(common_dir)
    }

    pub(crate) fn worktree_name(&self, path: &Path) -> Option<String> {
        self.snapshot.worktree_name(path)
    }

    pub(crate) fn removal_plan(&self, path: &Path) -> Result<LinkedWorktreeRemovalPlan, String> {
        self.snapshot.removal_plan(path)
    }

    pub(crate) fn recent_repositories(&self) -> impl Iterator<Item = (&Path, &Path)> {
        self.store
            .recent
            .iter()
            .map(|recent| (recent.common_dir.as_path(), recent.root.as_path()))
    }

    pub(crate) fn remember_repository(
        &mut self,
        common_dir: &Path,
        root: &Path,
    ) -> Result<(), String> {
        self.store
            .remember_and_save(common_dir.to_owned(), root.to_owned())
    }

    pub(crate) fn set_active_path(&mut self, path: Option<PathBuf>) {
        if self.snapshot.active_path == path {
            return;
        }
        self.snapshot.active_path = path;
        self.bump_revision();
    }

    pub(crate) fn observe_herdr(&mut self, observation: LinkedWorktreeObservation) -> bool {
        let candidates_changed = self.candidates != observation.candidates;
        let ownership_changed = self.snapshot.ownership != observation.ownership;
        self.candidates = observation.candidates;
        self.snapshot.ownership = observation.ownership;
        if ownership_changed {
            self.bump_revision();
        }
        candidates_changed
    }

    pub(crate) fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let known = self.store.repositories.clone();
        let candidates = self.candidates.clone();
        let sender = self.sender.clone();
        self.snapshot.loading = true;
        self.bump_revision();
        thread::spawn(move || {
            let mut common_dirs = known;
            let mut seen = common_dirs.iter().cloned().collect::<HashSet<_>>();
            let mut candidate_ranks = HashMap::new();
            let mut discovered = Vec::new();
            for (rank, candidate) in candidates.into_iter().enumerate() {
                let Ok(common_dir) = git::common_git_dir(&candidate.path) else {
                    continue;
                };
                candidate_ranks
                    .entry(common_dir.clone())
                    .or_insert((rank, candidate.group));
                if seen.insert(common_dir.clone()) {
                    discovered.push(common_dir.clone());
                    common_dirs.push(common_dir);
                }
            }
            common_dirs.sort_by_cached_key(|path| {
                (
                    !candidate_ranks.contains_key(path),
                    candidate_ranks
                        .get(path)
                        .map_or(usize::MAX, |(rank, _)| *rank),
                    path.to_string_lossy().to_lowercase(),
                )
            });
            let mut pruned = Vec::new();
            let repositories = common_dirs
                .into_iter()
                .filter_map(|common_dir| {
                    let group = candidate_ranks
                        .get(&common_dir)
                        .and_then(|(_, group)| group.clone());
                    let is_candidate = candidate_ranks.contains_key(&common_dir);
                    match git::list_worktrees(&common_dir) {
                        Ok(worktrees) => Some(LinkedWorktreeRepository {
                            label: repository_label(&common_dir, &worktrees),
                            group,
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
                            group,
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
            });
        });
    }

    pub(crate) fn poll(&mut self) -> LinkedWorktreeCatalogPoll {
        let mut result = LinkedWorktreeCatalogPoll::default();
        while let Ok(completion) = self.receiver.try_recv() {
            if completion.generation != self.generation {
                continue;
            }
            self.snapshot.loading = false;
            self.snapshot.repositories = completion.repositories;
            if !completion.discovered.is_empty()
                && let Err(error) = self.store.extend_and_save(completion.discovered)
            {
                result.notice = Some(error);
            }
            if !completion.pruned.is_empty()
                && let Err(error) = self.store.prune_and_save(&completion.pruned)
            {
                result.notice = Some(error);
            }
            self.bump_revision();
            result.changed = true;
        }
        result
    }

    fn bump_revision(&mut self) {
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
    }
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

#[cfg(test)]
mod tests;
