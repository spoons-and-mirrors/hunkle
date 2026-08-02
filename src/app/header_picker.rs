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
    AgentDestinations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentDestinationKind {
    Repository,
    Worktree,
}

#[derive(Debug, Clone)]
pub(crate) enum HeaderPickerItem {
    Repository {
        common_dir: PathBuf,
        path: PathBuf,
        label: String,
        stats: Option<(u64, u64)>,
        branch: Option<String>,
    },
    Worktree {
        worktree: LinkedWorktree,
        stats: Option<(u64, u64)>,
    },
    Branch(Branch),
    BranchBase(Branch),
    DiffTarget(Branch),
    AgentDestination {
        path: PathBuf,
        repository: String,
        branch: String,
        kind: AgentDestinationKind,
    },
}

#[derive(Debug)]
struct ChangeDetailsCompletion {
    index: usize,
    path: PathBuf,
    stats: Option<(u64, u64)>,
    branch: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchPickerStep {
    #[default]
    Branches,
    Base,
    Name,
}

#[derive(Debug, Default)]
pub(crate) struct HeaderPicker {
    pub(crate) kind: Option<HeaderPickerKind>,
    pub(crate) items: Vec<HeaderPickerItem>,
    all_items: Vec<HeaderPickerItem>,
    searchable_items: Vec<String>,
    default_selected: usize,
    searchable: bool,
    pub(crate) selected: usize,
    scroll: usize,
    viewport_rows: usize,
    scroll_follows_selection: bool,
    pub(crate) message: Option<String>,
    pub(crate) query: TextInput,
    pub(crate) branch_step: BranchPickerStep,
    pub(crate) branch_base: Option<Branch>,
    pub(crate) branch_name: TextInput,
    change_details_rx: Option<Receiver<ChangeDetailsCompletion>>,
}

impl HeaderPicker {
    pub(crate) fn open(
        &mut self,
        kind: HeaderPickerKind,
        items: Vec<HeaderPickerItem>,
        selected: usize,
    ) {
        self.change_details_rx = None;
        self.kind = Some(kind);
        self.default_selected = selected.min(items.len().saturating_sub(1));
        self.searchable = true;
        self.selected = self.default_selected;
        self.scroll = 0;
        self.viewport_rows = 0;
        self.scroll_follows_selection = true;
        self.searchable_items = items
            .iter()
            .map(HeaderPickerItem::normalized_search_text)
            .collect();
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
        self.change_details_rx = None;
        self.kind = Some(kind);
        self.items.clear();
        self.all_items.clear();
        self.searchable_items.clear();
        self.default_selected = 0;
        self.searchable = false;
        self.selected = 0;
        self.scroll = 0;
        self.viewport_rows = 0;
        self.scroll_follows_selection = true;
        self.message = Some(message);
        self.query.clear();
        self.query.focus();
        self.branch_step = BranchPickerStep::Branches;
        self.branch_base = None;
        self.branch_name.clear();
    }

    pub(crate) fn open_branch_bases(&mut self, items: Vec<HeaderPickerItem>, selected: usize) {
        self.change_details_rx = None;
        self.kind = Some(HeaderPickerKind::Branches);
        self.default_selected = selected.min(items.len().saturating_sub(1));
        self.searchable = true;
        self.selected = self.default_selected;
        self.scroll = 0;
        self.viewport_rows = 0;
        self.scroll_follows_selection = true;
        self.searchable_items = items
            .iter()
            .map(HeaderPickerItem::normalized_search_text)
            .collect();
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
        self.change_details_rx = None;
        self.kind = Some(HeaderPickerKind::Branches);
        self.selected = 0;
        self.scroll = 0;
        self.viewport_rows = 0;
        self.scroll_follows_selection = true;
        self.items.clear();
        self.all_items.clear();
        self.searchable_items.clear();
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
        self.change_details_rx = None;
        self.kind = None;
        self.items.clear();
        self.all_items.clear();
        self.searchable_items.clear();
        self.default_selected = 0;
        self.searchable = false;
        self.selected = 0;
        self.scroll = 0;
        self.viewport_rows = 0;
        self.scroll_follows_selection = true;
        self.message = None;
        self.query.clear();
        self.branch_step = BranchPickerStep::Branches;
        self.branch_base = None;
        self.branch_name.clear();
    }

    pub(crate) fn start_change_details(&mut self) {
        self.change_details_rx = None;
        if !matches!(
            self.kind,
            Some(HeaderPickerKind::Repositories | HeaderPickerKind::Worktrees)
        ) {
            return;
        }
        let paths = self
            .all_items
            .iter()
            .filter_map(|item| match item {
                HeaderPickerItem::Repository {
                    path,
                    stats,
                    branch,
                    ..
                } => Some((path.clone(), stats.is_none(), branch.is_none())),
                HeaderPickerItem::Worktree { worktree, stats } => {
                    Some((worktree.path.clone(), stats.is_none(), false))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.change_details_rx = Some(receiver);
        let _ = thread::Builder::new()
            .name("header-picker-change-details".to_owned())
            .spawn(move || {
                for (index, (path, load_stats, load_branch)) in paths.into_iter().enumerate() {
                    let stats = load_stats
                        .then(|| git::load_change_line_counts(&path).ok())
                        .flatten();
                    let branch = load_branch.then(|| git::branch_name(&path).ok()).flatten();
                    if sender
                        .send(ChangeDetailsCompletion {
                            index,
                            path,
                            stats,
                            branch,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
    }

    pub(crate) fn poll_change_details(&mut self) -> bool {
        let Some(receiver) = self.change_details_rx.as_ref() else {
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
        let mut searchable_changed = false;
        for completion in completions {
            let (_, all_searchable_changed) = Self::update_change_details_at(
                &mut self.all_items,
                completion.index,
                &completion.path,
                completion.stats,
                completion.branch.as_deref(),
            );
            if all_searchable_changed
                && let Some(item) = self.all_items.get(completion.index)
                && let Some(searchable) = self.searchable_items.get_mut(completion.index)
            {
                *searchable = item.normalized_search_text();
            }
            let (visible_changed, visible_searchable_changed) = if self.query.is_empty() {
                Self::update_change_details_at(
                    &mut self.items,
                    completion.index,
                    &completion.path,
                    completion.stats,
                    completion.branch.as_deref(),
                )
            } else {
                Self::update_change_details(
                    &mut self.items,
                    &completion.path,
                    completion.stats,
                    completion.branch.as_deref(),
                )
            };
            let completion_is_visible = !self.query.is_empty()
                || (completion.index >= self.scroll
                    && completion.index < self.scroll.saturating_add(self.viewport_rows));
            changed |= visible_changed && completion_is_visible;
            searchable_changed |= all_searchable_changed || visible_searchable_changed;
        }
        if disconnected {
            self.change_details_rx = None;
        }
        if searchable_changed && !self.query.is_empty() {
            self.apply_filter();
            changed = true;
        }
        changed
    }

    fn update_change_details(
        items: &mut [HeaderPickerItem],
        path: &Path,
        stats: Option<(u64, u64)>,
        branch: Option<&str>,
    ) -> (bool, bool) {
        let mut changed = false;
        let mut searchable_changed = false;
        for item in items {
            match item {
                HeaderPickerItem::Repository {
                    path: item_path,
                    stats: item_stats,
                    branch: item_branch,
                    ..
                } if item_path == path => {
                    if let Some(stats) = stats
                        && *item_stats != Some(stats)
                    {
                        *item_stats = Some(stats);
                        changed = true;
                    }
                    if let Some(branch) = branch
                        && item_branch.as_deref() != Some(branch)
                    {
                        *item_branch = Some(branch.to_owned());
                        changed = true;
                        searchable_changed = true;
                    }
                }
                HeaderPickerItem::Worktree {
                    worktree,
                    stats: item_stats,
                } if worktree.path == path => {
                    if let Some(stats) = stats
                        && *item_stats != Some(stats)
                    {
                        *item_stats = Some(stats);
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        (changed, searchable_changed)
    }

    fn update_change_details_at(
        items: &mut [HeaderPickerItem],
        index: usize,
        path: &Path,
        stats: Option<(u64, u64)>,
        branch: Option<&str>,
    ) -> (bool, bool) {
        let Some(item) = items.get_mut(index) else {
            return (false, false);
        };
        Self::update_change_details(std::slice::from_mut(item), path, stats, branch)
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
            self.scroll = 0;
            self.scroll_follows_selection = true;
            self.ensure_selection_visible();
            return;
        }
        self.items = self
            .all_items
            .iter()
            .zip(&self.searchable_items)
            .filter(|(_, searchable)| terms.iter().all(|term| searchable.contains(term)))
            .map(|(item, _)| item)
            .cloned()
            .collect();
        self.selected = 0;
        self.scroll = 0;
        self.scroll_follows_selection = true;
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.items.len() - 1);
        self.scroll_follows_selection = true;
        self.ensure_selection_visible();
    }

    pub(crate) fn scroll_by(&mut self, delta: isize) {
        self.scroll_follows_selection = false;
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.maximum_scroll());
    }

    pub(crate) fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows;
        self.scroll = self.scroll.min(self.maximum_scroll());
        if self.scroll_follows_selection {
            self.ensure_selection_visible();
        }
    }

    pub(crate) fn visible_start(&self) -> usize {
        self.scroll
    }

    fn maximum_scroll(&self) -> usize {
        self.items.len().saturating_sub(self.viewport_rows)
    }

    fn ensure_selection_visible(&mut self) {
        if self.viewport_rows == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll.saturating_add(self.viewport_rows) {
            self.scroll = self
                .selected
                .saturating_add(1)
                .saturating_sub(self.viewport_rows);
        }
        self.scroll = self.scroll.min(self.maximum_scroll());
    }
}

impl HeaderPickerItem {
    fn normalized_search_text(&self) -> String {
        self.searchable_text().to_lowercase()
    }

    fn searchable_text(&self) -> String {
        match self {
            Self::Repository {
                path,
                label,
                branch,
                ..
            } => {
                format!(
                    "{label} {} {}",
                    path.display(),
                    branch.as_deref().unwrap_or_default()
                )
            }
            Self::Worktree { worktree, .. } => format!(
                "{} {}",
                worktree.path.display(),
                worktree.branch.as_deref().unwrap_or_default()
            ),
            Self::Branch(branch) | Self::BranchBase(branch) | Self::DiffTarget(branch) => {
                branch.name.clone()
            }
            Self::AgentDestination {
                path,
                repository,
                branch,
                ..
            } => format!("{} {repository} {branch}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_moves_the_viewport_without_moving_selection() {
        let items = (0..12)
            .map(|index| HeaderPickerItem::AgentDestination {
                path: PathBuf::from(format!("/tmp/repository-{index}")),
                repository: format!("repository-{index}"),
                branch: "main".to_owned(),
                kind: AgentDestinationKind::Repository,
            })
            .collect();
        let mut picker = HeaderPicker::default();
        picker.open(HeaderPickerKind::AgentDestinations, items, 0);
        picker.set_viewport_rows(3);

        picker.scroll_by(2);
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.visible_start(), 2);

        picker.move_selection(1);
        assert_eq!(picker.selected, 1);
        assert_eq!(picker.visible_start(), 1);
    }

    #[test]
    fn filters_agent_destinations_by_branch() {
        let mut picker = HeaderPicker::default();
        picker.open(
            HeaderPickerKind::AgentDestinations,
            vec![HeaderPickerItem::AgentDestination {
                path: PathBuf::from("/tmp/checkout"),
                repository: "repository".to_owned(),
                branch: "topic-branch".to_owned(),
                kind: AgentDestinationKind::Worktree,
            }],
            0,
        );
        picker.query.insert("topic");
        picker.apply_filter();

        assert_eq!(picker.items.len(), 1);
    }

    #[test]
    fn filters_repositories_by_precomputed_label() {
        let mut picker = HeaderPicker::default();
        picker.open(
            HeaderPickerKind::Repositories,
            vec![HeaderPickerItem::Repository {
                common_dir: PathBuf::from("/tmp/catalog/.git"),
                path: PathBuf::from("/tmp/catalog"),
                label: "project-catalog".to_owned(),
                stats: None,
                branch: Some("main".to_owned()),
            }],
            0,
        );
        picker.query.insert("project-catalog");
        picker.apply_filter();

        assert_eq!(picker.items.len(), 1);
    }
}
