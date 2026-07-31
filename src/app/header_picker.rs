use crate::git::{Branch, LinkedWorktree};

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
    DiffTarget(Branch),
}

#[derive(Debug, Default)]
pub(crate) struct HeaderPicker {
    pub(crate) kind: Option<HeaderPickerKind>,
    pub(crate) items: Vec<HeaderPickerItem>,
    pub(crate) selected: usize,
    pub(crate) message: Option<String>,
}

impl HeaderPicker {
    pub(crate) fn open(
        &mut self,
        kind: HeaderPickerKind,
        items: Vec<HeaderPickerItem>,
        selected: usize,
    ) {
        self.kind = Some(kind);
        self.selected = selected.min(items.len().saturating_sub(1));
        self.items = items;
        self.message = None;
    }

    pub(crate) fn open_message(&mut self, kind: HeaderPickerKind, message: String) {
        self.kind = Some(kind);
        self.items.clear();
        self.selected = 0;
        self.message = Some(message);
    }

    pub(crate) fn close(&mut self) {
        self.kind = None;
        self.items.clear();
        self.selected = 0;
        self.message = None;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.kind.is_some()
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
