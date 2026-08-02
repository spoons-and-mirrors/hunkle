use super::*;

pub(crate) fn clone_repository(destination: &Path, url: &str) -> Result<PathBuf> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("clone destination must have a parent directory"))?;
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("could not open clone directory {}", parent.display()))?;
    let name = destination
        .file_name()
        .ok_or_else(|| anyhow!("clone destination must name a repository directory"))?;
    let destination = parent.join(name);
    let mut command = base_command(&parent);
    command.arg("clone").arg("--").arg(url).arg(&destination);
    let output = process::run(
        &mut command,
        Limits::new(GIT_STDOUT_LIMIT, GIT_STDERR_LIMIT, GIT_NETWORK_TIMEOUT),
    )
    .context("could not run git clone")?;
    ensure_complete(&output, "git clone")?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    fs::canonicalize(&destination)
        .with_context(|| format!("could not open cloned repository {}", destination.display()))
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
