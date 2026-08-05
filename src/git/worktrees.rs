use super::*;

const WORKTREE_REPOSITORY_ID_BITS: u32 = 20;

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

pub(crate) fn create_worktree(repository: &Path, branch: &str, base: &str) -> Result<PathBuf> {
    create_worktree_in(repository, branch, base, &worktree_storage_root()?)
}

pub(crate) fn create_worktree_for_branch(
    repository: &Path,
    branch: &str,
    remote: bool,
) -> Result<PathBuf> {
    create_worktree_for_branch_in(repository, branch, remote, &worktree_storage_root()?)
}

pub(super) fn create_worktree_for_branch_in(
    repository: &Path,
    branch: &str,
    remote: bool,
    storage_root: &Path,
) -> Result<PathBuf> {
    let local_branch = if remote {
        branch
            .split_once('/')
            .map(|(_, branch)| branch)
            .context("the remote branch has no branch name")?
    } else {
        branch
    };
    let path = managed_worktree_path(repository, local_branch, storage_root)?;
    let local_revision = format!("refs/heads/{local_branch}");
    let local_exists = if remote {
        let output = run(
            repository,
            &["show-ref", "--verify", "--quiet", &local_revision],
        )?;
        output.status.success()
    } else {
        true
    };
    if remote && local_exists {
        let output = run(
            repository,
            &[
                "for-each-ref",
                "--format=%(upstream:short)",
                &local_revision,
            ],
        )?;
        ensure_complete(&output, "git for-each-ref")?;
        if !output.status.success() {
            bail!("{}", clean_stderr(&output));
        }
        let upstream = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if upstream != branch {
            bail!(
                "local branch {local_branch} already exists and does not track {branch}; select {local_branch} instead"
            );
        }
    }
    let mut command = base_command(repository);
    command.args(["worktree", "add"]);
    if remote && !local_exists {
        command.args(["--track", "-b", local_branch]);
    }
    command.arg("--").arg(&path).arg(if local_exists {
        local_branch.to_owned()
    } else {
        format!("refs/remotes/{branch}")
    });
    let output = process::run(&mut command, git_limits())
        .with_context(|| format!("could not create worktree {}", path.display()))?;
    ensure_complete(&output, "git worktree add")?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    fs::canonicalize(&path)
        .with_context(|| format!("could not resolve created worktree {}", path.display()))
}

fn worktree_storage_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("hunkle").join("worktrees"));
    }
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .filter(|path| !path.is_empty())
    {
        return Ok(PathBuf::from(path).join("hunkle").join("worktrees"));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("the home directory is unavailable")?;
    #[cfg(windows)]
    return Ok(PathBuf::from(home)
        .join("AppData")
        .join("Local")
        .join("hunkle")
        .join("worktrees"));
    #[cfg(not(windows))]
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("hunkle")
        .join("worktrees"))
}

pub(super) fn create_worktree_in(
    repository: &Path,
    branch: &str,
    base: &str,
    storage_root: &Path,
) -> Result<PathBuf> {
    let path = managed_worktree_path(repository, branch, storage_root)?;
    let path_parent = path
        .parent()
        .context("the worktree path has no parent directory")?;
    fs::create_dir_all(path_parent).with_context(|| {
        format!(
            "could not create worktree directory {}",
            path_parent.display()
        )
    })?;

    let output = process::run(
        base_command(repository)
            .args(["worktree", "add", "-b"])
            .arg(branch)
            .arg("--")
            .arg(&path)
            .arg(base),
        git_limits(),
    )
    .with_context(|| format!("could not create worktree {}", path.display()))?;
    ensure_complete(&output, "git worktree add")?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    fs::canonicalize(&path)
        .with_context(|| format!("could not resolve created worktree {}", path.display()))
}

fn managed_worktree_path(repository: &Path, branch: &str, storage_root: &Path) -> Result<PathBuf> {
    let main = list_worktrees(repository)?
        .into_iter()
        .find(|worktree| worktree.is_main && !worktree.is_bare)
        .context("Git did not identify the main worktree")?;
    let repository_name = main
        .path
        .file_name()
        .context("the main worktree has no directory name")?;
    let common_dir = common_git_dir(&main.path)?;
    let mut repository_directory = repository_name.to_os_string();
    let repository_id = path_hash(&common_dir) >> (u64::BITS - WORKTREE_REPOSITORY_ID_BITS);
    repository_directory.push(format!("-{:05x}", repository_id));
    let path = storage_root
        .join(repository_directory)
        .join(branch.replace('/', "-"));
    let path_parent = path
        .parent()
        .context("the worktree path has no parent directory")?;
    fs::create_dir_all(path_parent).with_context(|| {
        format!(
            "could not create worktree directory {}",
            path_parent.display()
        )
    })?;
    Ok(path)
}

fn path_hash(path: &Path) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        for byte in path.as_os_str().as_bytes() {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            for byte in unit.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    hash
}

pub(crate) fn remove_worktree(repository: &Path, worktree: &Path) -> Result<()> {
    let output = process::run(
        base_command(repository)
            .args(["worktree", "remove", "--"])
            .arg(worktree),
        git_limits(),
    )
    .with_context(|| format!("could not remove worktree {}", worktree.display()))?;
    ensure_complete(&output, "git worktree remove")?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

pub(super) fn parse_worktrees(bytes: &[u8]) -> Result<Vec<LinkedWorktree>> {
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
