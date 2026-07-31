mod graph;
mod inventory;

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    process::{self, Limits, Output},
    repo_path::RepoPath,
};

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

pub(crate) fn discard_unstaged(root: &Path, change: &Change) -> Result<()> {
    if change.staged {
        bail!("Cannot discard a staged change");
    }
    for path in std::iter::once(&change.path).chain(change.original_path.iter()) {
        let unmerged = run_path_command(root, &["ls-files", "--unmerged", "--"], &[path])?;
        if !unmerged.status.success() {
            bail!("{}", clean_stderr(&unmerged));
        }
        if !unmerged.stdout.is_empty() {
            bail!("Cannot discard unresolved changes to {}", change.path);
        }
    }

    match change.code {
        '?' | 'C' => clean_untracked_path(root, &change.path),
        'R' => {
            let original_path = change
                .original_path
                .as_ref()
                .ok_or_else(|| anyhow!("Cannot restore rename without its original path"))?;
            run_path_command_ok(root, &["restore", "--worktree", "--"], &[original_path])?;
            clean_untracked_path(root, &change.path)
        }
        _ => run_path_command_ok(root, &["restore", "--worktree", "--"], &[&change.path]),
    }
}

fn clean_untracked_path(root: &Path, path: &RepoPath) -> Result<()> {
    run_path_command_ok(root, &["clean", "-f", "--"], &[path])?;
    if fs::symlink_metadata(root.join(path)).is_ok() {
        bail!("Git did not remove untracked path {path}");
    }
    Ok(())
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

#[cfg(test)]
pub fn discover(path: &Path) -> Result<PathBuf> {
    match discover_workspace(path)? {
        WorkspaceDiscovery::Repository(root) => Ok(root),
        WorkspaceDiscovery::Local(reason) => bail!("{reason}"),
    }
}

enum WorkspaceDiscovery {
    Repository(PathBuf),
    Local(String),
}

fn discover_workspace(path: &Path) -> Result<WorkspaceDiscovery> {
    let output = process::run(
        Command::new("git")
            .args(["-C"])
            .arg(path)
            .args(["rev-parse", "--show-toplevel"])
            .env("LC_ALL", "C"),
        git_limits(),
    )
    .with_context(|| "could not start git; make sure it is installed")?;

    ensure_complete(&output, "git rev-parse")?;
    if !output.status.success() {
        let error = clean_stderr(&output);
        if error.contains("not a git repository") {
            return Ok(WorkspaceDiscovery::Local(error));
        }
        bail!("{error}");
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        bail!("Git did not return a worktree path");
    }

    let root = fs::canonicalize(root).context("could not resolve repository root")?;
    let requested = fs::canonicalize(path).context("could not resolve requested directory")?;
    if root != requested {
        return Ok(WorkspaceDiscovery::Local(format!(
            "{} is not a repository root (enclosing repository: {})",
            requested.display(),
            root.display()
        )));
    }
    Ok(WorkspaceDiscovery::Repository(root))
}

pub(crate) fn common_git_dir(root: &Path) -> Result<PathBuf> {
    let output = run(root, &["rev-parse", "--git-common-dir"])?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }

    let path = path_from_git_bytes(trim_line_ending(&output.stdout));
    if path.as_os_str().is_empty() {
        bail!("Git returned an empty common repository directory");
    }
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    fs::canonicalize(&path).with_context(|| {
        format!(
            "could not resolve common repository directory {}",
            path.display()
        )
    })
}

pub(crate) fn list_worktrees(repository: &Path) -> Result<Vec<LinkedWorktree>> {
    let output = run(repository, &["worktree", "list", "--porcelain", "-z"])?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    parse_worktrees(&output.stdout)
}

pub(crate) fn remove_worktree(repository: &Path, worktree: &Path) -> Result<()> {
    let output = process::run(
        base_command(repository)
            .args(["worktree", "remove", "--"])
            .arg(worktree),
        git_limits(),
    )
    .context("could not run git worktree remove")?;
    ensure_complete(&output, "git worktree remove")?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

fn parse_worktrees(bytes: &[u8]) -> Result<Vec<LinkedWorktree>> {
    let mut worktrees = Vec::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let end = remaining
            .windows(2)
            .position(|bytes| bytes == b"\0\0")
            .context("malformed worktree list: record is not terminated")?;
        let record = &remaining[..end];
        remaining = &remaining[end + 2..];
        if record.is_empty() {
            bail!("malformed worktree list: empty record");
        }
        worktrees.push(parse_worktree_record(record, worktrees.is_empty())?);
    }
    Ok(worktrees)
}

fn parse_worktree_record(record: &[u8], is_main: bool) -> Result<LinkedWorktree> {
    let mut fields = record.split(|byte| *byte == 0);
    let path = fields
        .next()
        .and_then(|field| field.strip_prefix(b"worktree "))
        .filter(|path| !path.is_empty())
        .context("malformed worktree record: missing worktree path")?;
    let mut worktree = LinkedWorktree {
        path: path_from_git_bytes(path),
        head: None,
        branch: None,
        is_main,
        is_detached: false,
        is_bare: false,
        locked: false,
        locked_reason: None,
        prunable: false,
        prunable_reason: None,
    };

    for field in fields {
        if let Some(head) = field.strip_prefix(b"HEAD ") {
            if head.is_empty() || worktree.head.replace(text(head)).is_some() {
                bail!("malformed worktree record: invalid HEAD field");
            }
        } else if let Some(branch) = field.strip_prefix(b"branch ") {
            if branch.is_empty() || worktree.branch.replace(text(branch)).is_some() {
                bail!("malformed worktree record: invalid branch field");
            }
        } else if field == b"detached" {
            if worktree.is_detached {
                bail!("malformed worktree record: duplicate detached field");
            }
            worktree.is_detached = true;
        } else if field == b"bare" {
            if worktree.is_bare {
                bail!("malformed worktree record: duplicate bare field");
            }
            worktree.is_bare = true;
        } else if field == b"locked" {
            if worktree.locked {
                bail!("malformed worktree record: duplicate locked field");
            }
            worktree.locked = true;
        } else if let Some(reason) = field.strip_prefix(b"locked ") {
            if worktree.locked {
                bail!("malformed worktree record: duplicate locked field");
            }
            worktree.locked = true;
            worktree.locked_reason = Some(text(reason));
        } else if field == b"prunable" {
            if worktree.prunable {
                bail!("malformed worktree record: duplicate prunable field");
            }
            worktree.prunable = true;
        } else if let Some(reason) = field.strip_prefix(b"prunable ") {
            if worktree.prunable {
                bail!("malformed worktree record: duplicate prunable field");
            }
            worktree.prunable = true;
            worktree.prunable_reason = Some(text(reason));
        }
    }

    if worktree.is_bare {
        if worktree.head.is_some() || worktree.branch.is_some() || worktree.is_detached {
            bail!("malformed worktree record: bare worktree has checkout state");
        }
    } else if worktree.head.is_none() || worktree.branch.is_some() == worktree.is_detached {
        bail!("malformed worktree record: incomplete checkout state");
    }
    Ok(worktree)
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

pub(crate) fn delete_branch(
    root: &Path,
    branch: &str,
    remote: Option<(&str, &str)>,
    force: bool,
) -> Result<()> {
    if let Some(branch) = repository_branches(root)?
        .iter()
        .find(|candidate| !candidate.remote && candidate.name == branch)
        && let Some(reason) = branch_delete_protection(branch)
    {
        bail!(reason);
    }
    let mut args = vec!["branch", "--delete"];
    if force {
        args.push("--force");
    }
    args.extend(["--", branch]);
    run_ok(root, &args)?;
    if let Some((remote, remote_branch)) = remote {
        let refspec = format!(":refs/heads/{remote_branch}");
        let output = process::run(
            base_command(root)
                .arg("push")
                .arg("--")
                .arg(remote)
                .arg(&refspec),
            Limits::new(COMMAND_OUTPUT_LIMIT, GIT_STDERR_LIMIT, GIT_NETWORK_TIMEOUT),
        )
        .with_context(|| format!("could not delete {remote}/{remote_branch}"))?;
        if output.timed_out {
            bail!("Timed out deleting {remote}/{remote_branch}");
        }
        if !output.status.success() {
            bail!(
                "Deleted local branch {branch}, but could not delete {remote}/{remote_branch}: {}",
                clean_stderr(&output)
            );
        }
    }
    Ok(())
}

pub(crate) fn checkout_branch(root: &Path, branch: &str, remote: bool) -> Result<CommandOutput> {
    let output = if remote {
        run(root, &["switch", "--track", "--", branch])?
    } else {
        run(root, &["switch", "--no-guess", "--", branch])?
    };
    Ok(command_output(output))
}

#[cfg(test)]
pub fn load(path: &Path) -> Result<RepositoryData> {
    load_git_root(discover(path)?)
}

pub fn load_or_local(path: &Path) -> Result<RepositoryData> {
    match discover_workspace(path)? {
        WorkspaceDiscovery::Repository(root) => load_git_root(root),
        WorkspaceDiscovery::Local(reason) => {
            drop(reason);
            local_workspace(path)
        }
    }
}

pub(crate) fn bootstrap_or_local(path: &Path) -> Result<RepositoryData> {
    match discover_workspace(path)? {
        WorkspaceDiscovery::Repository(root) => {
            let common_dir = common_git_dir(&root)?;
            Ok(bootstrap_data(
                root,
                Some(common_dir),
                RepositoryKind::Git,
                String::new(),
            ))
        }
        WorkspaceDiscovery::Local(reason) => {
            drop(reason);
            let root = canonical_workspace_root(path)?;
            Ok(bootstrap_data(
                root,
                None,
                RepositoryKind::Local,
                "local".to_owned(),
            ))
        }
    }
}

fn bootstrap_data(
    root: PathBuf,
    common_dir: Option<PathBuf>,
    kind: RepositoryKind,
    branch: String,
) -> RepositoryData {
    let changes = Vec::<Change>::new();
    let files = Vec::<RepoPath>::new();
    let directories = Vec::<RepoPath>::new();
    RepositoryData {
        root,
        common_dir,
        kind,
        branch,
        changes_fingerprint: fingerprint(&changes),
        files_fingerprint: fingerprint(&(&files, &directories)),
        changes,
        files,
        directories,
        history: Vec::new(),
        commits: Vec::new(),
        inventory_truncated: false,
        change_counts: (0, 0),
        graph_width: 0,
        graph_truncated: false,
        branches: Vec::new(),
        github_remote: false,
        worktree_signature: None,
        details_ready: false,
    }
}

fn load_git_root(root: PathBuf) -> Result<RepositoryData> {
    let (common_dir, worktree, inventory, history, graph, refs) = thread::scope(|scope| {
        let common_dir = scope.spawn(|| common_git_dir(&root));
        let worktree = scope.spawn(|| load_worktree(&root));
        let inventory = scope.spawn(|| load_git_inventory(&root));
        let history = scope.spawn(|| load_history(&root));
        let graph = scope.spawn(|| load_graph(&root));
        let refs = scope.spawn(|| load_refs(&root));

        Ok::<_, anyhow::Error>((
            common_dir
                .join()
                .map_err(|_| anyhow!("repository identity worker panicked"))??,
            worktree
                .join()
                .map_err(|_| anyhow!("status worker panicked"))??,
            inventory
                .join()
                .map_err(|_| anyhow!("file worker panicked"))??,
            history
                .join()
                .map_err(|_| anyhow!("history worker panicked"))??,
            graph
                .join()
                .map_err(|_| anyhow!("graph worker panicked"))??,
            refs.join().map_err(|_| anyhow!("refs worker panicked"))??,
        ))
    })?;

    Ok(RepositoryData {
        root,
        common_dir: Some(common_dir),
        kind: RepositoryKind::Git,
        branch: history.branch,
        changes: worktree.changes,
        files: inventory.files,
        directories: inventory.directories,
        history: history.commits,
        commits: graph.commits,
        files_fingerprint: inventory.fingerprint,
        inventory_truncated: inventory.truncated,
        changes_fingerprint: worktree.fingerprint,
        change_counts: worktree.counts,
        graph_width: graph.width,
        graph_truncated: graph.truncated,
        branches: refs.branches,
        github_remote: refs.github_remote,
        worktree_signature: Some(worktree.signature),
        details_ready: true,
    })
}

pub fn refresh_repository(
    root: &Path,
    kind: RepositoryKind,
    scope: RefreshScope,
) -> Result<RepositoryUpdate> {
    if kind == RepositoryKind::Local {
        return Ok(RepositoryUpdate {
            root: root.to_owned(),
            scope,
            worktree: None,
            inventory: scope
                .includes(RefreshScope::INVENTORY)
                .then(|| load_local_inventory(root))
                .transpose()?,
            history: None,
            graph: None,
            refs: None,
        });
    }

    let (worktree, inventory, history, graph, refs) = thread::scope(|thread_scope| {
        let worktree = scope
            .includes(RefreshScope::WORKTREE)
            .then(|| thread_scope.spawn(|| load_worktree(root)));
        let inventory = scope
            .includes(RefreshScope::INVENTORY)
            .then(|| thread_scope.spawn(|| load_git_inventory(root)));
        let history = scope
            .includes(RefreshScope::HISTORY)
            .then(|| thread_scope.spawn(|| load_history(root)));
        let graph = scope
            .includes(RefreshScope::GRAPH)
            .then(|| thread_scope.spawn(|| load_graph(root)));
        let refs = scope
            .includes(RefreshScope::REFS)
            .then(|| thread_scope.spawn(|| load_refs(root)));

        Ok::<_, anyhow::Error>((
            join_refresh_worker(worktree, "status")?,
            join_refresh_worker(inventory, "file")?,
            join_refresh_worker(history, "history")?,
            join_refresh_worker(graph, "graph")?,
            join_refresh_worker(refs, "refs")?,
        ))
    })?;

    Ok(RepositoryUpdate {
        root: root.to_owned(),
        scope,
        worktree,
        inventory,
        history,
        graph,
        refs,
    })
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

fn load_worktree(root: &Path) -> Result<WorktreeData> {
    let (mut changes, signature) = status(root)?;
    populate_diff_stats(root, &mut changes)?;
    Ok(WorktreeData {
        fingerprint: fingerprint(&changes),
        counts: change_counts(&changes),
        signature,
        changes,
    })
}

fn load_git_inventory(root: &Path) -> Result<InventoryData> {
    let (files, directories, truncated) = inventory::git_entries(root)?;
    Ok(InventoryData {
        fingerprint: fingerprint(&(&files, &directories)),
        files,
        directories,
        truncated,
    })
}

fn load_local_inventory(root: &Path) -> Result<InventoryData> {
    let (files, directories, truncated) = inventory::local_entries(root)?;
    Ok(InventoryData {
        fingerprint: fingerprint(&(&files, &directories)),
        files,
        directories,
        truncated,
    })
}

fn load_history(root: &Path) -> Result<HistoryData> {
    let (branch, commits) = thread::scope(|scope| {
        let branch = scope.spawn(|| branch_name(root));
        let commits = scope.spawn(|| branch_history(root));
        Ok::<_, anyhow::Error>((
            branch
                .join()
                .map_err(|_| anyhow!("branch worker panicked"))??,
            commits
                .join()
                .map_err(|_| anyhow!("history worker panicked"))??,
        ))
    })?;
    Ok(HistoryData { branch, commits })
}

fn load_graph(root: &Path) -> Result<GraphData> {
    let prepared = graph::prepare(log(root)?);
    Ok(GraphData {
        commits: prepared.commits,
        width: prepared.width,
        truncated: prepared.truncated,
    })
}

fn load_refs(root: &Path) -> Result<RefsData> {
    let (branches, github_remote) = thread::scope(|scope| {
        let branches = scope.spawn(|| repository_branches(root));
        let github_remote = scope.spawn(|| repository_has_github_remote(root));
        Ok::<_, anyhow::Error>((
            branches
                .join()
                .map_err(|_| anyhow!("branch list worker panicked"))??,
            github_remote
                .join()
                .map_err(|_| anyhow!("remote worker panicked"))??,
        ))
    })?;
    Ok(RefsData {
        branches,
        github_remote,
    })
}

fn local_workspace(path: &Path) -> Result<RepositoryData> {
    let root = canonical_workspace_root(path)?;
    let inventory = load_local_inventory(&root)?;
    Ok(RepositoryData {
        root,
        common_dir: None,
        kind: RepositoryKind::Local,
        branch: "local".to_owned(),
        changes: Vec::new(),
        files: inventory.files,
        directories: inventory.directories,
        history: Vec::new(),
        commits: Vec::new(),
        files_fingerprint: inventory.fingerprint,
        inventory_truncated: inventory.truncated,
        changes_fingerprint: fingerprint(&Vec::<Change>::new()),
        change_counts: (0, 0),
        graph_width: 0,
        graph_truncated: false,
        branches: Vec::new(),
        github_remote: false,
        worktree_signature: None,
        details_ready: true,
    })
}

fn canonical_workspace_root(path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(path).context("could not resolve workspace directory")?;
    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }
    Ok(root)
}

fn repository_has_github_remote(root: &Path) -> Result<bool> {
    let output = run(root, &["remote", "-v"])?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .any(is_github_remote_url))
}

fn is_github_remote_url(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.starts_with("git@github.com:")
        || value.starts_with("https://github.com/")
        || value.starts_with("http://github.com/")
        || value.starts_with("ssh://git@github.com/")
        || value.starts_with("git://github.com/")
}

fn repository_branches(root: &Path) -> Result<Vec<Branch>> {
    let output = run(
        root,
        &[
            "for-each-ref",
            "--format=%(HEAD)%00%(refname)%00%(refname:short)%00%(objectname:short)%00%(upstream:short)%00%(committerdate:relative)%00%(subject)%00%(symref:short)%00",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut default_branch = None;
    let mut default_from_origin = false;
    let fields = output.stdout.split(|byte| *byte == 0);
    let mut branches = fields
        .collect::<Vec<_>>()
        .chunks_exact(8)
        .filter_map(|fields| {
            let text = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
            let refname = text(trim_ascii(fields[1]));
            let name = text(fields[2]);
            if let Some(remote) = refname
                .strip_prefix("refs/remotes/")
                .and_then(|name| name.strip_suffix("/HEAD"))
            {
                let symref = text(fields[7]);
                if let Some(target) = symref.strip_prefix(&format!("{remote}/")) {
                    let from_origin = remote == "origin";
                    if default_branch.is_none() || (from_origin && !default_from_origin) {
                        default_branch = Some(target.to_owned());
                        default_from_origin = from_origin;
                    }
                }
                return None;
            }
            Some(Branch {
                name,
                upstream: text(fields[4]),
                oid: text(fields[3]),
                date: text(fields[5]),
                subject: text(fields[6]),
                remote: refname.starts_with("refs/remotes/"),
                current: trim_ascii(fields[0]) == b"*",
                default: false,
            })
        })
        .collect::<Vec<_>>();
    if let Some(default_branch) = default_branch {
        for branch in &mut branches {
            branch.default = !branch.remote && branch.name == default_branch;
        }
    }
    branches.sort_by(|left, right| {
        right
            .current
            .cmp(&left.current)
            .then_with(|| left.remote.cmp(&right.remote))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(branches)
}

fn fingerprint<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn change_counts(changes: &[Change]) -> (usize, usize) {
    changes.iter().fold((0, 0), |(staged, unstaged), change| {
        if change.staged {
            (staged + 1, unstaged)
        } else {
            (staged, unstaged + 1)
        }
    })
}

pub fn file_content(root: &Path, relative_path: &RepoPath) -> Result<String> {
    const MAX_PREVIEW_BYTES: u64 = 1_048_576;

    let path = root.join(relative_path);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(&path)
            .with_context(|| format!("could not read link {}", path.display()))?;
        return Ok(format!("Symbolic link -> {}", target.display()));
    }
    if metadata.is_dir() {
        return Ok("Directory\n\nThis path may be a Git submodule.".to_owned());
    }
    if !metadata.is_file() {
        return Ok("Preview unavailable for this special file type.".to_owned());
    }
    let (file, metadata) = open_regular_file(&path)
        .with_context(|| format!("could not safely read {}", path.display()))?;
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Ok(format!(
            "File is too large to preview\n\n{} bytes",
            metadata.len()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_PREVIEW_BYTES + 1) as usize);
    file.take(MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read {}", path.display()))?;
    if bytes.len() > MAX_PREVIEW_BYTES as usize {
        return Ok(format!(
            "File is too large to preview\n\nMore than {MAX_PREVIEW_BYTES} bytes"
        ));
    }
    if bytes.contains(&0) {
        return Ok(format!("Binary file\n\n{} bytes", bytes.len()));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn stage(root: &Path, change: &Change) -> Result<()> {
    let mut paths = Vec::new();
    if let Some(original) = &change.original_path {
        paths.push(original);
    }
    paths.push(&change.path);
    run_path_command_ok(root, &["add", "--"], &paths)
}

pub fn unstage(root: &Path, change: &Change) -> Result<()> {
    let mut paths = Vec::new();
    if let Some(original) = &change.original_path {
        paths.push(original);
    }
    paths.push(&change.path);
    let output = run_path_command(root, &["restore", "--staged", "--"], &paths)?;
    if output.status.success() {
        return Ok(());
    }

    // `restore --staged` cannot address an unborn HEAD, while reset can.
    run_path_command_ok(root, &["reset", "--"], &paths)
}

pub fn stage_all(root: &Path) -> Result<()> {
    run_ok(root, &["add", "-A"])
}

pub fn unstage_all(root: &Path) -> Result<()> {
    let output = run(root, &["restore", "--staged", "."])?;
    if output.status.success() {
        return Ok(());
    }
    run_ok(root, &["reset"])
}

pub fn stage_hunk(root: &Path, diff: &str, index: usize) -> Result<()> {
    let patch = hunk_patch(diff, index).context("diff hunk is no longer available")?;
    let output = process::run_with_input(
        base_command(root).args(["apply", "--cached", "-"]),
        patch.into_bytes(),
        git_limits(),
    )
    .context("could not finish git apply --cached")?;
    ensure_complete(&output, "git apply --cached")?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

fn hunk_patch(diff: &str, target: usize) -> Option<String> {
    let lines: Vec<&str> = diff.lines().collect();
    let first_hunk = lines.iter().position(|line| line.starts_with("@@"))?;
    let start = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("@@"))
        .nth(target)?
        .0;
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.starts_with("@@") || line.starts_with("diff --git"))
        .map_or(lines.len(), |offset| start + 1 + offset);

    let mut patch = lines[..first_hunk].join("\n");
    patch.push('\n');
    patch.push_str(&lines[start..end].join("\n"));
    patch.push('\n');
    Some(patch)
}

pub fn commit(root: &Path, message: &str) -> Result<CommandOutput> {
    if message.trim().is_empty() {
        bail!("Commit message cannot be empty");
    }
    let output = run(root, &["commit", "-m", message.trim()])?;
    Ok(command_output(output))
}

pub(crate) fn commit_draft_path(root: &Path) -> Result<PathBuf> {
    let output = run(root, &["rev-parse", "--absolute-git-dir"])?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let git_directory = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if git_directory.is_empty() {
        bail!("Git returned an empty repository directory");
    }
    Ok(PathBuf::from(git_directory).join("HUNKLE_COMMIT_DRAFT"))
}

pub fn fetch(root: &Path) -> Result<CommandOutput> {
    let output = run_limited(
        root,
        &["fetch", "--all", "--prune"],
        Limits::new(COMMAND_OUTPUT_LIMIT, GIT_STDERR_LIMIT, GIT_NETWORK_TIMEOUT),
    )?;
    Ok(command_output(output))
}

pub fn run_command(root: &Path, args: &[String]) -> Result<CommandOutput> {
    let output = process::run(
        base_command(root).args(args),
        Limits::new(COMMAND_OUTPUT_LIMIT, GIT_STDERR_LIMIT, GIT_NETWORK_TIMEOUT),
    )
    .with_context(|| format!("could not run git {}", args.join(" ")))?;
    Ok(command_output(output))
}

pub(crate) fn worktree_signature(root: &Path) -> Result<WorktreeSignature> {
    let output = run(
        root,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }

    let changes = parse_status(&output.stdout)?;
    Ok(status_signature(root, &output.stdout, &changes))
}

fn status_signature(root: &Path, output: &[u8], changes: &[Change]) -> WorktreeSignature {
    let mut state = DefaultHasher::new();
    output.hash(&mut state);
    for path in changes
        .iter()
        .flat_map(|change| std::iter::once(&change.path).chain(change.original_path.as_ref()))
    {
        path.hash(&mut state);
        let path = root.join(path);
        if let Ok(metadata) = fs::symlink_metadata(path) {
            metadata.len().hash(&mut state);
            metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .hash(&mut state);
        }
    }
    let mut branch = DefaultHasher::new();
    output
        .split(|byte| *byte == 0)
        .find(|field| field.starts_with(b"## "))
        .hash(&mut branch);
    WorktreeSignature {
        state: state.finish(),
        branch: branch.finish(),
    }
}

pub fn diff(root: &Path, change: &Change) -> Result<String> {
    if !change.staged && change.code == '?' {
        const MAX_UNTRACKED_PREVIEW_BYTES: u64 = 128 * 1024;
        const MAX_UNTRACKED_PREVIEW_LINES: usize = 500;

        let path = root.join(&change.path);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("could not inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("could not read link {}", path.display()))?;
            return Ok(format!(
                "Untracked symbolic link: {}\n\nTarget: {}",
                change.path,
                target.display()
            ));
        }
        if !metadata.is_file() {
            return Ok(format!(
                "Untracked special file: {}\n\nPreview unavailable for this file type.",
                change.path
            ));
        }
        let (file, metadata) = open_regular_file(&path)
            .with_context(|| format!("could not safely read {}", path.display()))?;
        let mut bytes =
            Vec::with_capacity(metadata.len().min(MAX_UNTRACKED_PREVIEW_BYTES + 1) as usize);
        file.take(MAX_UNTRACKED_PREVIEW_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("could not read {}", path.display()))?;
        if bytes.contains(&0) {
            return Ok(format!("Binary untracked file\n\n{} bytes", metadata.len()));
        }
        let byte_truncated = bytes.len() > MAX_UNTRACKED_PREVIEW_BYTES as usize;
        bytes.truncate(MAX_UNTRACKED_PREVIEW_BYTES as usize);
        let text = String::from_utf8_lossy(&bytes);
        let mut lines = text.lines();
        let preview = lines
            .by_ref()
            .take(MAX_UNTRACKED_PREVIEW_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        let line_truncated = lines.next().is_some();
        let suffix = if byte_truncated || line_truncated {
            format!("\n\n[Preview truncated; file is {} bytes]", metadata.len())
        } else {
            String::new()
        };
        return Ok(format!(
            "Untracked file: {}\n\n{preview}{suffix}",
            change.path
        ));
    }

    let prefix = if change.staged {
        &["diff", "--cached", "--no-ext-diff", "--unified=3", "--"][..]
    } else {
        &["diff", "--no-ext-diff", "--unified=3", "--"][..]
    };
    let mut paths = Vec::new();
    if let Some(original) = &change.original_path {
        paths.push(original);
    }
    paths.push(&change.path);
    let output = run_path_command_limited(
        root,
        prefix,
        &paths,
        Limits::new(DIFF_PREVIEW_LIMIT, GIT_STDERR_LIMIT, GIT_TIMEOUT),
    )?;
    if output.timed_out {
        bail!("Git diff timed out");
    }
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(preview_text(output.stdout, output.stdout_truncated))
}

pub fn section_diff(root: &Path, changes: &[Change], staged: bool) -> Result<String> {
    let section = changes
        .iter()
        .filter(|change| change.staged == staged)
        .collect::<Vec<_>>();
    let (mut bytes, mut truncated) = if section.iter().any(|change| change.code != '?') {
        let args = if staged {
            &["diff", "--cached", "--no-ext-diff", "--unified=3"][..]
        } else {
            &["diff", "--no-ext-diff", "--unified=3"][..]
        };
        let output = run_limited(
            root,
            args,
            Limits::new(DIFF_PREVIEW_LIMIT, GIT_STDERR_LIMIT, SECTION_DIFF_TIMEOUT),
        )?;
        if output.timed_out {
            bail!("Git diff timed out");
        }
        if !output.status.success() {
            bail!("{}", clean_stderr(&output));
        }
        (output.stdout, output.stdout_truncated)
    } else {
        (Vec::new(), false)
    };

    for change in section
        .iter()
        .filter(|change| !staged && change.code == '?')
    {
        if truncated || bytes.len() >= DIFF_PREVIEW_LIMIT {
            truncated = true;
            break;
        }
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let description = diff(root, change)?;
        let prefix = format!("Untracked file: {}\n\n", change.path);
        let body = description.strip_prefix(&prefix).unwrap_or(&description);
        let old_path = quoted_diff_path(&change.path, b"a/")?;
        let new_path = quoted_diff_path(&change.path, b"b/")?;
        let line_count = body.lines().count();
        let hunk = if line_count == 0 {
            "@@ -0,0 +0,0 @@".to_owned()
        } else {
            format!("@@ -0,0 +1,{line_count} @@")
        };
        let mut content =
            format!("diff --git {old_path} {new_path}\n--- /dev/null\n+++ {new_path}\n{hunk}\n");
        for line in body.lines() {
            content.push('+');
            content.push_str(line);
            content.push('\n');
        }
        let remaining = DIFF_PREVIEW_LIMIT.saturating_sub(bytes.len());
        let content = content.as_bytes();
        let retained = content.len().min(remaining);
        bytes.extend_from_slice(&content[..retained]);
        if retained < content.len() {
            truncated = true;
            break;
        }
    }

    Ok(preview_text(bytes, truncated))
}

fn quoted_diff_path(path: &RepoPath, prefix: &[u8]) -> Result<String> {
    use std::fmt::Write;

    let mut bytes = prefix.to_vec();
    bytes.extend(path.git_bytes()?);
    let mut quoted = String::from("\"");
    for byte in bytes {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            b'\r' => quoted.push_str("\\r"),
            0x20..=0x7e => quoted.push(char::from(byte)),
            _ => write!(quoted, "\\{byte:03o}")?,
        }
    }
    quoted.push('"');
    Ok(quoted)
}

pub fn commit_diff(root: &Path, oid: &str) -> Result<String> {
    let output = run_limited(
        root,
        &[
            "show",
            "--format=",
            "--no-ext-diff",
            "--first-parent",
            "--unified=3",
            oid,
        ],
        Limits::new(DIFF_PREVIEW_LIMIT, GIT_STDERR_LIMIT, GIT_TIMEOUT),
    )?;
    if output.timed_out {
        bail!("Git commit preview timed out");
    }
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(preview_text(output.stdout, output.stdout_truncated))
}

pub fn branch_diff(root: &Path, target: &str, current: &str) -> Result<String> {
    let merge_base = run_limited(
        root,
        &["merge-base", target, current],
        Limits::new(1024, GIT_STDERR_LIMIT, SECTION_DIFF_TIMEOUT),
    )?;
    if merge_base.timed_out {
        bail!("Git merge-base timed out");
    }
    if !merge_base.status.success() {
        bail!("{}", clean_stderr(&merge_base));
    }
    let merge_base = String::from_utf8_lossy(&merge_base.stdout)
        .trim()
        .to_owned();
    if merge_base.is_empty() {
        bail!("Git did not find a merge base");
    }
    let output = run_limited(
        root,
        &["diff", "--no-ext-diff", "--unified=3", &merge_base, "--"],
        Limits::new(DIFF_PREVIEW_LIMIT, GIT_STDERR_LIMIT, SECTION_DIFF_TIMEOUT),
    )?;
    if output.timed_out {
        bail!("Git branch diff timed out");
    }
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut bytes = output.stdout;
    let mut truncated = output.stdout_truncated;
    if !truncated {
        let (changes, _) = status(root)?;
        let untracked = changes
            .into_iter()
            .filter(|change| !change.staged && change.code == '?')
            .collect::<Vec<_>>();
        if !untracked.is_empty() {
            let content = section_diff(root, &untracked, false)?;
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            let remaining = DIFF_PREVIEW_LIMIT.saturating_sub(bytes.len());
            let retained = content.len().min(remaining);
            bytes.extend_from_slice(&content.as_bytes()[..retained]);
            truncated = retained < content.len();
        }
    }
    if bytes.is_empty() {
        return Ok("No branch differences".to_owned());
    }
    Ok(preview_text(bytes, truncated))
}

pub fn commit_summaries(root: &Path, oids: &[String]) -> Result<HashMap<String, DiffSummary>> {
    if oids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut args = vec![
        "show",
        "--numstat",
        "-z",
        "--format=%H%x00",
        "--no-renames",
        "--first-parent",
    ];
    args.extend(oids.iter().map(String::as_str));
    let output = run(root, &args)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    parse_commit_summaries(&output.stdout)
}

fn parse_commit_summaries(bytes: &[u8]) -> Result<HashMap<String, DiffSummary>> {
    const MAX_FILES_PER_SUMMARY: usize = 2_000;
    let mut summaries = HashMap::new();
    let mut current: Option<(String, DiffSummary)> = None;
    for entry in bytes.split(|byte| *byte == 0) {
        let entry = entry.strip_prefix(b"\n").unwrap_or(entry);
        if (40..=64).contains(&entry.len()) && entry.iter().all(u8::is_ascii_hexdigit) {
            if let Some((oid, summary)) = current.replace((
                String::from_utf8_lossy(entry).into_owned(),
                DiffSummary::default(),
            )) {
                summaries.insert(oid, summary);
            }
        } else if let Some((_, summary)) = current.as_mut() {
            let mut fields = entry.splitn(3, |byte| *byte == b'\t');
            let (Some(additions), Some(deletions), Some(path)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            summary.additions = summary
                .additions
                .saturating_add(String::from_utf8_lossy(additions).parse().unwrap_or(0));
            summary.deletions = summary
                .deletions
                .saturating_add(String::from_utf8_lossy(deletions).parse().unwrap_or(0));
            if summary.files.len() < MAX_FILES_PER_SUMMARY {
                summary.files.push(RepoPath::from_git_bytes(path)?);
            } else {
                summary.files_truncated = true;
            }
        }
    }
    if let Some((oid, summary)) = current {
        summaries.insert(oid, summary);
    }
    Ok(summaries)
}

fn branch_name(root: &Path) -> Result<String> {
    let output = run(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }

    let output = run(root, &["rev-parse", "--short", "HEAD"])?;
    if output.status.success() {
        Ok(format!(
            "detached @ {}",
            String::from_utf8_lossy(&output.stdout).trim()
        ))
    } else {
        Ok("no commits".to_owned())
    }
}

fn status(root: &Path) -> Result<(Vec<Change>, WorktreeSignature)> {
    let output = run(
        root,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let changes = parse_status(&output.stdout)?;
    let signature = status_signature(root, &output.stdout, &changes);
    Ok((changes, signature))
}

fn parse_status(bytes: &[u8]) -> Result<Vec<Change>> {
    let fields: Vec<&[u8]> = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect();
    let mut changes = Vec::new();
    let mut index = 0;

    while index < fields.len() {
        let field = fields[index];
        if field.len() < 4 || field.starts_with(b"## ") {
            index += 1;
            continue;
        }

        let x = field[0] as char;
        let y = field[1] as char;
        let path = RepoPath::from_git_bytes(&field[3..])?;
        let renamed = matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C');
        let original_path = renamed
            .then(|| fields.get(index + 1))
            .flatten()
            .map(|path| RepoPath::from_git_bytes(path))
            .transpose()?;

        if x != ' ' && x != '?' && x != '!' {
            changes.push(Change {
                path: path.clone(),
                original_path: original_path.clone(),
                code: x,
                staged: true,
                additions: 0,
                deletions: 0,
            });
        }
        if y != ' ' && y != '!' {
            changes.push(Change {
                path,
                original_path,
                code: y,
                staged: false,
                additions: 0,
                deletions: 0,
            });
        }

        if renamed {
            index += 1;
        }
        index += 1;
    }

    changes.sort_by(|a, b| b.staged.cmp(&a.staged).then_with(|| a.path.cmp(&b.path)));
    Ok(changes)
}

fn populate_diff_stats(root: &Path, changes: &mut [Change]) -> Result<()> {
    let staged = if changes.iter().any(|change| change.staged) {
        diff_stats(root, true)?
    } else {
        HashMap::new()
    };
    let unstaged = if changes
        .iter()
        .any(|change| !change.staged && change.code != '?')
    {
        diff_stats(root, false)?
    } else {
        HashMap::new()
    };
    let mut untracked_budget = UNTRACKED_TOTAL_LINE_LIMIT;
    for change in changes {
        if change.code == '?' && !change.staged {
            if untracked_budget > 0
                && let Ok((lines, bytes_read)) = count_file_lines(
                    &root.join(&change.path),
                    untracked_budget.min(UNTRACKED_FILE_LINE_LIMIT),
                )
            {
                change.additions = lines;
                untracked_budget = untracked_budget.saturating_sub(bytes_read);
            }
            continue;
        }
        let stats = if change.staged { &staged } else { &unstaged };
        let (mut additions, mut deletions) = stats.get(&change.path).copied().unwrap_or_default();
        if let Some(original) = &change.original_path {
            let original_stats = stats.get(original).copied().unwrap_or_default();
            additions = additions.saturating_add(original_stats.0);
            deletions = deletions.saturating_add(original_stats.1);
        }
        change.additions = additions;
        change.deletions = deletions;
    }
    Ok(())
}

fn diff_stats(root: &Path, staged: bool) -> Result<HashMap<RepoPath, (u64, u64)>> {
    let args = if staged {
        ["diff", "--cached", "--no-renames", "--numstat", "-z"].as_slice()
    } else {
        ["diff", "--no-renames", "--numstat", "-z"].as_slice()
    };
    let output = run(root, args)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut stats = HashMap::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let (Some(additions), Some(deletions), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let additions = String::from_utf8_lossy(additions).parse().unwrap_or(0);
        let deletions = String::from_utf8_lossy(deletions).parse().unwrap_or(0);
        stats.insert(RepoPath::from_git_bytes(path)?, (additions, deletions));
    }
    Ok(stats)
}

fn count_file_lines(path: &Path, byte_limit: u64) -> Result<(u64, u64)> {
    let (file, metadata) = open_regular_file(path)?;
    let file_len = metadata.len();
    if file_len > byte_limit {
        return Ok((0, 0));
    }
    let mut reader = BufReader::new(file);
    let mut lines = 0u64;
    let mut has_bytes = false;
    let mut ends_with_newline = true;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        if buffer.contains(&0) {
            return Ok((0, file_len));
        }
        has_bytes = true;
        lines = lines.saturating_add(buffer.iter().filter(|byte| **byte == b'\n').count() as u64);
        ends_with_newline = buffer.last() == Some(&b'\n');
        let consumed = buffer.len();
        reader.consume(consumed);
    }
    Ok((lines + u64::from(has_bytes && !ends_with_newline), file_len))
}

fn open_regular_file(path: &Path) -> Result<(fs::File, fs::Metadata)> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("path is not a regular file");
    }
    Ok((file, metadata))
}

fn log(root: &Path) -> Result<Vec<Commit>> {
    read_log(
        root,
        &[
            "--date-order",
            "--ignore-missing",
            "--max-count=5001",
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
        ],
    )
}

fn branch_history(root: &Path) -> Result<Vec<Commit>> {
    read_log(
        root,
        &[
            "--date-order",
            "--ignore-missing",
            "--max-count=200",
            "HEAD",
        ],
    )
}

fn read_log(root: &Path, revisions: &[&str]) -> Result<Vec<Commit>> {
    let format = "--format=%H%x00%P%x00%D%x00%an%x00%ad%x00%s%x00%B%x00";
    let mut args = vec![
        "log",
        format,
        "--date=format:%Y-%m-%d %H:%M",
        "--decorate=short",
    ];
    args.extend_from_slice(revisions);
    let output = run(root, &args)?;

    if !output.status.success() {
        let stderr = clean_stderr(&output);
        if stderr.contains("does not have any commits yet")
            || stderr.contains("bad revision 'HEAD'")
            || stderr.contains("ambiguous argument 'HEAD'")
        {
            return Ok(Vec::new());
        }
        bail!("{stderr}");
    }

    Ok(parse_log(&output.stdout))
}

fn parse_log(bytes: &[u8]) -> Vec<Commit> {
    bytes
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>()
        .chunks_exact(7)
        .map(|fields| {
            let text = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
            let decorations = text(fields[2]);
            Commit {
                oid: text(trim_ascii(fields[0])),
                parents: text(fields[1])
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                refs: decorations
                    .split(", ")
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect(),
                author: text(fields[3]),
                date: compact_commit_date(text(fields[4])),
                subject: text(fields[5]),
                message: text(fields[6]),
                graph: Vec::new(),
            }
        })
        .collect()
}

fn compact_commit_date(date: String) -> String {
    let bytes = date.as_bytes();
    if !date.is_ascii()
        || bytes.len() != 16
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
    {
        return date;
    }
    let month = match &date[5..7] {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return date,
    };
    format!("{}{month} {}", &date[8..10], &date[11..16])
}

fn run(root: &Path, args: &[&str]) -> Result<Output> {
    let output = run_limited(root, args, git_limits())?;
    ensure_complete(&output, &format!("git {}", args.join(" ")))?;
    Ok(output)
}

fn run_limited(root: &Path, args: &[&str], limits: Limits) -> Result<Output> {
    process::run(base_command(root).args(args), limits)
        .with_context(|| format!("could not run git {}", args.join(" ")))
}

fn run_path_command(root: &Path, args: &[&str], paths: &[&RepoPath]) -> Result<Output> {
    let output = run_path_command_limited(root, args, paths, git_limits())?;
    ensure_complete(&output, &format!("git {}", args.join(" ")))?;
    Ok(output)
}

fn run_path_command_limited(
    root: &Path,
    args: &[&str],
    paths: &[&RepoPath],
    limits: Limits,
) -> Result<Output> {
    process::run(
        base_command(root)
            .args(args)
            .args(paths.iter().map(|path| path.as_os_str())),
        limits,
    )
    .with_context(|| format!("could not run git {}", args.join(" ")))
}

fn git_limits() -> Limits {
    Limits::new(GIT_STDOUT_LIMIT, GIT_STDERR_LIMIT, GIT_TIMEOUT)
}

fn ensure_complete(output: &Output, label: &str) -> Result<()> {
    if output.timed_out {
        bail!("{label} timed out");
    }
    if output.stdout_truncated {
        bail!("{label} produced more than {GIT_STDOUT_LIMIT} bytes");
    }
    Ok(())
}

fn base_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["--no-pager", "--no-optional-locks"])
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "false")
        .env("SSH_ASKPASS", "false")
        .env("GIT_EDITOR", "false")
        .env("GIT_SEQUENCE_EDITOR", "false")
        .stdin(Stdio::null());
    command
}

fn run_ok(root: &Path, args: &[&str]) -> Result<()> {
    let output = run(root, args)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

fn run_path_command_ok(root: &Path, args: &[&str], paths: &[&RepoPath]) -> Result<()> {
    let output = run_path_command(root, args, paths)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

fn clean_stderr(output: &Output) -> String {
    if output.timed_out {
        return "Git command timed out".to_owned();
    }
    let mut message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if output.stderr_truncated {
        message.push_str("\n[stderr truncated]");
    }
    if message.is_empty() {
        format!("Git exited with {}", output.status)
    } else {
        message
    }
}

fn command_output(output: Output) -> CommandOutput {
    let success = output.status.success() && !output.timed_out;
    let mut stderr = command_text(output.stderr, output.stderr_truncated, "stderr");
    if output.timed_out {
        stderr.push_str("\n[command timed out]");
    }
    CommandOutput {
        stdout: command_text(output.stdout, output.stdout_truncated, "stdout"),
        stderr,
        success,
        exit_code: output.status.code(),
    }
}

fn preview_text(bytes: Vec<u8>, truncated: bool) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str(&format!(
            "\n\n[Preview truncated at {DIFF_PREVIEW_LIMIT} bytes]"
        ));
    }
    text
}

fn command_text(bytes: Vec<u8>, truncated: bool, stream: &str) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str(&format!("\n[{stream} truncated]"));
    }
    text
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_main_worktree_with_spaces() {
        let parsed = parse_worktrees(
            b"worktree /repo/main worktree\0HEAD 0123456789abcdef\0branch refs/heads/main\0\0",
        )
        .unwrap();

        assert_eq!(
            parsed,
            [LinkedWorktree {
                path: PathBuf::from("/repo/main worktree"),
                head: Some("0123456789abcdef".to_owned()),
                branch: Some("refs/heads/main".to_owned()),
                is_main: true,
                is_detached: false,
                is_bare: false,
                locked: false,
                locked_reason: None,
                prunable: false,
                prunable_reason: None,
            }]
        );
    }

    #[test]
    fn parses_detached_locked_and_prunable_worktrees() {
        let parsed = parse_worktrees(
            b"worktree /repo\0HEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0branch refs/heads/main\0\0worktree /repo/detached\0HEAD bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\0detached\0locked reason with spaces\0prunable gitdir file points to non-existent location\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].is_main);
        assert!(!parsed[1].is_main);
        assert!(parsed[1].is_detached);
        assert_eq!(parsed[1].branch, None);
        assert!(parsed[1].locked);
        assert_eq!(
            parsed[1].locked_reason.as_deref(),
            Some("reason with spaces")
        );
        assert!(parsed[1].prunable);
        assert_eq!(
            parsed[1].prunable_reason.as_deref(),
            Some("gitdir file points to non-existent location")
        );
    }

    #[test]
    fn parses_bare_and_reasonless_locked_worktrees() {
        let parsed = parse_worktrees(
            b"worktree /srv/repository.git\0bare\0locked\0prunable metadata missing\0\0",
        )
        .unwrap();

        assert!(parsed[0].is_main);
        assert!(parsed[0].is_bare);
        assert_eq!(parsed[0].head, None);
        assert_eq!(parsed[0].branch, None);
        assert!(parsed[0].locked);
        assert_eq!(parsed[0].locked_reason, None);
        assert_eq!(
            parsed[0].prunable_reason.as_deref(),
            Some("metadata missing")
        );
    }

    #[test]
    fn rejects_malformed_worktree_records() {
        for malformed in [
            b"HEAD abc\0branch refs/heads/main\0\0".as_slice(),
            b"worktree /repo\0HEAD abc\0branch refs/heads/main\0".as_slice(),
            b"worktree /repo\0HEAD abc\0\0".as_slice(),
            b"worktree /repo\0HEAD abc\0branch refs/heads/main\0detached\0\0".as_slice(),
            b"worktree /repo.git\0bare\0HEAD abc\0detached\0\0".as_slice(),
        ] {
            assert!(parse_worktrees(malformed).is_err(), "{malformed:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_worktree_paths() {
        use std::os::unix::ffi::OsStrExt;

        let parsed =
            parse_worktrees(b"worktree /repo/linked-\xff\0HEAD 0123456789abcdef\0detached\0\0")
                .unwrap();

        assert_eq!(parsed[0].path.as_os_str().as_bytes(), b"/repo/linked-\xff");
    }

    #[test]
    fn lists_and_safely_removes_a_real_linked_worktree() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main repository");
        let linked = directory.path().join("linked worktree");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Test Author"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        git(&root, &["commit", "-m", "base"]);
        git(
            &root,
            &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
        );

        let common = common_git_dir(&root).unwrap();
        assert_eq!(common_git_dir(&linked).unwrap(), common);
        let listed = list_worktrees(&common).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].path, fs::canonicalize(&root).unwrap());
        assert!(listed[0].is_main);
        assert_eq!(listed[0].branch.as_deref(), Some("refs/heads/main"));
        assert_eq!(listed[1].path, fs::canonicalize(&linked).unwrap());
        assert!(!listed[1].is_main);
        assert_eq!(listed[1].branch.as_deref(), Some("refs/heads/topic"));

        fs::write(linked.join("tracked.txt"), "dirty\n").unwrap();
        assert!(remove_worktree(&common, &linked).is_err());
        assert!(linked.exists());
        git(&linked, &["restore", "tracked.txt"]);

        remove_worktree(&common, &linked).unwrap();
        assert!(!linked.exists());
        assert_eq!(list_worktrees(&common).unwrap().len(), 1);
    }

    #[test]
    fn parses_staged_and_unstaged_status_entries() {
        let parsed = parse_status(b"M  staged.rs\0 M changed.rs\0?? new.rs\0MM both.rs\0").unwrap();
        assert_eq!(parsed.len(), 5);
        assert!(
            parsed
                .iter()
                .any(|change| change.path == "staged.rs" && change.staged)
        );
        assert!(
            parsed
                .iter()
                .any(|change| change.path == "new.rs" && !change.staged)
        );
        assert_eq!(
            parsed
                .iter()
                .filter(|change| change.path == "both.rs")
                .count(),
            2
        );
    }

    #[test]
    fn preserves_both_paths_for_renames() {
        let parsed = parse_status(b"R  new.rs\0old.rs\0").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "new.rs");
        assert_eq!(parsed[0].original_path.as_ref().unwrap(), "old.rs");
    }

    #[test]
    fn untracked_line_counts_respect_the_read_budget() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new.txt");
        fs::write(&path, "a\nb\n").unwrap();

        assert_eq!(count_file_lines(&path, 4).unwrap(), (2, 4));
        assert_eq!(count_file_lines(&path, 3).unwrap(), (0, 0));
    }

    #[test]
    fn section_diffs_include_every_file_in_the_selected_section() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Section Test"]);
        git(root, &["config", "user.email", "section@example.com"]);
        fs::write(root.join("staged.txt"), "before\n").unwrap();
        fs::write(root.join("unstaged.txt"), "before\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);

        fs::write(root.join("staged.txt"), "after\n").unwrap();
        fs::write(root.join("unstaged.txt"), "after\n").unwrap();
        fs::write(root.join("untracked.txt"), "new\n").unwrap();
        git(root, &["add", "staged.txt"]);
        let (changes, _) = status(root).unwrap();

        let staged = section_diff(root, &changes, true).unwrap();
        assert!(staged.contains("diff --git a/staged.txt b/staged.txt"));
        assert!(!staged.contains("unstaged.txt"));
        assert!(!staged.contains("untracked.txt"));

        let unstaged = section_diff(root, &changes, false).unwrap();
        assert!(unstaged.contains("diff --git a/unstaged.txt b/unstaged.txt"));
        assert!(unstaged.contains("untracked.txt"));
        assert_eq!(unstaged.matches("diff --git").count(), 2);
    }

    #[test]
    fn branch_diffs_show_changes_unique_to_the_current_branch() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Branch Diff Test"]);
        git(root, &["config", "user.email", "branch-diff@example.com"]);
        fs::write(root.join("shared.txt"), "base\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["switch", "-c", "feature"]);
        fs::write(root.join("feature.txt"), "feature only\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "feature"]);
        git(root, &["switch", "main"]);
        fs::write(root.join("target.txt"), "target only\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "target"]);
        git(root, &["switch", "feature"]);
        git(root, &["tag", "main", "feature"]);
        fs::write(root.join("feature.txt"), "feature only\nunstaged\n").unwrap();
        fs::write(root.join("staged.txt"), "staged\n").unwrap();
        fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
        git(root, &["add", "staged.txt"]);

        let preview = branch_diff(root, "refs/heads/main", "refs/heads/feature").unwrap();

        assert!(preview.contains("diff --git a/feature.txt b/feature.txt"));
        assert!(preview.contains("+feature only"));
        assert!(preview.contains("+unstaged"));
        assert!(preview.contains("diff --git a/staged.txt b/staged.txt"));
        assert!(preview.contains("+staged"));
        assert!(preview.contains("diff --git \"a/untracked.txt\" \"b/untracked.txt\""));
        assert!(preview.contains("+untracked"));
        assert!(!preview.contains("target.txt"));
        assert_eq!(branch_name(root).unwrap(), "feature");
    }

    #[cfg(unix)]
    #[test]
    fn untracked_symlink_preview_does_not_read_its_target() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), "outside secret").unwrap();
        symlink(outside.path(), workspace.path().join("link")).unwrap();
        let change = Change {
            path: RepoPath::from("link"),
            original_path: None,
            code: '?',
            staged: false,
            additions: 0,
            deletions: 0,
        };

        let preview = diff(workspace.path(), &change).unwrap();

        assert!(preview.contains("Untracked symbolic link"));
        assert!(!preview.contains("outside secret"));
        let section = section_diff(workspace.path(), &[change], false).unwrap();
        assert!(section.contains("diff --git \"a/link\" \"b/link\""));
        assert!(section.contains("Untracked symbolic link"));
        assert!(!section.contains("outside secret"));
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_rejected_before_opening() {
        let workspace = tempfile::tempdir().unwrap();
        let fifo = workspace.path().join("pipe");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let path = RepoPath::from("pipe");
        let change = Change {
            path: path.clone(),
            original_path: None,
            code: '?',
            staged: false,
            additions: 0,
            deletions: 0,
        };

        assert!(
            file_content(workspace.path(), &path)
                .unwrap()
                .contains("special file")
        );
        assert!(
            diff(workspace.path(), &change)
                .unwrap()
                .contains("Untracked special file")
        );
        assert!(
            section_diff(workspace.path(), &[change], false)
                .unwrap()
                .contains("Untracked special file")
        );
        assert!(count_file_lines(&fifo, u64::MAX).is_err());
    }

    #[test]
    fn does_not_climb_to_an_enclosing_repository() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        let nested = root.join("nested/config");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("settings.toml"), "theme = 'test'\n").unwrap();

        assert_eq!(discover(root).unwrap(), fs::canonicalize(root).unwrap());
        let error = discover(&nested).unwrap_err().to_string();
        assert!(error.contains("not a repository root"));
        assert!(error.contains(&fs::canonicalize(root).unwrap().display().to_string()));
        assert!(load(&nested).is_err());

        let workspace = load_or_local(&nested).unwrap();
        assert_eq!(workspace.kind, RepositoryKind::Local);
        assert_eq!(workspace.root, fs::canonicalize(&nested).unwrap());
        assert_eq!(workspace.branch, "local");
        assert_eq!(workspace.files, ["settings.toml"]);
        assert!(workspace.changes.is_empty());
        assert!(workspace.history.is_empty());
        assert!(workspace.commits.is_empty());
    }

    #[test]
    fn loads_a_plain_directory_as_a_local_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("src/components")).unwrap();
        fs::write(root.join("README.md"), "local\n").unwrap();
        fs::write(root.join("src/components/card.rs"), "component\n").unwrap();

        let workspace = load_or_local(root).unwrap();
        assert_eq!(workspace.kind, RepositoryKind::Local);
        assert_eq!(workspace.root, fs::canonicalize(root).unwrap());
        assert_eq!(workspace.branch, "local");
        assert_eq!(workspace.files, ["README.md", "src/components/card.rs"]);
        assert!(workspace.changes.is_empty());
        assert!(workspace.history.is_empty());
        assert!(workspace.commits.is_empty());
    }

    #[test]
    fn bootstrap_defers_repository_facets_until_full_refresh() {
        let git_directory = tempfile::tempdir().unwrap();
        let git_root = git_directory.path();
        git(git_root, &["init", "-b", "main"]);
        git(git_root, &["config", "user.name", "Test Author"]);
        git(git_root, &["config", "user.email", "test@example.com"]);
        fs::write(git_root.join("tracked.txt"), "tracked\n").unwrap();
        git(git_root, &["add", "tracked.txt"]);
        git(git_root, &["commit", "-m", "initial"]);

        let mut repository = bootstrap_or_local(git_root).unwrap();
        assert_eq!(repository.kind, RepositoryKind::Git);
        assert!(repository.common_dir.is_some());
        assert!(!repository.details_ready);
        assert!(repository.files.is_empty());
        assert!(repository.history.is_empty());
        assert!(repository.commits.is_empty());

        let update =
            refresh_repository(&repository.root, repository.kind, RefreshScope::ALL).unwrap();
        repository.apply(update);
        assert!(repository.details_ready);
        assert_eq!(repository.branch, "main");
        assert_eq!(repository.files, ["tracked.txt"]);
        assert_eq!(repository.history.len(), 1);
        assert_eq!(repository.commits.len(), 1);

        let local_directory = tempfile::tempdir().unwrap();
        fs::write(local_directory.path().join("local.txt"), "local\n").unwrap();
        let mut workspace = bootstrap_or_local(local_directory.path()).unwrap();
        assert_eq!(workspace.kind, RepositoryKind::Local);
        assert!(!workspace.details_ready);
        assert!(workspace.files.is_empty());

        let update =
            refresh_repository(&workspace.root, workspace.kind, RefreshScope::ALL).unwrap();
        workspace.apply(update);
        assert!(workspace.details_ready);
        assert_eq!(workspace.files, ["local.txt"]);
    }

    #[test]
    fn recognizes_github_remote_urls() {
        assert!(is_github_remote_url("git@github.com:owner/repo.git"));
        assert!(is_github_remote_url("https://github.com/owner/repo.git"));
        assert!(is_github_remote_url("ssh://git@github.com/owner/repo.git"));
        assert!(!is_github_remote_url("https://gitlab.com/owner/repo.git"));
    }

    #[test]
    fn parses_complete_multiline_commit_messages() {
        let commits = parse_log(
            b"abc\0parent\0HEAD -> main\0Ada\x002026-08-07 18:29\0Subject\0Subject\n\nBody \x1f line\n\nFinal \x1e note\0",
        );

        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].date, "07Aug 18:29");
        assert_eq!(commits[0].subject, "Subject");
        assert_eq!(
            commits[0].message,
            "Subject\n\nBody \u{1f} line\n\nFinal \u{1e} note"
        );
    }

    #[test]
    fn loads_a_real_repository_with_a_merge_and_worktree_change() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("base.txt"), "base\n").unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["checkout", "-b", "feature"]);
        fs::write(root.join("feature.txt"), "feature\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "feature"]);
        git(root, &["checkout", "main"]);
        fs::write(root.join("main.txt"), "main\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "main work"]);
        git(
            root,
            &["merge", "--no-ff", "feature", "-m", "merge feature"],
        );
        fs::write(root.join("main.txt"), "changed\n").unwrap();
        fs::create_dir(root.join("ignored")).unwrap();
        fs::write(root.join("ignored/cache.txt"), "generated\n").unwrap();

        let repo = load(root).unwrap();
        assert_eq!(repo.branch, "main");
        assert_eq!(
            repo.branches
                .iter()
                .map(|branch| (branch.name.as_str(), branch.current))
                .collect::<Vec<_>>(),
            [("main", true), ("feature", false)]
        );
        assert_eq!(repo.commits.len(), 4);
        assert_eq!(repo.history.len(), 4);
        assert_eq!(repo.commits[0].date.len(), 11);
        assert_eq!(repo.commits[0].date.as_bytes().get(5), Some(&b' '));
        assert_eq!(repo.commits[0].date.as_bytes().get(8), Some(&b':'));
        assert!(
            repo.history[0]
                .refs
                .iter()
                .any(|name| name.contains("HEAD"))
        );
        assert_eq!(repo.commits[0].parents.len(), 2);
        assert!(repo.commits[0].graph.iter().any(|cell| cell.symbol == '─'));
        assert_eq!(repo.changes.len(), 1);
        assert_eq!(repo.changes[0].path, "main.txt");
        assert_eq!(
            (repo.changes[0].additions, repo.changes[0].deletions),
            (1, 1)
        );
        assert!(repo.files.iter().any(|path| path == "base.txt"));
        assert!(repo.files.iter().any(|path| path == "feature.txt"));
        assert!(repo.files.iter().any(|path| path == "ignored/cache.txt"));
        assert!(
            !repo
                .changes
                .iter()
                .any(|change| change.path == "ignored/cache.txt")
        );
        assert_eq!(
            file_content(root, &RepoPath::from("main.txt")).unwrap(),
            "changed\n"
        );
        let selected_commit_diff = commit_diff(root, &repo.history[0].oid).unwrap();
        assert!(selected_commit_diff.contains("diff --git"));

        stage(root, &repo.changes[0]).unwrap();
        let staged = load(root).unwrap();
        assert!(staged.changes[0].staged);
        assert_eq!(
            (staged.changes[0].additions, staged.changes[0].deletions),
            (1, 1)
        );

        unstage(root, &staged.changes[0]).unwrap();
        let unstaged = load(root).unwrap();
        assert!(!unstaged.changes[0].staged);

        stage(root, &unstaged.changes[0]).unwrap();
        let output = super::commit(root, "update main").unwrap();
        assert!(output.success, "{}", output.stderr);
        let committed = load(root).unwrap();
        assert!(committed.changes.is_empty());
        assert_eq!(committed.commits.len(), 5);
        assert_eq!(committed.history.len(), 5);

        let fetched = super::fetch(root).unwrap();
        assert!(fetched.success, "{}", fetched.stderr);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_invalid_utf8_inventory_status_and_whole_file_operations() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);

        let first_name = OsString::from_vec(b"collision-\x80.txt".to_vec());
        let second_name = OsString::from_vec(b"collision-\x81.txt".to_vec());
        let first_path = RepoPath::from(PathBuf::from(&first_name));
        let second_path = RepoPath::from(PathBuf::from(&second_name));
        assert_eq!(first_name.to_string_lossy(), second_name.to_string_lossy());

        fs::write(root.join(&first_name), "first original\n").unwrap();
        fs::write(root.join(&second_name), "second original\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "invalid byte paths"]);
        fs::write(root.join(&first_name), "first changed\n").unwrap();
        fs::write(root.join(&second_name), "second changed\n").unwrap();

        let repo = load(root).unwrap();
        assert!(repo.files.contains(&first_path));
        assert!(repo.files.contains(&second_path));
        let first = repo
            .changes
            .iter()
            .find(|change| change.path == first_path)
            .unwrap()
            .clone();
        let second = repo
            .changes
            .iter()
            .find(|change| change.path == second_path)
            .unwrap();
        assert_ne!(first.path, second.path);
        let first_diff = diff(root, &first).unwrap();
        assert!(first_diff.contains("first changed"));
        assert!(!first_diff.contains("second changed"));

        stage(root, &first).unwrap();
        let staged = load(root).unwrap();
        assert!(
            staged
                .changes
                .iter()
                .any(|change| change.path == first_path && change.staged)
        );
        assert!(
            staged
                .changes
                .iter()
                .any(|change| change.path == second_path && !change.staged)
        );

        let staged_first = staged
            .changes
            .iter()
            .find(|change| change.path == first_path && change.staged)
            .unwrap();
        unstage(root, staged_first).unwrap();
        let unstaged = load(root).unwrap();
        let unstaged_first = unstaged
            .changes
            .iter()
            .find(|change| change.path == first_path && !change.staged)
            .unwrap()
            .clone();
        discard_unstaged(root, &unstaged_first).unwrap();

        assert_eq!(
            fs::read(root.join(&first_name)).unwrap(),
            b"first original\n"
        );
        assert_eq!(
            fs::read(root.join(&second_name)).unwrap(),
            b"second changed\n"
        );
    }

    #[test]
    fn git_files_include_untracked_and_ignored_files_but_exclude_deleted_tracked_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        fs::write(root.join("untracked.txt"), "new\n").unwrap();
        fs::create_dir_all(root.join("empty/nested")).unwrap();
        fs::create_dir_all(root.join("empty/ignored")).unwrap();
        fs::create_dir(root.join("config")).unwrap();
        fs::create_dir(root.join("logs")).unwrap();
        fs::write(root.join("logs/debug.log"), "debug\n").unwrap();
        fs::write(
            root.join(".gitignore"),
            "empty/ignored/\n.env*\nconfig/\nlogs/*.log\n",
        )
        .unwrap();
        fs::write(root.join(".env"), "SECRET=value\n").unwrap();
        fs::write(root.join(".env.local"), "SECRET=local\n").unwrap();
        fs::write(root.join(".envrc"), "not an env file\n").unwrap();
        fs::write(root.join("config/.env.production"), "SECRET=prod\n").unwrap();
        fs::remove_file(root.join("tracked.txt")).unwrap();

        let (files, directories, truncated) = inventory::git_entries(root).unwrap();
        assert!(!truncated);
        assert_eq!(
            files,
            [
                ".env",
                ".env.local",
                ".envrc",
                ".gitignore",
                "config/.env.production",
                "logs/debug.log",
                "untracked.txt"
            ]
        );
        assert_eq!(
            directories,
            ["config", "empty", "empty/ignored", "empty/nested", "logs"]
        );
    }

    #[test]
    fn gitlinks_are_exposed_as_directories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("tracked"), "content").unwrap();
        git(root, &["add", "tracked"]);
        git(root, &["commit", "-m", "initial"]);
        let oid_output = run(root, &["rev-parse", "HEAD"]).unwrap();
        let oid = String::from_utf8_lossy(&oid_output.stdout);
        let cache_info = format!("160000,{},{path}", oid.trim(), path = "module");
        git(root, &["update-index", "--add", "--cacheinfo", &cache_info]);
        fs::create_dir(root.join("module")).unwrap();

        let (files, directories, truncated) = inventory::git_entries(root).unwrap();
        assert!(!truncated);
        assert!(!files.iter().any(|path| path == "module"));
        assert!(directories.iter().any(|path| path == "module"));
    }

    #[test]
    fn git_files_exclude_absent_sparse_checkout_entries() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        fs::write(root.join("sparse.txt"), "tracked\n").unwrap();
        git(root, &["add", "sparse.txt"]);
        git(root, &["update-index", "--skip-worktree", "sparse.txt"]);
        fs::remove_file(root.join("sparse.txt")).unwrap();

        assert!(inventory::git_entries(root).unwrap().0.is_empty());
    }

    #[test]
    fn truncates_large_untracked_previews() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("large.txt"), vec![b'x'; 256 * 1024]).unwrap();
        let change = Change {
            path: "large.txt".into(),
            original_path: None,
            code: '?',
            staged: false,
            additions: 0,
            deletions: 0,
        };

        let preview = diff(root, &change).unwrap();
        assert!(preview.contains("Preview truncated; file is 262144 bytes"));
        assert!(preview.len() < 140 * 1024);
    }

    #[test]
    fn scoped_refresh_updates_only_requested_facets() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-m", "base"]);

        let mut repository = load(root).unwrap();
        let original_commit = repository.commits[0].oid.clone();
        let original_files = repository.files.clone();
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("new.txt"), "new\n").unwrap();

        let update = refresh_repository(
            &repository.root,
            RepositoryKind::Git,
            RefreshScope::WORKTREE,
        )
        .unwrap();
        assert!(update.worktree.is_some());
        assert!(update.inventory.is_none());
        assert!(update.history.is_none());
        assert!(update.graph.is_none());
        assert!(update.refs.is_none());
        repository.apply(update);
        assert_eq!(repository.files, original_files);
        assert_eq!(repository.commits[0].oid, original_commit);
        assert!(
            repository
                .changes
                .iter()
                .any(|change| change.path == "tracked.txt")
        );

        let update = refresh_repository(
            &repository.root,
            RepositoryKind::Git,
            RefreshScope::WORKTREE_AND_INVENTORY,
        )
        .unwrap();
        assert!(update.worktree.is_some());
        assert!(update.inventory.is_some());
        repository.apply(update);
        assert!(repository.files.iter().any(|file| file == "new.txt"));
        assert_eq!(repository.commits[0].oid, original_commit);
    }

    #[test]
    fn parses_batched_commit_change_summaries() {
        let summaries = parse_commit_summaries(
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0\0\n12\t3\tsrc/app.rs\0-\t-\tassets/logo\x1e.png\0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\0\0\n4\t0\tREADME.md\0",
        )
        .unwrap();

        assert_eq!(
            summaries["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
            DiffSummary {
                files: vec!["src/app.rs".into(), "assets/logo\u{1e}.png".into()],
                files_truncated: false,
                additions: 12,
                deletions: 3,
            }
        );
        assert_eq!(
            summaries["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
            DiffSummary {
                files: vec!["README.md".into()],
                files_truncated: false,
                additions: 4,
                deletions: 0,
            }
        );
    }

    #[test]
    fn bounds_paths_retained_by_commit_summaries() {
        let mut output = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0\0\n".to_vec();
        for index in 0..=2_000 {
            output.extend_from_slice(format!("1\t2\tfile-{index}\0").as_bytes());
        }

        let summaries = parse_commit_summaries(&output).unwrap();
        let summary = &summaries["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"];
        assert_eq!(summary.files.len(), 2_000);
        assert!(summary.files_truncated);
        assert_eq!(summary.additions, 2_001);
        assert_eq!(summary.deletions, 4_002);
    }

    #[test]
    fn stages_only_the_selected_hunk() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        let original = (1..=20)
            .map(|line| format!("line {line:02}"))
            .collect::<Vec<_>>();
        fs::write(root.join("split.txt"), original.join("\n") + "\n").unwrap();
        git(root, &["add", "split.txt"]);
        git(root, &["commit", "-m", "base"]);

        let mut changed = original;
        changed[1] = "changed first".to_owned();
        changed[18] = "changed second".to_owned();
        fs::write(root.join("split.txt"), changed.join("\n") + "\n").unwrap();
        let change = load(root).unwrap().changes.remove(0);
        let patch = diff(root, &change).unwrap();
        assert_eq!(
            patch.lines().filter(|line| line.starts_with("@@")).count(),
            2
        );

        stage_hunk(root, &patch, 0).unwrap();

        let staged = String::from_utf8(
            run(root, &["diff", "--cached", "--", "split.txt"])
                .unwrap()
                .stdout,
        )
        .unwrap();
        let unstaged =
            String::from_utf8(run(root, &["diff", "--", "split.txt"]).unwrap().stdout).unwrap();
        assert!(staged.contains("changed first"));
        assert!(!staged.contains("changed second"));
        assert!(!unstaged.contains("changed first"));
        assert!(unstaged.contains("changed second"));
        let changes = load(root).unwrap().changes;
        assert!(
            changes
                .iter()
                .any(|change| change.path == "split.txt" && change.staged)
        );
        assert!(
            changes
                .iter()
                .any(|change| change.path == "split.txt" && !change.staged)
        );
    }

    #[test]
    fn discards_only_selected_unstaged_changes_and_preserves_the_index() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        fs::write(root.join("other.txt"), "other base\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);

        fs::write(root.join("tracked.txt"), "staged\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        fs::write(root.join("tracked.txt"), "unstaged\n").unwrap();
        fs::write(root.join("other.txt"), "other unstaged\n").unwrap();
        let change = load(root)
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == "tracked.txt" && !change.staged)
            .unwrap();

        discard_unstaged(root, &change).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "staged\n"
        );
        assert_eq!(
            String::from_utf8(run(root, &["show", ":tracked.txt"]).unwrap().stdout).unwrap(),
            "staged\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("other.txt")).unwrap(),
            "other unstaged\n"
        );
        let changes = load(root).unwrap().changes;
        assert_eq!(
            changes
                .iter()
                .filter(|change| change.path == "tracked.txt")
                .count(),
            1
        );
        assert!(
            changes
                .iter()
                .any(|change| change.path == "tracked.txt" && change.staged)
        );
        assert!(
            changes
                .iter()
                .any(|change| change.path == "other.txt" && !change.staged)
        );
    }

    #[test]
    fn discards_untracked_files_and_restores_deleted_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        fs::remove_file(root.join("tracked.txt")).unwrap();
        fs::write(root.join("remove.txt"), "remove\n").unwrap();
        fs::write(root.join("keep.txt"), "keep\n").unwrap();
        let changes = load(root).unwrap().changes;

        let deleted = changes
            .iter()
            .find(|change| change.path == "tracked.txt" && !change.staged)
            .unwrap();
        discard_unstaged(root, deleted).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "tracked\n"
        );

        let untracked = changes
            .iter()
            .find(|change| change.path == "remove.txt" && !change.staged)
            .unwrap();
        discard_unstaged(root, untracked).unwrap();
        assert!(!root.join("remove.txt").exists());
        assert!(root.join("keep.txt").exists());
    }

    #[test]
    fn discards_an_unstaged_rename() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("old.txt"), "content\n").unwrap();
        git(root, &["add", "old.txt"]);
        git(root, &["commit", "-m", "base"]);
        fs::rename(root.join("old.txt"), root.join("new.txt")).unwrap();
        let change = Change {
            path: "new.txt".into(),
            original_path: Some("old.txt".into()),
            code: 'R',
            staged: false,
            additions: 0,
            deletions: 0,
        };

        discard_unstaged(root, &change).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("old.txt")).unwrap(),
            "content\n"
        );
        assert!(!root.join("new.txt").exists());
        assert!(load(root).unwrap().changes.is_empty());
    }

    #[test]
    fn refuses_to_discard_an_unresolved_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("conflict.txt"), "base\n").unwrap();
        git(root, &["add", "conflict.txt"]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["switch", "-c", "side"]);
        fs::write(root.join("conflict.txt"), "side\n").unwrap();
        git(root, &["commit", "-am", "side"]);
        git(root, &["switch", "main"]);
        fs::write(root.join("conflict.txt"), "main\n").unwrap();
        git(root, &["commit", "-am", "main"]);
        let merge = run(root, &["merge", "side"]).unwrap();
        assert!(!merge.status.success());
        let before = fs::read_to_string(root.join("conflict.txt")).unwrap();
        let change = load(root)
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == "conflict.txt" && !change.staged)
            .unwrap();

        assert!(discard_unstaged(root, &change).is_err());
        assert_eq!(
            fs::read_to_string(root.join("conflict.txt")).unwrap(),
            before
        );
        assert!(
            !run(root, &["ls-files", "--unmerged"])
                .unwrap()
                .stdout
                .is_empty()
        );
    }

    #[test]
    fn protects_checked_out_conventional_and_default_branches() {
        let branch = |name: &str, current: bool, default: bool| Branch {
            name: name.to_owned(),
            upstream: String::new(),
            oid: String::new(),
            date: String::new(),
            subject: String::new(),
            remote: false,
            current,
            default,
        };

        assert!(branch_delete_protection(&branch("topic", true, false)).is_some());
        for name in ["main", "master", "dev"] {
            assert!(branch_delete_protection(&branch(name, false, false)).is_some());
        }
        assert!(branch_delete_protection(&branch("stable", false, true)).is_some());
        assert!(branch_delete_protection(&branch("topic", false, false)).is_none());
    }

    #[test]
    fn deletes_a_local_branch_and_its_remote_ref() {
        let directory = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-m", "initial"]);
        git(root, &["branch", "cleanup"]);
        git(root, &["switch", "cleanup"]);
        fs::write(root.join("trash.txt"), "unmerged\n").unwrap();
        git(root, &["add", "trash.txt"]);
        git(root, &["commit", "-m", "unmerged cleanup work"]);
        git(root, &["switch", "main"]);
        git(remote.path(), &["init", "--bare"]);
        git(
            root,
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(root, &["push", "origin", "cleanup"]);

        git(root, &["branch", "dev"]);
        assert!(delete_branch(root, "dev", None, true).is_err());
        assert!(
            run(root, &["show-ref", "--verify", "refs/heads/dev"])
                .unwrap()
                .status
                .success()
        );

        git(root, &["branch", "stable"]);
        git(
            root,
            &[
                "update-ref",
                "refs/remotes/origin/stable",
                "refs/heads/stable",
            ],
        );
        git(
            root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/stable",
            ],
        );
        let stable = repository_branches(root)
            .unwrap()
            .into_iter()
            .find(|branch| branch.name == "stable" && !branch.remote)
            .unwrap();
        assert!(stable.default);
        assert!(delete_branch(root, "stable", None, true).is_err());
        assert!(
            run(root, &["show-ref", "--verify", "refs/heads/stable"])
                .unwrap()
                .status
                .success()
        );

        assert!(delete_branch(root, "cleanup", Some(("origin", "cleanup")), false).is_err());
        delete_branch(root, "cleanup", Some(("origin", "cleanup")), true).unwrap();

        assert!(
            !run(root, &["show-ref", "--verify", "refs/heads/cleanup"])
                .unwrap()
                .status
                .success()
        );
        assert!(
            !run(
                remote.path(),
                &["show-ref", "--verify", "refs/heads/cleanup"]
            )
            .unwrap()
            .status
            .success()
        );
    }

    #[test]
    fn checks_out_local_and_remote_branches_without_overriding_git_safety() {
        let directory = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();
        let occupied = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test Author"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("tracked.txt"), "main\n").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-m", "initial"]);
        git(root, &["branch", "topic"]);

        let output = checkout_branch(root, "topic", false).unwrap();
        assert!(output.success, "{}", output.stderr);
        assert_eq!(branch_name(root).unwrap(), "topic");
        git(root, &["switch", "main"]);

        git(
            root,
            &[
                "worktree",
                "add",
                occupied.path().to_str().unwrap(),
                "topic",
            ],
        );
        let output = checkout_branch(root, "topic", false).unwrap();
        assert!(!output.success);
        assert_eq!(branch_name(root).unwrap(), "main");

        git(remote.path(), &["init", "--bare"]);
        git(
            root,
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(root, &["branch", "remote-topic"]);
        git(root, &["push", "origin", "remote-topic"]);
        git(root, &["branch", "-D", "remote-topic"]);
        let output = checkout_branch(root, "origin/remote-topic", true).unwrap();
        assert!(output.success, "{}", output.stderr);
        assert_eq!(branch_name(root).unwrap(), "remote-topic");
        let upstream = run(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&upstream.stdout).trim(),
            "origin/remote-topic"
        );
    }

    #[cfg(test)]
    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if args.first() == Some(&"init") {
            git(root, &["config", "core.autocrlf", "false"]);
        }
    }
}
