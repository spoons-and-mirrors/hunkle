use std::path::{Path, PathBuf};

use crate::git::Commit;

use super::TextInput;

#[derive(Debug, Default)]
pub(crate) struct GraphSearch {
    root: Option<PathBuf>,
    searchable_commits: Vec<String>,
    visible_indices: Vec<usize>,
    match_positions: Vec<usize>,
    selected_match: Option<usize>,
    pub(crate) input: TextInput,
}

impl GraphSearch {
    pub(crate) fn sync(&mut self, root: &Path, commits: &[Commit], author_visible: &[usize]) {
        if self.root.as_deref() != Some(root) {
            self.root = Some(root.to_path_buf());
            self.input.clear();
        }
        self.searchable_commits = commits.iter().map(searchable_commit_text).collect();
        self.apply(author_visible);
    }

    pub(crate) fn apply(&mut self, author_visible: &[usize]) {
        self.visible_indices = author_visible.to_vec();
        let terms = self
            .input
            .text()
            .split_whitespace()
            .map(normalize_search_text)
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();
        self.match_positions = if terms.is_empty() {
            Vec::new()
        } else {
            author_visible
                .iter()
                .enumerate()
                .filter_map(|(position, index)| {
                    self.searchable_commits
                        .get(*index)
                        .is_some_and(|text| terms.iter().all(|term| text.contains(term.as_str())))
                        .then_some(position)
                })
                .collect()
        };
        self.selected_match = (!self.match_positions.is_empty()).then_some(0);
    }

    pub(crate) fn visible_indices(&self) -> &[usize] {
        &self.visible_indices
    }

    pub(crate) fn current_match_position(&self) -> Option<usize> {
        self.selected_match
            .and_then(|selected| self.match_positions.get(selected).copied())
    }

    pub(crate) fn match_status(&self) -> Option<(usize, usize)> {
        if self.input.is_empty() {
            return None;
        }
        Some((
            self.selected_match.map_or(0, |selected| selected + 1),
            self.match_positions.len(),
        ))
    }

    pub(crate) fn cycle_match(&mut self, forward: bool) -> Option<usize> {
        let count = self.match_positions.len();
        if count == 0 {
            return None;
        }
        let current = self.selected_match.unwrap_or(0);
        self.selected_match = Some(if forward {
            (current + 1) % count
        } else {
            current.checked_sub(1).unwrap_or(count - 1)
        });
        self.current_match_position()
    }
}

fn searchable_commit_text(commit: &Commit) -> String {
    normalize_search_text(&format!(
        "{} {} {} {} {}",
        commit.oid,
        commit.refs.join(" "),
        commit.subject,
        commit.message,
        commit.date
    ))
}

fn normalize_search_text(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(oid: &str, author: &str, date: &str, subject: &str, message: &str) -> Commit {
        Commit {
            oid: oid.to_owned(),
            parents: Vec::new(),
            refs: Vec::new(),
            author: author.to_owned(),
            date: date.to_owned(),
            subject: subject.to_owned(),
            message: message.to_owned(),
            graph: Vec::new(),
        }
    }

    #[test]
    fn finds_commit_content_and_date_without_filtering_the_graph() {
        let commits = vec![
            commit(
                "abc1234",
                "Ada Lovelace",
                "03Aug 14:20",
                "Polish graph search",
                "Searches the complete commit message",
            ),
            commit(
                "def5678",
                "Grace Hopper",
                "02Aug 09:10",
                "Improve navigation",
                "Keeps keyboard movement predictable",
            ),
        ];
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[0, 1]);

        for query in ["abc", "graph", "complete message", "03Aug"] {
            search.input.set(query);
            search.apply(&[0, 1]);
            assert_eq!(search.visible_indices(), &[0, 1], "query={query}");
            assert_eq!(search.current_match_position(), Some(0), "query={query}");
            assert_eq!(search.match_status(), Some((1, 1)), "query={query}");
        }

        search.input.set("Ada");
        search.apply(&[0, 1]);
        assert_eq!(search.visible_indices(), &[0, 1]);
        assert_eq!(search.current_match_position(), None);
        assert_eq!(search.match_status(), Some((0, 0)));
    }

    #[test]
    fn cycles_matches_within_the_author_filter() {
        let commits = vec![
            commit("abc", "Ada", "03Aug", "First match", ""),
            commit("def", "Grace", "03Aug", "Second match", ""),
            commit("ghi", "Linus", "02Aug", "No match", ""),
        ];
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[0, 1, 2]);
        search.input.set("03Aug");
        search.apply(&[0, 1, 2]);

        assert_eq!(search.visible_indices(), &[0, 1, 2]);
        assert_eq!(search.match_status(), Some((1, 2)));
        assert_eq!(search.cycle_match(true), Some(1));
        assert_eq!(search.match_status(), Some((2, 2)));
        assert_eq!(search.cycle_match(true), Some(0));
        assert_eq!(search.cycle_match(false), Some(1));

        search.apply(&[1, 2]);
        assert_eq!(search.visible_indices(), &[1, 2]);
        assert_eq!(search.current_match_position(), Some(0));
        assert_eq!(search.match_status(), Some((1, 1)));
    }
}
