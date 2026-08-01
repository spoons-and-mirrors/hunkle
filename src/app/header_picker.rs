use crate::git::{self, Branch, LinkedWorktree};

use super::TextInput;

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderPickerKind {
    Repositories,
    Worktrees,
    Branches,
    DiffTargets,
}

#[derive(Debug, Clone)]
pub(crate) enum HeaderPickerItem {
    Repository {
        common_dir: PathBuf,
        path: PathBuf,
        stats: Option<(u64, u64)>,
    },
    Worktree(LinkedWorktree),
    Branch(Branch),
    BranchBase(Branch),
    DiffTarget(Branch),
}

#[derive(Debug)]
struct RepositoryStatsCompletion {
    common_dir: PathBuf,
    stats: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchPickerStep {
    Branches,
    Base,
    Name,
}

#[derive(Debug, Default)]
pub(crate) struct HeaderPicker {
    pub(crate) kind: Option<HeaderPickerKind>,
    pub(crate) items: Vec<HeaderPickerItem>,
    all_items: Vec<HeaderPickerItem>,
    default_selected: usize,
    searchable: bool,
    pub(crate) selected: usize,
    pub(crate) message: Option<String>,
    pub(crate) query: TextInput,
    pub(crate) branch_step: BranchPickerStep,
    pub(crate) branch_base: Option<Branch>,
    pub(crate) branch_name: TextInput,
    repository_stats_rx: Option<Receiver<RepositoryStatsCompletion>>,
}

impl Default for BranchPickerStep {
    fn default() -> Self {
        Self::Branches
    }
}

impl HeaderPicker {
    pub(crate) fn open(
        &mut self,
        kind: HeaderPickerKind,
        items: Vec<HeaderPickerItem>,
        selected: usize,
    ) {
        self.repository_stats_rx = None;
        self.kind = Some(kind);
        self.default_selected = selected.min(items.len().saturating_sub(1));
        self.searchable = true;
        self.selected = self.default_selected;
        self.items.clone_from(&items);
        self.all_items = items;
        self.message = None;
        self.query.clear();
        self.query.focus();
        self.branch_step = BranchPickerStep::Branches;
        self.branch_base = None;
        self.branch_name.clear();
    }

    pub(crate) fn open_message(&mut self, kind: HeaderPickerKind, message: String) {
        self.repository_stats_rx = None;
        self.kind = Some(kind);
        self.items.clear();
        self.all_items.clear();
        self.default_selected = 0;
        self.searchable = false;
        self.selected = 0;
        self.message = Some(message);
        self.query.clear();
        self.query.focus();
        self.branch_step = BranchPickerStep::Branches;
        self.branch_base = None;
        self.branch_name.clear();
    }

    pub(crate) fn open_branch_bases(&mut self, items: Vec<HeaderPickerItem>, selected: usize) {
        self.repository_stats_rx = None;
        self.kind = Some(HeaderPickerKind::Branches);
        self.default_selected = selected.min(items.len().saturating_sub(1));
        self.searchable = true;
        self.selected = self.default_selected;
        self.items.clone_from(&items);
        self.all_items = items;
        self.message = None;
        self.query.clear();
        self.query.focus();
        self.branch_step = BranchPickerStep::Base;
        self.branch_base = None;
        self.branch_name.clear();
    }

    pub(crate) fn open_branch_name(&mut self, base: Branch) {
        self.repository_stats_rx = None;
        self.kind = Some(HeaderPickerKind::Branches);
        self.selected = 0;
        self.items.clear();
        self.all_items.clear();
        self.default_selected = 0;
        self.searchable = false;
        self.message = None;
        self.query.clear();
        self.branch_step = BranchPickerStep::Name;
        self.branch_base = Some(base);
        self.branch_name.clear();
        self.branch_name.focus();
    }

    pub(crate) fn close(&mut self) {
        self.repository_stats_rx = None;
        self.kind = None;
        self.items.clear();
        self.all_items.clear();
        self.default_selected = 0;
        self.searchable = false;
        self.selected = 0;
        self.message = None;
        self.query.clear();
        self.branch_step = BranchPickerStep::Branches;
        self.branch_base = None;
        self.branch_name.clear();
    }

    pub(crate) fn start_repository_stats(&mut self) {
        self.repository_stats_rx = None;
        if self.kind != Some(HeaderPickerKind::Repositories) {
            return;
        }
        let repositories = self
            .all_items
            .iter()
            .filter_map(|item| match item {
                HeaderPickerItem::Repository {
                    common_dir, path, ..
                } => Some((common_dir.clone(), path.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        if repositories.is_empty() {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.repository_stats_rx = Some(receiver);
        let _ = thread::Builder::new()
            .name("repository-picker-stats".to_owned())
            .spawn(move || {
                for (common_dir, path) in repositories {
                    let stats = git::load_change_line_counts(&path).ok();
                    if sender
                        .send(RepositoryStatsCompletion { common_dir, stats })
                        .is_err()
                    {
                        break;
                    }
                }
            });
    }

    pub(crate) fn poll_repository_stats(&mut self) -> bool {
        let Some(receiver) = self.repository_stats_rx.as_ref() else {
            return false;
        };
        let mut completions = Vec::new();
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(completion) => completions.push(completion),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        let mut changed = false;
        for completion in completions {
            let Some(stats) = completion.stats else {
                continue;
            };
            changed |=
                Self::update_repository_stats(&mut self.all_items, &completion.common_dir, stats);
            changed |=
                Self::update_repository_stats(&mut self.items, &completion.common_dir, stats);
        }
        if disconnected {
            self.repository_stats_rx = None;
        }
        changed
    }

    fn update_repository_stats(
        items: &mut [HeaderPickerItem],
        common_dir: &Path,
        stats: (u64, u64),
    ) -> bool {
        let mut changed = false;
        for item in items {
            if let HeaderPickerItem::Repository {
                common_dir: item_common_dir,
                stats: item_stats,
                ..
            } = item
                && item_common_dir == common_dir
                && *item_stats != Some(stats)
            {
                *item_stats = Some(stats);
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn is_open(&self) -> bool {
        self.kind.is_some()
    }

    pub(crate) fn naming_branch(&self) -> bool {
        self.kind == Some(HeaderPickerKind::Branches) && self.branch_step == BranchPickerStep::Name
    }

    pub(crate) fn filtering(&self) -> bool {
        self.is_open() && self.searchable && !self.naming_branch()
    }

    pub(crate) fn apply_filter(&mut self) {
        let terms = self
            .query
            .text()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if terms.is_empty() {
            self.items.clone_from(&self.all_items);
            self.selected = self
                .default_selected
                .min(self.items.len().saturating_sub(1));
            return;
        }
        self.items = self
            .all_items
            .iter()
            .filter(|item| {
                let searchable = item.searchable_text().to_lowercase();
                terms.iter().all(|term| searchable.contains(term))
            })
            .cloned()
            .collect();
        self.selected = 0;
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.items.len() - 1);
    }

    pub(crate) fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
        }
    }
}

impl HeaderPickerItem {
    fn searchable_text(&self) -> String {
        match self {
            Self::Repository { path, .. } => path.display().to_string(),
            Self::Worktree(worktree) => format!(
                "{} {}",
                worktree.path.display(),
                worktree.branch.as_deref().unwrap_or_default()
            ),
            Self::Branch(branch) | Self::BranchBase(branch) | Self::DiffTarget(branch) => {
                branch.name.clone()
            }
        }
    }
}
