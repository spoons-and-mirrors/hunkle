use crate::git::{Branch, LinkedWorktree};

use super::TextInput;

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderPickerKind {
    Repositories,
    Worktrees,
    Branches,
    DiffTargets,
}

#[derive(Debug, Clone)]
pub(crate) enum HeaderPickerItem {
    Repository { common_dir: PathBuf, path: PathBuf },
    Worktree(LinkedWorktree),
    Branch(Branch),
    BranchBase(Branch),
    DiffTarget(Branch),
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
