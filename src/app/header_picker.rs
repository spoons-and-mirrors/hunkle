use crate::git::{self, Branch, LinkedWorktree};

use super::{TextInput, linked_worktrees::RepositoryPickerItem};

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
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchPickerStep {
    #[default]
    Branches,
    Base,
    Name,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryPickerStep {
    #[default]
    Repositories,
    Clone,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloneField {
    #[default]
    Directory,
    Url,
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
    pub(crate) repository_step: RepositoryPickerStep,
    pub(crate) clone_directory: TextInput,
    pub(crate) clone_url: TextInput,
    pub(crate) clone_field: CloneField,
    clone_rx: Option<Receiver<Result<PathBuf, String>>>,
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
        self.repository_step = RepositoryPickerStep::Repositories;
        self.clone_field = CloneField::Directory;
        self.clone_directory.clear();
        self.clone_url.clear();
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
        self.repository_step = RepositoryPickerStep::Repositories;
        self.clone_field = CloneField::Directory;
        self.clone_directory.clear();
        self.clone_url.clear();
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
        self.repository_step = RepositoryPickerStep::Repositories;
        self.clone_field = CloneField::Directory;
        self.clone_directory.clear();
        self.clone_url.clear();
    }

    pub(crate) fn start_change_details(&mut self) {
        self.change_details_rx = None;
        if self.kind != Some(HeaderPickerKind::Worktrees) {
            return;
        }
        let paths = self
            .all_items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                HeaderPickerItem::Worktree { worktree, stats } if stats.is_none() => {
                    Some((index, worktree.path.clone()))
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
                for (index, path) in paths {
                    let stats = git::load_change_line_counts(&path).ok();
                    if sender
                        .send(ChangeDetailsCompletion { index, path, stats })
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
        for completion in completions {
            Self::update_worktree_stats_at(
                &mut self.all_items,
                completion.index,
                &completion.path,
                completion.stats,
            );
            let visible_changed = if self.query.is_empty() {
                Self::update_worktree_stats_at(
                    &mut self.items,
                    completion.index,
                    &completion.path,
                    completion.stats,
                )
            } else {
                Self::update_worktree_stats(&mut self.items, &completion.path, completion.stats)
            };
            let completion_is_visible = !self.query.is_empty()
                || (completion.index >= self.scroll
                    && completion.index < self.scroll.saturating_add(self.viewport_rows));
            changed |= visible_changed && completion_is_visible;
        }
        if disconnected {
            self.change_details_rx = None;
        }
        changed
    }

    pub(crate) fn sync_repository_details(&mut self, details: &[RepositoryPickerItem]) -> bool {
        if self.kind != Some(HeaderPickerKind::Repositories) {
            return false;
        }
        let details = details
            .iter()
            .map(|detail| (detail.common_dir.as_path(), detail))
            .collect::<std::collections::HashMap<_, _>>();
        let (all_changed, searchable_changed) =
            Self::update_repository_details(&mut self.all_items, &details);
        if searchable_changed {
            self.searchable_items = self
                .all_items
                .iter()
                .map(HeaderPickerItem::normalized_search_text)
                .collect();
            self.apply_filter();
        } else {
            Self::update_repository_details(&mut self.items, &details);
        }
        all_changed
    }

    fn update_repository_details(
        items: &mut [HeaderPickerItem],
        details: &std::collections::HashMap<&Path, &RepositoryPickerItem>,
    ) -> (bool, bool) {
        let mut changed = false;
        let mut searchable_changed = false;
        for item in items {
            let HeaderPickerItem::Repository {
                common_dir,
                label,
                stats,
                branch,
                ..
            } = item
            else {
                continue;
            };
            let Some(detail) = details.get(common_dir.as_path()) else {
                continue;
            };
            if label != &detail.label {
                label.clone_from(&detail.label);
                changed = true;
                searchable_changed = true;
            }
            if let Some(detail_stats) = detail.stats
                && *stats != Some(detail_stats)
            {
                *stats = Some(detail_stats);
                changed = true;
            }
            if branch != &detail.branch {
                branch.clone_from(&detail.branch);
                changed = true;
                searchable_changed = true;
            }
        }
        (changed, searchable_changed)
    }

    fn update_worktree_stats(
        items: &mut [HeaderPickerItem],
        path: &Path,
        stats: Option<(u64, u64)>,
    ) -> bool {
        let mut changed = false;
        for item in items {
            if let HeaderPickerItem::Worktree {
                worktree,
                stats: item_stats,
            } = item
                && worktree.path == path
                && let Some(stats) = stats
                && *item_stats != Some(stats)
            {
                *item_stats = Some(stats);
                changed = true;
            }
        }
        changed
    }

    fn update_worktree_stats_at(
        items: &mut [HeaderPickerItem],
        index: usize,
        path: &Path,
        stats: Option<(u64, u64)>,
    ) -> bool {
        let Some(item) = items.get_mut(index) else {
            return false;
        };
        Self::update_worktree_stats(std::slice::from_mut(item), path, stats)
    }

    pub(crate) fn is_open(&self) -> bool {
        self.kind.is_some()
    }

    pub(crate) fn naming_branch(&self) -> bool {
        self.kind == Some(HeaderPickerKind::Branches) && self.branch_step == BranchPickerStep::Name
    }

    pub(crate) fn cloning_repository(&self) -> bool {
        self.kind == Some(HeaderPickerKind::Repositories)
            && self.repository_step == RepositoryPickerStep::Clone
    }

    pub(crate) fn filtering(&self) -> bool {
        self.is_open() && self.searchable && !self.naming_branch() && !self.cloning_repository()
    }

    pub(crate) fn begin_clone(&mut self, directory: &Path) {
        self.repository_step = RepositoryPickerStep::Clone;
        self.clone_directory.set(directory.display().to_string());
        self.clone_directory.focus();
        self.clone_url.clear();
        self.clone_field = CloneField::Directory;
        self.message = None;
    }

    pub(crate) fn set_clone_field(&mut self, field: CloneField) {
        self.clone_field = field;
        match field {
            CloneField::Directory => self.clone_directory.focus(),
            CloneField::Url => self.clone_url.focus(),
        }
    }

    pub(crate) fn clone_input_mut(&mut self) -> &mut TextInput {
        match self.clone_field {
            CloneField::Directory => &mut self.clone_directory,
            CloneField::Url => &mut self.clone_url,
        }
    }

    pub(crate) fn clone_running(&self) -> bool {
        self.clone_rx.is_some()
    }

    pub(crate) fn start_clone(&mut self, directory: PathBuf, url: String) -> bool {
        if self.clone_running() {
            return false;
        }
        let (sender, receiver) = mpsc::channel();
        self.clone_rx = Some(receiver);
        thread::spawn(move || {
            let result = git::clone_repository(&directory, &url).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        true
    }

    pub(crate) fn poll_clone(&mut self) -> Option<Result<PathBuf, String>> {
        let result = match self.clone_rx.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("repository clone worker stopped".to_string()),
        };
        self.clone_rx = None;
        Some(result)
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

    #[test]
    fn synchronizes_cached_repository_details_as_one_batch() {
        let mut picker = HeaderPicker::default();
        picker.open(
            HeaderPickerKind::Repositories,
            vec![
                HeaderPickerItem::Repository {
                    common_dir: PathBuf::from("/tmp/alpha/.git"),
                    path: PathBuf::from("/tmp/alpha"),
                    label: "alpha".to_owned(),
                    stats: None,
                    branch: None,
                },
                HeaderPickerItem::Repository {
                    common_dir: PathBuf::from("/tmp/bravo/.git"),
                    path: PathBuf::from("/tmp/bravo"),
                    label: "bravo".to_owned(),
                    stats: None,
                    branch: None,
                },
            ],
            0,
        );

        assert!(picker.sync_repository_details(&[
            RepositoryPickerItem {
                common_dir: PathBuf::from("/tmp/alpha/.git"),
                root: PathBuf::from("/tmp/alpha"),
                label: "alpha".to_owned(),
                stats: Some((12, 3)),
                branch: Some("main".to_owned()),
            },
            RepositoryPickerItem {
                common_dir: PathBuf::from("/tmp/bravo/.git"),
                root: PathBuf::from("/tmp/bravo"),
                label: "bravo".to_owned(),
                stats: Some((7, 2)),
                branch: Some("topic".to_owned()),
            },
        ]));
        assert!(matches!(
            &picker.items[0],
            HeaderPickerItem::Repository {
                stats: Some((12, 3)),
                branch: Some(branch),
                ..
            } if branch == "main"
        ));
        assert!(matches!(
            &picker.items[1],
            HeaderPickerItem::Repository {
                stats: Some((7, 2)),
                branch: Some(branch),
                ..
            } if branch == "topic"
        ));
    }
}
