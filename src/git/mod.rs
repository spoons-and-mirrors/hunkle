mod graph;
mod inventory;

pub(super) use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, UNIX_EPOCH},
};

pub(super) use anyhow::{Context, Result, anyhow, bail};

pub(super) use crate::{
    process::{self, Limits, Output},
    repo_path::RepoPath,
};

#[derive(Debug)]
struct WorktreeData {
    changes: Vec<Change>,
    fingerprint: u64,
    counts: (usize, usize),
    signature: WorktreeSignature,
}

#[derive(Debug)]
struct InventoryData {
    files: Vec<RepoPath>,
    directories: Vec<RepoPath>,
    fingerprint: u64,
    truncated: bool,
}

#[derive(Debug)]
struct HistoryData {
    branch: String,
    commits: Vec<Commit>,
}

#[derive(Debug)]
struct GraphData {
    commits: Vec<Commit>,
    width: usize,
    truncated: bool,
}

#[derive(Debug)]
struct RefsData {
    branches: Vec<Branch>,
    github_remote: bool,
}


const GIT_STDOUT_LIMIT: usize = 64 * 1024 * 1024;
const GIT_STDERR_LIMIT: usize = 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const SECTION_DIFF_TIMEOUT: Duration = Duration::from_secs(10);
const GIT_NETWORK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const COMMAND_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const DIFF_PREVIEW_LIMIT: usize = 2 * 1024 * 1024;
const UNTRACKED_FILE_LINE_LIMIT: u64 = 8 * 1024 * 1024;
const UNTRACKED_TOTAL_LINE_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryKind {
    Git,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedWorktree {
    pub(crate) path: PathBuf,
    pub(crate) head: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) is_main: bool,
    pub(crate) is_detached: bool,
    pub(crate) is_bare: bool,
    pub(crate) locked: bool,
    pub(crate) locked_reason: Option<String>,
    pub(crate) prunable: bool,
    pub(crate) prunable_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepositoryData {
    pub root: PathBuf,
    pub(crate) common_dir: Option<PathBuf>,
    pub kind: RepositoryKind,
    pub branch: String,
    pub changes: Vec<Change>,
    pub files: Vec<RepoPath>,
    pub directories: Vec<RepoPath>,
    pub history: Vec<Commit>,
    pub commits: Vec<Commit>,
    pub files_fingerprint: u64,
    pub inventory_truncated: bool,
    pub changes_fingerprint: u64,
    pub change_counts: (usize, usize),
    pub graph_width: usize,
    pub graph_truncated: bool,
    pub branches: Vec<Branch>,
    pub github_remote: bool,
    pub(crate) worktree_signature: Option<WorktreeSignature>,
    pub(crate) details_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorktreeSignature {
    state: u64,
    branch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshScope(u8);

impl RefreshScope {
    const INVENTORY: Self = Self(1 << 1);
    const HISTORY: Self = Self(1 << 2);
    const GRAPH: Self = Self(1 << 3);
    const REFS: Self = Self(1 << 4);

    pub const WORKTREE: Self = Self(1);
    pub const WORKTREE_AND_INVENTORY: Self = Self(Self::WORKTREE.0 | Self::INVENTORY.0);
    pub const HISTORY_AND_REFS: Self = Self(Self::HISTORY.0 | Self::GRAPH.0 | Self::REFS.0);
    pub const ALL: Self =
        Self(Self::WORKTREE.0 | Self::INVENTORY.0 | Self::HISTORY.0 | Self::GRAPH.0 | Self::REFS.0);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn includes(self, facet: Self) -> bool {
        self.0 & facet.0 != 0
    }
}

impl WorktreeSignature {
    pub(crate) fn refresh_scope_since(self, previous: Self) -> RefreshScope {
        if self.branch == previous.branch {
            RefreshScope::WORKTREE_AND_INVENTORY
        } else {
            RefreshScope::ALL
        }
    }

    #[cfg(test)]
    pub(crate) const fn for_test(state: u64, branch: u64) -> Self {
        Self { state, branch }
    }
}

#[derive(Debug)]
pub struct RepositoryUpdate {
    root: PathBuf,
    scope: RefreshScope,
    worktree: Option<WorktreeData>,
    inventory: Option<InventoryData>,
    history: Option<HistoryData>,
    graph: Option<GraphData>,
    refs: Option<RefsData>,
}






#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub upstream: String,
    pub oid: String,
    pub date: String,
    pub subject: String,
    pub remote: bool,
    pub current: bool,
    pub default: bool,
}

impl Branch {
    pub(crate) fn revision(&self) -> String {
        if self.remote {
            format!("refs/remotes/{}", self.name)
        } else {
            format!("refs/heads/{}", self.name)
        }
    }
}

pub(crate) fn branch_delete_protection(branch: &Branch) -> Option<String> {
    if branch.current {
        return Some("Cannot delete the checked-out branch".to_owned());
    }
    if matches!(branch.name.as_str(), "main" | "master" | "dev") {
        return Some(format!("Cannot delete protected branch {}", branch.name));
    }
    branch.default.then(|| {
        format!(
            "Cannot delete the repository's default branch {}",
            branch.name
        )
    })
}

impl RepositoryData {
    pub fn is_local(&self) -> bool {
        self.kind == RepositoryKind::Local
    }

    pub(crate) fn apply(&mut self, update: RepositoryUpdate) {
        debug_assert_eq!(self.root, update.root);
        self.details_ready |= update.scope == RefreshScope::ALL;
        if let Some(worktree) = update.worktree {
            self.changes = worktree.changes;
            self.changes_fingerprint = worktree.fingerprint;
            self.change_counts = worktree.counts;
            self.worktree_signature = Some(worktree.signature);
        }
        if let Some(inventory) = update.inventory {
            self.files = inventory.files;
            self.directories = inventory.directories;
            self.files_fingerprint = inventory.fingerprint;
            self.inventory_truncated = inventory.truncated;
        }
        if let Some(history) = update.history {
            self.branch = history.branch;
            self.history = history.commits;
        }
        if let Some(graph) = update.graph {
            self.commits = graph.commits;
            self.graph_width = graph.width;
            self.graph_truncated = graph.truncated;
        }
        if let Some(refs) = update.refs {
            self.branches = refs.branches;
            self.github_remote = refs.github_remote;
        }
    }
}

impl RepositoryUpdate {
    pub(crate) fn worktree_signature(&self) -> Option<WorktreeSignature> {
        self.worktree.as_ref().map(|worktree| worktree.signature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Change {
    pub path: RepoPath,
    pub original_path: Option<RepoPath>,
    pub code: char,
    pub staged: bool,
    pub additions: u64,
    pub deletions: u64,
}



#[derive(Debug, Clone)]
pub struct Commit {
    pub oid: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub message: String,
    pub graph: Vec<GraphCell>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub files: Vec<RepoPath>,
    pub files_truncated: bool,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphCell {
    pub symbol: char,
    pub color: usize,
}

#[derive(Debug)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub exit_code: Option<i32>,
}









fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn trim_line_ending(mut bytes: &[u8]) -> &[u8] {
    if bytes.ends_with(b"\n") {
        bytes = &bytes[..bytes.len() - 1];
    }
    if bytes.ends_with(b"\r") {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}










fn join_refresh_worker<T>(
    worker: Option<thread::ScopedJoinHandle<'_, Result<T>>>,
    label: &str,
) -> Result<Option<T>> {
    worker
        .map(|worker| {
            worker
                .join()
                .map_err(|_| anyhow!("{label} worker panicked"))?
        })
        .transpose()
}


































































mod branches;
pub(super) use branches::*;
mod diff;
pub(super) use diff::*;
mod discovery;
pub(super) use discovery::*;
mod log;
pub(super) use log::*;
mod mutations;
pub(super) use mutations::*;
mod subprocess;
pub(super) use subprocess::*;
mod status;
pub(super) use status::*;
mod worktrees;
pub(super) use worktrees::*;
#[cfg(test)]
mod tests;