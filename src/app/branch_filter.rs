use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;

use crate::git::Commit;

use super::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchFilterEffect {
    Close,
    Changed,
}

#[derive(Debug, Default)]
pub(crate) struct BranchFilter {
    root: Option<PathBuf>,
    hidden_branches: Vec<String>,
    visible_indices: Vec<usize>,
    pub input: TextInput,
    pub state: ListState,
}

impl BranchFilter {
    pub fn open(&mut self, root: &Path, commits: &[Commit]) {
        self.sync(root, commits);
        self.input.focus();
        if self.state.selected().is_none() && !self.hidden_branches.is_empty() {
            self.state.select(Some(0));
        }
    }

    pub fn sync(&mut self, root: &Path, commits: &[Commit]) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_path_buf());
            self.hidden_branches.clear();
            self.input.clear();
            self.state = ListState::default();
        }
        self.rebuild_visible_indices(commits);
    }

    pub fn hidden_branches(&self) -> &[String] {
        &self.hidden_branches
    }

    pub fn visible_indices(&self) -> &[usize] {
        &self.visible_indices
    }

    pub fn is_ref_hidden(&self, reference: &str) -> bool {
        self.hidden_branches
            .iter()
            .any(|branch| ref_matches_branch(reference, branch))
    }

    pub fn add_hidden_branch(&mut self, branch: &str, commits: &[Commit]) -> bool {
        let branch = branch.trim();
        if branch.is_empty() {
            return false;
        }
        let branch = branch
            .strip_prefix("refs/heads/")
            .unwrap_or(branch)
            .strip_prefix("refs/remotes/")
            .unwrap_or(branch);
        if self.hidden_branches.iter().any(|b| b == branch) {
            self.input.clear();
            return false;
        }
        self.hidden_branches.push(branch.to_owned());
        self.input.clear();
        self.rebuild_visible_indices(commits);
        true
    }

    pub fn remove_hidden_branch(&mut self, index: usize, commits: &[Commit]) -> bool {
        if index >= self.hidden_branches.len() {
            return false;
        }
        self.hidden_branches.remove(index);
        if let Some(selected) = self.state.selected() {
            if self.hidden_branches.is_empty() {
                self.state.select(None);
            } else if selected >= self.hidden_branches.len() {
                self.state.select(Some(self.hidden_branches.len().saturating_sub(1)));
            }
        }
        self.rebuild_visible_indices(commits);
        true
    }

    pub fn rebuild_visible_indices(&mut self, commits: &[Commit]) {
        if self.hidden_branches.is_empty() {
            self.visible_indices = (0..commits.len()).collect();
            return;
        }

        let mut visible_oids = HashSet::new();
        let mut queue = VecDeque::new();
        let commit_by_oid: HashMap<&str, &Commit> =
            commits.iter().map(|c| (c.oid.as_str(), c)).collect();

        for commit in commits {
            let has_unhidden_ref = commit.refs.iter().any(|r| {
                if r == "HEAD" {
                    return true;
                }
                !self.is_ref_hidden(r)
            });
            if has_unhidden_ref && visible_oids.insert(commit.oid.as_str()) {
                queue.push_back(commit);
            }
        }

        while let Some(commit) = queue.pop_front() {
            for parent_oid in &commit.parents {
                if visible_oids.insert(parent_oid.as_str())
                    && let Some(parent) = commit_by_oid.get(parent_oid.as_str())
                {
                    queue.push_back(parent);
                }
            }
        }

        self.visible_indices = commits
            .iter()
            .enumerate()
            .filter_map(|(index, commit)| {
                visible_oids.contains(commit.oid.as_str()).then_some(index)
            })
            .collect();
    }

    pub fn select(&mut self, index: usize) {
        if index < self.hidden_branches.len() {
            self.state.select(Some(index));
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.hidden_branches.len().checked_sub(1) else {
            self.state.select(None);
            return;
        };
        let current = self.state.selected().unwrap_or(0);
        self.state
            .select(Some(current.saturating_add_signed(delta).min(last)));
    }

    pub fn handle_key(&mut self, key: KeyEvent, commits: &[Commit]) -> Option<BranchFilterEffect> {
        match key.code {
            KeyCode::Esc => Some(BranchFilterEffect::Close),
            KeyCode::Enter => {
                let text = self.input.text().trim().to_owned();
                if !text.is_empty() && self.add_hidden_branch(&text, commits) {
                    Some(BranchFilterEffect::Changed)
                } else {
                    None
                }
            }
            KeyCode::Down => {
                self.move_selection(1);
                None
            }
            KeyCode::Up => {
                self.move_selection(-1);
                None
            }
            KeyCode::Delete | KeyCode::Backspace if self.input.is_empty() => {
                if let Some(selected) = self.state.selected()
                    && self.remove_hidden_branch(selected, commits)
                {
                    return Some(BranchFilterEffect::Changed);
                }
                None
            }
            _ => {
                self.input.handle_edit_key(key);
                None
            }
        }
    }
}

pub(crate) fn ref_matches_branch(reference: &str, branch: &str) -> bool {
    let branch = branch.trim();
    if branch.is_empty() {
        return false;
    }
    if reference.starts_with("tag: ") || reference == "HEAD" {
        return false;
    }
    let mut target = reference.strip_prefix("HEAD -> ").unwrap_or(reference);
    if let Some(rest) = target.strip_prefix("refs/heads/") {
        target = rest;
    } else if let Some(rest) = target.strip_prefix("refs/remotes/") {
        target = rest;
    }
    if target == branch {
        return true;
    }
    if let Some((_remote, rest)) = target.split_once('/')
        && rest == branch
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_commit(oid: &str, parents: Vec<&str>, refs: Vec<&str>) -> Commit {
        Commit {
            oid: oid.to_owned(),
            parents: parents.into_iter().map(str::to_owned).collect(),
            refs: refs.into_iter().map(str::to_owned).collect(),
            author: "Author".to_owned(),
            date: "today".to_owned(),
            subject: format!("Commit {oid}"),
            message: String::new(),
            graph: Vec::new(),
        }
    }

    #[test]
    fn test_ref_matches_branch() {
        assert!(ref_matches_branch("HEAD -> main", "main"));
        assert!(ref_matches_branch("main", "main"));
        assert!(ref_matches_branch("origin/main", "main"));
        assert!(ref_matches_branch("upstream/main", "main"));
        assert!(ref_matches_branch("refs/heads/main", "main"));
        assert!(ref_matches_branch("refs/remotes/origin/main", "main"));
        assert!(ref_matches_branch("feature/foo", "feature/foo"));
        assert!(ref_matches_branch("origin/feature/foo", "feature/foo"));
        assert!(ref_matches_branch("HEAD -> feature/foo", "feature/foo"));

        assert!(!ref_matches_branch("tag: v1.0", "v1.0"));
        assert!(!ref_matches_branch("HEAD", "HEAD"));
        assert!(!ref_matches_branch("feature/bar", "feature/foo"));
        assert!(!ref_matches_branch("main", "other"));
    }

    #[test]
    fn test_hiding_branch_filters_exclusive_commits() {
        // Commits:
        // 1 (root) -> 2 (main)
        // 2 -> 3 -> 4 (feature)
        let commits = vec![
            dummy_commit("4", vec!["3"], vec!["feature"]),
            dummy_commit("3", vec!["2"], vec![]),
            dummy_commit("2", vec!["1"], vec!["main"]),
            dummy_commit("1", vec![], vec![]),
        ];
        let mut filter = BranchFilter::default();
        filter.sync(Path::new("/test"), &commits);
        assert_eq!(filter.visible_indices(), &[0, 1, 2, 3]);

        assert!(filter.add_hidden_branch("feature", &commits));
        // Commits 4 and 3 should be hidden, leaving 2 and 1 (indices 2 and 3)
        assert_eq!(filter.visible_indices(), &[2, 3]);
        assert!(filter.is_ref_hidden("feature"));
        assert!(!filter.is_ref_hidden("main"));

        assert!(filter.remove_hidden_branch(0, &commits));
        assert_eq!(filter.visible_indices(), &[0, 1, 2, 3]);
    }

    #[test]
    fn test_merged_branch_retains_ancestors() {
        // Commits:
        // 1 (root) -> 2 (base)
        // 2 -> 3 (feature)
        // 2 -> 4 (main)
        // 4 + 3 -> 5 (merge on main)
        let commits = vec![
            dummy_commit("5", vec!["4", "3"], vec!["main"]),
            dummy_commit("4", vec!["2"], vec![]),
            dummy_commit("3", vec!["2"], vec!["feature"]),
            dummy_commit("2", vec!["1"], vec![]),
            dummy_commit("1", vec![], vec![]),
        ];
        let mut filter = BranchFilter::default();
        filter.sync(Path::new("/test"), &commits);

        assert!(filter.add_hidden_branch("feature", &commits));
        // Since feature (commit 3) is an ancestor of main (commit 5), all commits 1..5 remain visible
        assert_eq!(filter.visible_indices(), &[0, 1, 2, 3, 4]);
        // But the ref "feature" is marked hidden
        assert!(filter.is_ref_hidden("feature"));
    }
}
