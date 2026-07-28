use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::Instant,
};

use anyhow::{Context, Result, bail};

use super::{base_command, clean_stderr, git_limits, run_limited};
use crate::{
    process::{self, Output},
    repo_path::RepoPath,
};

pub(super) const MAX_INVENTORY_ENTRIES: usize = 100_000;
const MAX_INVENTORY_PATH_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn git_entries(root: &Path) -> Result<(Vec<RepoPath>, Vec<RepoPath>, bool)> {
    let mut truncated = false;
    let output = inventory_output(
        root,
        &[
            "ls-files",
            "-z",
            "-t",
            "--cached",
            "--others",
            "--exclude-standard",
            "--deleted",
        ],
        &mut truncated,
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut states = HashMap::<RepoPath, (bool, bool)>::new();
    for entry in output.stdout.split(|byte| *byte == 0) {
        let Some((&tag, path)) = entry.split_first() else {
            continue;
        };
        let path = path.strip_prefix(b" ").unwrap_or(path);
        if path.is_empty() {
            continue;
        }
        let path = RepoPath::from_git_bytes(path)?;
        if states.len() >= MAX_INVENTORY_ENTRIES && !states.contains_key(&path) {
            truncated = true;
            continue;
        }
        let absent_skip_worktree = tag == b'S' && root.join(&path).symlink_metadata().is_err();
        let state = states.entry(path).or_default();
        if tag == b'R' || absent_skip_worktree {
            state.1 = true;
        } else {
            state.0 = true;
        }
    }
    let mut files: Vec<RepoPath> = states
        .into_iter()
        .filter_map(|(path, (present, deleted))| (present && !deleted).then_some(path))
        .collect();
    let (ignored_files, ignored_directories, ignored_truncated) = ignored_entries(root)?;
    truncated |= ignored_truncated;
    let remaining = MAX_INVENTORY_ENTRIES.saturating_sub(files.len());
    if ignored_files.len() > remaining {
        truncated = true;
    }
    files.extend(ignored_files.into_iter().take(remaining));
    files.sort_unstable();
    files.dedup();
    if files.len() > MAX_INVENTORY_ENTRIES {
        files.truncate(MAX_INVENTORY_ENTRIES);
        truncated = true;
    }

    let output = inventory_output(
        root,
        &[
            "ls-files",
            "-z",
            "--others",
            "--directory",
            "--empty-directory",
            "--exclude-standard",
        ],
        &mut truncated,
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut roots: Vec<_> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|path| path.strip_suffix(b"/"))
        .map(RepoPath::from_git_bytes)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| !path.is_empty())
        .take(MAX_INVENTORY_ENTRIES + 1)
        .collect();
    if roots.len() > MAX_INVENTORY_ENTRIES {
        roots.pop();
        truncated = true;
    }
    let (mut directories, expanded_truncated) = expand_directories(
        root,
        roots,
        MAX_INVENTORY_ENTRIES.saturating_sub(files.len()),
    )?;
    truncated |= expanded_truncated;
    directories.extend(ignored_directories);

    let output = inventory_output(root, &["ls-files", "-z", "--stage"], &mut truncated)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let submodules: HashSet<RepoPath> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let separator = entry.iter().position(|byte| *byte == b'\t')?;
            let (metadata, path) = entry.split_at(separator);
            let path = path.get(1..)?;
            metadata
                .starts_with(b"160000 ")
                .then(|| RepoPath::from_git_bytes(path))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| {
            root.join(path)
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_dir())
        })
        .collect();
    files.retain(|path| !submodules.contains(path));
    directories.extend(submodules);
    normalize_inventory(&mut files, &mut directories, &mut truncated);
    Ok((files, directories, truncated))
}

fn ignored_entries(root: &Path) -> Result<(Vec<RepoPath>, Vec<RepoPath>, bool)> {
    let mut truncated = false;
    let output = inventory_output(
        root,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
        ],
        &mut truncated,
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut files: Vec<_> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(RepoPath::from_git_bytes)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .take(MAX_INVENTORY_ENTRIES + 1)
        .collect();
    if files.len() > MAX_INVENTORY_ENTRIES {
        files.pop();
        truncated = true;
    }
    let output = inventory_output(
        root,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
        &mut truncated,
    )?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    let mut directories: Vec<_> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|path| path.strip_suffix(b"/"))
        .map(RepoPath::from_git_bytes)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| !path.is_empty())
        .take(MAX_INVENTORY_ENTRIES.saturating_sub(files.len()) + 1)
        .collect();
    let remaining = MAX_INVENTORY_ENTRIES.saturating_sub(files.len());
    if directories.len() > remaining {
        directories.pop();
        truncated = true;
    }
    Ok((files, directories, truncated))
}

fn expand_directories(
    root: &Path,
    mut roots: Vec<RepoPath>,
    limit: usize,
) -> Result<(Vec<RepoPath>, bool)> {
    roots.sort_unstable();
    roots.dedup();
    let mut truncated = roots.len() > limit;
    roots.truncate(limit);
    let mut directories: HashSet<RepoPath> = roots.iter().cloned().collect();
    let mut frontier = roots;
    while !frontier.is_empty() {
        if directories.len() >= limit {
            truncated = true;
            break;
        }
        let mut candidates = Vec::new();
        let mut candidate_bytes = 0_usize;
        let mut candidate_limit_reached = false;
        'frontier: for relative in std::mem::take(&mut frontier) {
            let path = root.join(&relative);
            let entries = fs::read_dir(&path).with_context(|| {
                format!("could not read inventory directory {}", path.display())
            })?;
            for entry in entries {
                let entry = entry
                    .with_context(|| format!("could not read an entry in {}", path.display()))?;
                if entry.file_name() == ".git" {
                    continue;
                }
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).with_context(|| {
                    format!("could not read inventory metadata for {}", path.display())
                })?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    continue;
                }
                if let Ok(path) = path.strip_prefix(root) {
                    let candidate = RepoPath::from(path);
                    if candidates.len() >= limit
                        || candidate_bytes.saturating_add(candidate.byte_len())
                            > MAX_INVENTORY_PATH_BYTES / 2
                    {
                        candidate_limit_reached = true;
                        break 'frontier;
                    }
                    candidate_bytes = candidate_bytes.saturating_add(candidate.byte_len());
                    candidates.push(candidate);
                }
            }
        }
        truncated |= candidate_limit_reached;
        candidates.sort_unstable();
        candidates.dedup();
        let ignored = ignored_paths(root, &candidates)?;
        frontier.clear();
        for path in candidates
            .into_iter()
            .filter(|path| !ignored.contains(path))
        {
            if directories.contains(&path) {
                continue;
            }
            if directories.len() >= limit {
                truncated = true;
                break;
            }
            directories.insert(path.clone());
            frontier.push(path);
        }
    }
    Ok((directories.into_iter().collect(), truncated))
}

fn ignored_paths(root: &Path, paths: &[RepoPath]) -> Result<HashSet<RepoPath>> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(&path.git_bytes()?);
        input.push(0);
    }
    let output = process::run_with_input(
        base_command(root).args(["check-ignore", "-z", "--stdin"]),
        input,
        git_limits(),
    )
    .context("could not read git check-ignore")?;
    if output.timed_out || output.stdout_truncated {
        bail!("git check-ignore exceeded its resource limit");
    }
    if !output.status.success() && output.status.code() != Some(1) {
        bail!("{}", clean_stderr(&output));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(RepoPath::from_git_bytes)
        .collect()
}

pub(super) fn local_entries(root: &Path) -> Result<(Vec<RepoPath>, Vec<RepoPath>, bool)> {
    let started = Instant::now();
    crate::diagnostics::event(format!("local inventory started root={}", root.display()));
    let mut files = Vec::new();
    let mut directory_paths = Vec::new();
    let mut directories = vec![root.to_owned()];
    let mut truncated = false;
    let mut path_bytes = 0_usize;
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory).with_context(|| {
            format!("could not read workspace directory {}", directory.display())
        })?;
        for entry in entries {
            let entry = entry
                .with_context(|| format!("could not read an entry in {}", directory.display()))?;
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).with_context(|| {
                format!("could not read workspace metadata for {}", path.display())
            })?;
            if metadata.is_dir() {
                if let Ok(relative) = path.strip_prefix(root) {
                    let relative = RepoPath::from(relative);
                    if files.len().saturating_add(directory_paths.len()) >= MAX_INVENTORY_ENTRIES
                        || path_bytes.saturating_add(relative.byte_len()) > MAX_INVENTORY_PATH_BYTES
                    {
                        truncated = true;
                        break;
                    }
                    path_bytes = path_bytes.saturating_add(relative.byte_len());
                    directory_paths.push(relative);
                }
                directories.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                let relative = RepoPath::from(relative);
                if files.len().saturating_add(directory_paths.len()) >= MAX_INVENTORY_ENTRIES
                    || path_bytes.saturating_add(relative.byte_len()) > MAX_INVENTORY_PATH_BYTES
                {
                    truncated = true;
                    break;
                }
                path_bytes = path_bytes.saturating_add(relative.byte_len());
                files.push(relative);
            }
        }
        if truncated {
            break;
        }
    }
    files.sort();
    files.dedup();
    directory_paths.sort();
    directory_paths.dedup();
    crate::diagnostics::event(format!(
        "local inventory finished root={} files={} directories={} elapsed_ms={}",
        root.display(),
        files.len(),
        directory_paths.len(),
        started.elapsed().as_millis()
    ));
    Ok((files, directory_paths, truncated))
}

fn normalize_inventory(
    files: &mut Vec<RepoPath>,
    directories: &mut Vec<RepoPath>,
    truncated: &mut bool,
) {
    files.sort_unstable();
    files.dedup();
    directories.sort_unstable();
    directories.dedup();
    let mut entries = 0_usize;
    let mut bytes = 0_usize;
    files.retain(|path| {
        let keep = entries < MAX_INVENTORY_ENTRIES
            && bytes.saturating_add(path.byte_len()) <= MAX_INVENTORY_PATH_BYTES;
        if keep {
            entries += 1;
            bytes = bytes.saturating_add(path.byte_len());
        } else {
            *truncated = true;
        }
        keep
    });
    directories.retain(|path| {
        let keep = entries < MAX_INVENTORY_ENTRIES
            && bytes.saturating_add(path.byte_len()) <= MAX_INVENTORY_PATH_BYTES;
        if keep {
            entries += 1;
            bytes = bytes.saturating_add(path.byte_len());
        } else {
            *truncated = true;
        }
        keep
    });
}

fn inventory_output(root: &Path, args: &[&str], truncated: &mut bool) -> Result<Output> {
    let mut output = run_limited(root, args, git_limits())?;
    if output.timed_out {
        bail!("git {} timed out", args.join(" "));
    }
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    if output.stdout_truncated {
        *truncated = true;
        if let Some(end) = output.stdout.iter().rposition(|byte| *byte == 0) {
            output.stdout.truncate(end + 1);
        } else {
            output.stdout.clear();
        }
    }
    Ok(output)
}
