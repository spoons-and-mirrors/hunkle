use super::*;

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
    let ignored_files = Vec::<RepoPath>::new();
    let directories = Vec::<RepoPath>::new();
    RepositoryData {
        root,
        common_dir,
        kind,
        branch,
        ahead: 0,
        behind: 0,
        changes_fingerprint: fingerprint(&changes),
        files_fingerprint: fingerprint(&(&files, &directories, &ignored_files)),
        changes,
        files,
        ignored_files,
        directories,
        history: Vec::new(),
        commits: Vec::new(),
        inventory_truncated: false,
        change_counts: (0, 0),
        graph_width: 0,
        graph_truncated: false,
        branches: Vec::new(),
        worktree_signature: None,
        details_ready: false,
    }
}

fn load_git_root(root: PathBuf) -> Result<RepositoryData> {
    let (common_dir, worktree, inventory, history, graph, refs) = thread::scope(|scope| {
        let common_dir = scope.spawn(|| common_git_dir(&root));
        let worktree = scope.spawn(|| load_worktree(&root, None));
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
        ahead: worktree.sync.ahead,
        behind: worktree.sync.behind,
        changes: worktree.changes,
        files: inventory.files,
        ignored_files: inventory.ignored_files,
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
        worktree_signature: Some(worktree.signature),
        details_ready: true,
    })
}

#[cfg(test)]
pub fn refresh_repository(
    root: &Path,
    kind: RepositoryKind,
    scope: RefreshScope,
) -> Result<RepositoryUpdate> {
    refresh_repository_with_status(root, kind, scope, None)
}

pub(crate) fn refresh_repository_with_status(
    root: &Path,
    kind: RepositoryKind,
    scope: RefreshScope,
    worktree_status: Option<WorktreeStatus>,
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
            .then(|| thread_scope.spawn(|| load_worktree(root, worktree_status)));
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

fn load_worktree(root: &Path, status: Option<WorktreeStatus>) -> Result<WorktreeData> {
    let WorktreeStatus {
        mut changes,
        signature,
        sync,
    } = status.map_or_else(|| worktree_status(root), Ok)?;
    populate_diff_stats(root, &mut changes)?;
    Ok(WorktreeData {
        fingerprint: fingerprint(&changes),
        counts: change_counts(&changes),
        sync,
        signature,
        changes,
    })
}

pub(crate) fn load_change_line_counts(root: &Path) -> Result<(u64, u64)> {
    let worktree = load_worktree(root, None)?;
    Ok(change_line_counts(&worktree.changes))
}

fn load_git_inventory(root: &Path) -> Result<InventoryData> {
    let (files, directories, ignored_files, truncated) = inventory::git_entries(root)?;
    Ok(InventoryData {
        fingerprint: fingerprint(&(&files, &directories, &ignored_files)),
        files,
        ignored_files,
        directories,
        truncated,
    })
}

fn load_local_inventory(root: &Path) -> Result<InventoryData> {
    let (files, directories, truncated) = inventory::local_entries(root)?;
    Ok(InventoryData {
        fingerprint: fingerprint(&(&files, &directories)),
        files,
        ignored_files: Vec::new(),
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
    Ok(RefsData {
        branches: repository_branches(root)?,
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
        ahead: 0,
        behind: 0,
        changes: Vec::new(),
        files: inventory.files,
        ignored_files: inventory.ignored_files,
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

pub(crate) fn repository_branches(root: &Path) -> Result<Vec<Branch>> {
    let output = run(
        root,
        &[
            "for-each-ref",
            "--format=%(HEAD)%00%(refname)%00%(refname:short)%00%(symref:short)%00%(upstream:short)%00%(committerdate:unix)%00",
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
        .chunks_exact(6)
        .filter_map(|fields| {
            let text = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
            let refname = text(trim_ascii(fields[1]));
            let name = text(fields[2]);
            if let Some(remote) = refname
                .strip_prefix("refs/remotes/")
                .and_then(|name| name.strip_suffix("/HEAD"))
            {
                let symref = text(fields[3]);
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
                upstream: (!fields[4].is_empty()).then(|| text(fields[4])),
                remote: refname.starts_with("refs/remotes/"),
                current: trim_ascii(fields[0]) == b"*",
                default: false,
                last_touched_at: String::from_utf8_lossy(trim_ascii(fields[5])).parse().ok(),
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

pub(crate) fn change_line_counts(changes: &[Change]) -> (u64, u64) {
    changes
        .iter()
        .fold((0, 0), |(additions, deletions), change| {
            (
                additions.saturating_add(change.additions),
                deletions.saturating_add(change.deletions),
            )
        })
}
