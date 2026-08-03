use super::*;

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
    let mut inventory = DefaultHasher::new();
    let mut path_states = Vec::new();
    for change in changes {
        match change.code {
            '?' | 'A' => {
                path_states.push((&change.path, true));
            }
            'D' => {
                path_states.push((&change.path, false));
            }
            'R' => {
                path_states.push((&change.path, true));
                if let Some(original) = &change.original_path {
                    path_states.push((original, false));
                }
            }
            'C' => {
                path_states.push((&change.path, true));
            }
            _ => {}
        }
        if change
            .path
            .file_name()
            .is_some_and(|name| name == ".gitignore")
        {
            change.path.hash(&mut inventory);
            let path = root.join(&change.path);
            if let Ok(metadata) = fs::symlink_metadata(path) {
                metadata.len().hash(&mut inventory);
                metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .hash(&mut inventory);
            }
        }
    }
    path_states.sort_unstable();
    path_states.dedup();
    path_states.hash(&mut inventory);
    WorktreeSignature {
        state: state.finish(),
        inventory: inventory.finish(),
        branch: branch.finish(),
    }
}

pub(super) fn status(root: &Path) -> Result<(Vec<Change>, WorktreeSignature, BranchSync)> {
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
    let sync = parse_branch_sync(&output.stdout);
    Ok((changes, signature, sync))
}

pub(super) fn parse_branch_sync(bytes: &[u8]) -> BranchSync {
    let Some(header) = bytes
        .split(|byte| *byte == 0)
        .find(|field| field.starts_with(b"## "))
    else {
        return BranchSync::default();
    };
    let header = String::from_utf8_lossy(header);
    let Some(summary) = header.rsplit_once(" [").map(|(_, summary)| summary) else {
        return BranchSync::default();
    };
    let mut sync = BranchSync::default();
    for state in summary.trim_end_matches(']').split(", ") {
        if let Some(ahead) = state.strip_prefix("ahead ") {
            sync.ahead = ahead.parse().unwrap_or_default();
        } else if let Some(behind) = state.strip_prefix("behind ") {
            sync.behind = behind.parse().unwrap_or_default();
        }
    }
    sync
}

pub(super) fn parse_status(bytes: &[u8]) -> Result<Vec<Change>> {
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

pub(super) fn populate_diff_stats(root: &Path, changes: &mut [Change]) -> Result<()> {
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

pub(super) fn count_file_lines(path: &Path, byte_limit: u64) -> Result<(u64, u64)> {
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

pub(super) fn open_regular_file(path: &Path) -> Result<(fs::File, fs::Metadata)> {
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
