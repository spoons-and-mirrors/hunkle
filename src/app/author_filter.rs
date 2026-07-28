use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;

use crate::git::Commit;

#[derive(Debug, Clone)]
pub(crate) struct AuthorEntry {
    pub name: String,
    pub commits: usize,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorFilterEffect {
    Close,
    Changed,
}

#[derive(Debug, Default)]
pub(crate) struct AuthorFilter {
    root: Option<PathBuf>,
    disabled: HashSet<String>,
    entries: Vec<AuthorEntry>,
    commit_authors: Vec<String>,
    visible_indices: Vec<usize>,
    pub state: ListState,
}

impl AuthorFilter {
    pub fn open(&mut self, root: &Path, commits: &[Commit]) {
        self.sync(root, commits);
        if self.state.selected().is_none() && !self.entries.is_empty() {
            self.state.select(Some(0));
        }
    }

    pub fn sync(&mut self, root: &Path, commits: &[Commit]) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_path_buf());
            self.disabled.clear();
            self.state = ListState::default();
        }
        let mut counts = BTreeMap::new();
        for commit in commits {
            *counts.entry(commit.author.clone()).or_insert(0usize) += 1;
        }
        self.disabled.retain(|author| counts.contains_key(author));
        self.entries = counts
            .into_iter()
            .map(|(name, commits)| AuthorEntry {
                enabled: !self.disabled.contains(&name),
                name,
                commits,
            })
            .collect();
        self.commit_authors = commits.iter().map(|commit| commit.author.clone()).collect();
        self.rebuild_visible_indices();
        if let Some(selected) = self.state.selected()
            && selected >= self.entries.len()
        {
            self.state.select(self.entries.len().checked_sub(1));
        }
    }

    pub fn entries(&self) -> &[AuthorEntry] {
        &self.entries
    }

    pub fn active_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.enabled).count()
    }

    pub fn visible_indices(&self) -> &[usize] {
        &self.visible_indices
    }

    fn rebuild_visible_indices(&mut self) {
        self.visible_indices = self
            .commit_authors
            .iter()
            .enumerate()
            .filter_map(|(index, author)| (!self.disabled.contains(author)).then_some(index))
            .collect()
    }

    pub fn matches(&self, commit: &Commit) -> bool {
        !self.disabled.contains(&commit.author)
    }

    pub fn ensure_enabled(&mut self, author: &str) {
        if self.disabled.remove(author)
            && let Some(entry) = self.entries.iter_mut().find(|entry| entry.name == author)
        {
            entry.enabled = true;
            self.rebuild_visible_indices();
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.entries.len() {
            self.state.select(Some(index));
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.entries.len().checked_sub(1) else {
            self.state.select(None);
            return;
        };
        let current = self.state.selected().unwrap_or(0);
        self.state
            .select(Some(current.saturating_add_signed(delta).min(last)));
    }

    pub fn toggle(&mut self, index: usize) -> bool {
        let Some(entry) = self.entries.get_mut(index) else {
            return false;
        };
        entry.enabled = !entry.enabled;
        if entry.enabled {
            self.disabled.remove(&entry.name);
        } else {
            self.disabled.insert(entry.name.clone());
        }
        self.rebuild_visible_indices();
        true
    }

    pub fn enable_all(&mut self) {
        self.disabled.clear();
        for entry in &mut self.entries {
            entry.enabled = true;
        }
        self.rebuild_visible_indices();
    }

    pub fn disable_all(&mut self) {
        self.disabled = self
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        for entry in &mut self.entries {
            entry.enabled = false;
        }
        self.rebuild_visible_indices();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AuthorFilterEffect> {
        match key.code {
            KeyCode::Esc => Some(AuthorFilterEffect::Close),
            KeyCode::Down => {
                self.move_selection(1);
                None
            }
            KeyCode::Up => {
                self.move_selection(-1);
                None
            }
            KeyCode::Home => {
                self.state.select((!self.entries.is_empty()).then_some(0));
                None
            }
            KeyCode::End => {
                self.state.select(self.entries.len().checked_sub(1));
                None
            }
            KeyCode::Enter | KeyCode::Char(' ') => self
                .state
                .selected()
                .and_then(|index| self.toggle(index).then_some(AuthorFilterEffect::Changed)),
            KeyCode::Char('a') => {
                self.enable_all();
                Some(AuthorFilterEffect::Changed)
            }
            KeyCode::Char('n') => {
                self.disable_all();
                Some(AuthorFilterEffect::Changed)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(author: &str) -> Commit {
        Commit {
            oid: author.to_owned(),
            parents: Vec::new(),
            refs: Vec::new(),
            author: author.to_owned(),
            date: String::new(),
            subject: String::new(),
            message: String::new(),
            graph: Vec::new(),
        }
    }

    #[test]
    fn filters_authors_and_resets_for_another_repository() {
        let commits = vec![commit("Ada"), commit("Lin"), commit("Ada")];
        let mut filter = AuthorFilter::default();
        filter.open(Path::new("/one"), &commits);
        assert_eq!(filter.entries()[0].commits, 2);
        assert!(filter.toggle(0));
        assert_eq!(filter.visible_indices(), &[1]);

        filter.open(Path::new("/two"), &commits);
        assert_eq!(filter.visible_indices(), &[0, 1, 2]);
    }
}
