use std::path::{Path, PathBuf};

use crate::git::Commit;

use super::TextInput;

#[derive(Debug, Default)]
pub(crate) struct GraphSearch {
    root: Option<PathBuf>,
    searchable_commits: Vec<String>,
    visible_indices: Vec<usize>,
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
        let terms = self
            .input
            .text()
            .split_whitespace()
            .map(normalize_search_text)
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();
        self.visible_indices = author_visible
            .iter()
            .copied()
            .filter(|index| {
                self.searchable_commits
                    .get(*index)
                    .is_some_and(|text| terms.iter().all(|term| text.contains(term.as_str())))
            })
            .collect();
    }

    pub(crate) fn visible_indices(&self) -> &[usize] {
        &self.visible_indices
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
    fn searches_commit_content_and_date_but_not_author() {
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
            assert_eq!(search.visible_indices(), &[0], "query={query}");
        }

        search.input.set("Ada");
        search.apply(&[0, 1]);
        assert!(search.visible_indices().is_empty());
    }

    #[test]
    fn intersects_search_results_with_the_author_filter() {
        let commits = vec![
            commit("abc", "Ada", "03Aug", "First match", ""),
            commit("def", "Grace", "03Aug", "Second match", ""),
        ];
        let mut search = GraphSearch::default();
        search.sync(Path::new("/repo"), &commits, &[1]);
        search.input.set("03Aug");
        search.apply(&[1]);

        assert_eq!(search.visible_indices(), &[1]);
    }
}
