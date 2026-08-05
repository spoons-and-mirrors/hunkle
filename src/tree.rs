use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    path::{Component, Path},
};

use crate::{
    filesystem::{WorkspaceEntry, read_workspace_directory},
    git::Change,
    repo_path::{RepoPath, display_os_str},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeRow {
    pub(crate) prefix: String,
    pub(crate) label: String,
    pub(crate) depth: usize,
    pub(crate) change_index: Option<usize>,
    pub(crate) directory_path: Option<RepoPath>,
    pub(crate) directory_expanded: Option<bool>,
    pub(crate) section: Option<WorktreeSection>,
    pub(crate) section_stats: Option<(u64, u64)>,
    pub(crate) descendant_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeSection {
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExplorerRow {
    pub(crate) prefix: String,
    pub(crate) label: String,
    pub(crate) depth: usize,
    pub(crate) file_path: Option<RepoPath>,
    pub(crate) directory_path: Option<RepoPath>,
    pub(crate) directory_expanded: Option<bool>,
    pub(crate) descendant_count: usize,
}

#[derive(Default)]
struct Node {
    children: BTreeMap<OsString, Node>,
    entries: Vec<usize>,
    descendant_count: usize,
    explicit_directory: bool,
}

pub(crate) struct FileTree {
    directories: HashMap<RepoPath, Vec<WorkspaceEntry>>,
}

pub(crate) struct PreparedFileTree {
    tree: FileTree,
}

impl PreparedFileTree {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            tree: FileTree::from_root(root),
        }
    }

    pub(crate) fn into_tree(self) -> FileTree {
        self.tree
    }
}

impl FileTree {
    pub(crate) fn from_root(root: &Path) -> Self {
        let mut tree = Self {
            directories: HashMap::new(),
        };
        if let Ok(entries) = read_workspace_directory(root, &RepoPath::default()) {
            let _ = tree.replace_directory(RepoPath::default(), entries);
        }
        tree
    }

    pub(crate) fn replace_directory(
        &mut self,
        directory: RepoPath,
        entries: Vec<WorkspaceEntry>,
    ) -> bool {
        if self.directories.get(&directory) == Some(&entries) {
            return false;
        }
        let child_directories: HashSet<_> = entries
            .iter()
            .filter(|entry| entry.is_directory)
            .map(|entry| entry.path.as_path())
            .collect();
        self.directories.retain(|path, _| {
            path == &directory
                || !path.as_path().starts_with(directory.as_path())
                || path
                    .as_path()
                    .ancestors()
                    .find(|ancestor| ancestor.parent() == Some(directory.as_path()))
                    .is_some_and(|child| child_directories.contains(child))
        });
        drop(child_directories);
        self.directories.insert(directory, entries);
        true
    }

    pub(crate) fn has_directory(&self, directory: &RepoPath) -> bool {
        self.directories.contains_key(directory)
    }

    pub(crate) fn loaded_directories(&self) -> Vec<RepoPath> {
        self.directories.keys().cloned().collect()
    }

    pub(crate) fn rows_expanded(&self, expanded: &HashSet<RepoPath>) -> Vec<ExplorerRow> {
        let mut rows = Vec::new();
        flatten_file_tree(self, &RepoPath::default(), &[], expanded, &mut rows);
        rows
    }
}

struct WorktreeSectionTree {
    section: WorktreeSection,
    root: Node,
    additions: u64,
    deletions: u64,
}

pub(crate) struct WorktreeTree {
    sections: Vec<WorktreeSectionTree>,
}

impl WorktreeTree {
    pub(crate) fn new(changes: &[Change]) -> Self {
        let mut sections = Vec::new();
        for section in [WorktreeSection::Staged, WorktreeSection::Unstaged] {
            let mut root = Node::default();
            let mut additions = 0_u64;
            let mut deletions = 0_u64;
            for (index, change) in changes.iter().enumerate() {
                let belongs = match section {
                    WorktreeSection::Staged => change.staged,
                    WorktreeSection::Unstaged => !change.staged,
                };
                if belongs {
                    insert_path(&mut root, &change.path, index);
                    additions = additions.saturating_add(change.additions);
                    deletions = deletions.saturating_add(change.deletions);
                }
            }
            if root.descendant_count > 0 {
                sections.push(WorktreeSectionTree {
                    section,
                    root,
                    additions,
                    deletions,
                });
            }
        }
        Self { sections }
    }

    pub(crate) fn rows(&self, collapsed: &HashSet<RepoPath>) -> Vec<WorktreeRow> {
        let mut rows = Vec::new();
        for tree in &self.sections {
            append_worktree_section(tree, collapsed, &mut rows);
        }
        rows
    }
}

fn append_worktree_section(
    tree: &WorktreeSectionTree,
    collapsed: &HashSet<RepoPath>,
    rows: &mut Vec<WorktreeRow>,
) {
    rows.push(WorktreeRow {
        prefix: String::new(),
        label: String::new(),
        depth: 0,
        change_index: None,
        directory_path: None,
        directory_expanded: None,
        section: Some(tree.section),
        section_stats: None,
        descendant_count: tree.root.descendant_count,
    });
    rows.push(WorktreeRow {
        prefix: String::new(),
        label: match tree.section {
            WorktreeSection::Staged => "STAGED".to_owned(),
            WorktreeSection::Unstaged => "UNSTAGED".to_owned(),
        },
        depth: 0,
        change_index: None,
        directory_path: None,
        directory_expanded: None,
        section: Some(tree.section),
        section_stats: Some((tree.additions, tree.deletions)),
        descendant_count: tree.root.descendant_count,
    });
    flatten_worktree(&tree.root, &RepoPath::default(), &[], true, collapsed, rows);
}

fn insert_path(root: &mut Node, path: &RepoPath, entry_index: usize) {
    let mut node = root;
    node.descendant_count += 1;
    for component in path.as_path().components() {
        let Component::Normal(component) = component else {
            continue;
        };
        node = node.children.entry(component.to_owned()).or_default();
        node.descendant_count += 1;
    }
    node.entries.push(entry_index);
}

fn sorted_children(node: &Node) -> Vec<(&OsString, &Node)> {
    let mut children: Vec<_> = node.children.iter().collect();
    children.sort_by(|(left_name, left), (right_name, right)| {
        (left.children.is_empty() && !left.explicit_directory)
            .cmp(&(right.children.is_empty() && !right.explicit_directory))
            .then_with(|| left_name.cmp(right_name))
    });
    children
}

fn flatten_file_tree(
    tree: &FileTree,
    directory: &RepoPath,
    lineage: &[bool],
    expanded: &HashSet<RepoPath>,
    rows: &mut Vec<ExplorerRow>,
) {
    let Some(children) = tree.directories.get(directory) else {
        return;
    };
    let child_count = children.len();
    for (position, child) in children.iter().enumerate() {
        let is_last = position + 1 == child_count;
        let prefix = tree_prefix(lineage, is_last, directory.is_empty() && position == 0);
        let label = child
            .path
            .file_name()
            .map(display_os_str)
            .unwrap_or_else(|| child.path.display());
        if !child.is_directory {
            rows.push(ExplorerRow {
                prefix,
                label,
                depth: lineage.len(),
                file_path: Some(child.path.clone()),
                directory_path: None,
                directory_expanded: None,
                descendant_count: 1,
            });
            continue;
        }
        let is_expanded = expanded.contains(&child.path);
        rows.push(ExplorerRow {
            prefix,
            label,
            depth: lineage.len(),
            file_path: None,
            directory_path: Some(child.path.clone()),
            directory_expanded: Some(is_expanded),
            descendant_count: tree.directories.get(&child.path).map_or(0, Vec::len),
        });
        if is_expanded {
            let mut child_lineage = lineage.to_vec();
            child_lineage.push(is_last);
            flatten_file_tree(tree, &child.path, &child_lineage, expanded, rows);
        }
    }
}

fn flatten_worktree(
    node: &Node,
    parent_path: &RepoPath,
    lineage: &[bool],
    top_level: bool,
    collapsed: &HashSet<RepoPath>,
    rows: &mut Vec<WorktreeRow>,
) {
    let children = sorted_children(node);
    let child_count = children.len();
    for (position, (name, child)) in children.into_iter().enumerate() {
        let is_last = position + 1 == child_count;
        let first_root = top_level && position == 0;
        let mut path = parent_path.join(name);
        let prefix = tree_prefix(lineage, is_last, first_root);

        if child.children.is_empty() {
            for (duplicate, change_index) in child.entries.iter().enumerate() {
                rows.push(WorktreeRow {
                    prefix: if duplicate == 0 {
                        prefix.clone()
                    } else {
                        tree_prefix(lineage, is_last, first_root)
                    },
                    label: display_os_str(name),
                    depth: lineage.len(),
                    change_index: Some(*change_index),
                    directory_path: None,
                    directory_expanded: None,
                    section: None,
                    section_stats: None,
                    descendant_count: 1,
                });
            }
        } else {
            for change_index in &child.entries {
                rows.push(WorktreeRow {
                    prefix: prefix.clone(),
                    label: display_os_str(name),
                    depth: lineage.len(),
                    change_index: Some(*change_index),
                    directory_path: None,
                    directory_expanded: None,
                    section: None,
                    section_stats: None,
                    descendant_count: 1,
                });
            }
            let mut label = display_os_str(name);
            let mut directory = child;
            while directory.entries.is_empty() && directory.children.len() == 1 {
                let (next_name, next) = directory.children.first_key_value().expect("one child");
                if next.children.is_empty() || !next.entries.is_empty() {
                    break;
                }
                label.push('/');
                label.push_str(&display_os_str(next_name));
                path = path.join(next_name);
                directory = next;
            }
            let expanded = !collapsed.contains(&path);
            rows.push(WorktreeRow {
                prefix,
                label,
                depth: lineage.len(),
                change_index: None,
                directory_path: Some(path.clone()),
                directory_expanded: Some(expanded),
                section: None,
                section_stats: None,
                descendant_count: directory
                    .descendant_count
                    .saturating_sub(directory.entries.len()),
            });
            if expanded {
                let mut child_lineage = lineage.to_vec();
                child_lineage.push(is_last);
                flatten_worktree(directory, &path, &child_lineage, false, collapsed, rows);
            }
        }
    }
}

#[cfg(test)]
fn build_worktree(changes: &[Change], collapsed: &HashSet<RepoPath>) -> Vec<WorktreeRow> {
    WorktreeTree::new(changes).rows(collapsed)
}

fn tree_prefix(lineage: &[bool], _is_last: bool, _first_root: bool) -> String {
    let mut prefix = String::from(" ");
    for _ in lineage {
        prefix.push_str("│ ");
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_hierarchical_worktree_without_repeating_paths() {
        let changes = [
            change("cli/crates/sleev-tui/src/app.rs"),
            change("cli/crates/sleev-tui/src/views/home.rs"),
            change("cli/crates/sleev-tui/tests/app.rs"),
        ];

        let rows = build_worktree(&changes, &HashSet::new());
        let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "",
                "UNSTAGED",
                "cli/crates/sleev-tui",
                "src",
                "views",
                "home.rs",
                "app.rs",
                "tests",
                "app.rs"
            ]
        );
        assert_eq!(rows[1].section, Some(WorktreeSection::Unstaged));
        assert_eq!(rows[2].prefix, " ");
        assert_eq!(rows[3].prefix, " │ ");
        assert_eq!(rows[4].label, "views");
        assert_eq!(rows[6].change_index, Some(0));
        assert_eq!(rows[5].change_index, Some(1));
        assert_eq!(rows[8].change_index, Some(2));

        let collapsed = HashSet::from([RepoPath::from("cli/crates/sleev-tui")]);
        let rows = build_worktree(&changes, &collapsed);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].directory_expanded, Some(false));
    }

    #[test]
    fn places_staged_changes_before_unstaged_changes() {
        let mut staged = change("src/app.rs");
        staged.staged = true;
        staged.additions = 4;
        staged.deletions = 1;
        let mut unstaged = change("src/app.rs");
        unstaged.additions = 2;
        unstaged.deletions = 1;
        let rows = build_worktree(&[unstaged, staged], &HashSet::new());

        assert_eq!(rows[1].section, Some(WorktreeSection::Staged));
        assert_eq!(rows[1].label, "STAGED");
        assert_eq!(rows[1].section_stats, Some((4, 1)));
        assert_eq!(rows[3].change_index, Some(1));
        assert_eq!(rows[5].section, Some(WorktreeSection::Unstaged));
        assert_eq!(rows[5].label, "UNSTAGED");
        assert_eq!(rows[5].section_stats, Some((2, 1)));
        assert_eq!(rows[7].change_index, Some(0));
    }

    #[test]
    fn keeps_a_deleted_file_alongside_an_untracked_directory_at_the_same_path() {
        let changes = [
            change("foo"),
            change("foo/bar.txt"),
            change("dir/nested"),
            change("dir/nested/child.txt"),
        ];

        let rows = build_worktree(&changes, &HashSet::new());

        assert!(rows.iter().any(|row| row.change_index == Some(0)));
        assert!(rows.iter().any(|row| row.change_index == Some(1)));
        assert!(rows.iter().any(|row| row.change_index == Some(2)));
        assert!(rows.iter().any(|row| row.change_index == Some(3)));
        let directory = rows
            .iter()
            .find(|row| {
                row.directory_path
                    .as_ref()
                    .is_some_and(|path| path == "foo")
            })
            .unwrap();
        assert_eq!(directory.descendant_count, 1);
    }

    #[test]
    fn builds_a_lazy_repository_file_tree() {
        let mut tree = FileTree {
            directories: HashMap::new(),
        };
        assert!(tree.replace_directory(
            RepoPath::default(),
            vec![
                entry("src", true),
                entry("empty", true),
                entry("README.md", false),
            ],
        ));
        assert!(!tree.replace_directory(
            RepoPath::default(),
            vec![
                entry("src", true),
                entry("empty", true),
                entry("README.md", false),
            ],
        ));
        let rows = tree.rows_expanded(&HashSet::new());
        let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["src", "empty", "README.md"]);
        assert!(
            rows[2]
                .file_path
                .as_ref()
                .is_some_and(|path| path == "README.md")
        );

        let expanded = HashSet::from([RepoPath::from("src")]);
        assert_eq!(tree.rows_expanded(&expanded).len(), 3);
        let _ = tree.replace_directory(
            "src".into(),
            vec![entry("src/app", true), entry("src/main.rs", false)],
        );
        let rows = tree.rows_expanded(&expanded);
        let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["src", "app", "main.rs", "empty", "README.md"]);
        assert!(
            rows[2]
                .file_path
                .as_ref()
                .is_some_and(|path| path == "src/main.rs")
        );
        assert_eq!(rows[1].directory_expanded, Some(false));

        let _ = tree.replace_directory("src/app".into(), vec![entry("src/app/lib.rs", false)]);
        assert!(tree.has_directory(&"src/app".into()));
        let _ = tree.replace_directory("src".into(), vec![entry("src/main.rs", false)]);
        assert!(!tree.has_directory(&"src/app".into()));
    }

    fn entry(path: &str, is_directory: bool) -> WorkspaceEntry {
        WorkspaceEntry {
            path: path.into(),
            is_directory,
        }
    }

    fn change(path: &str) -> Change {
        Change {
            path: path.into(),
            original_path: None,
            code: 'M',
            staged: false,
            additions: 0,
            deletions: 0,
        }
    }
}
