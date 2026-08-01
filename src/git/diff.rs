use super::*;

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

pub(super) fn parse_commit_summaries(bytes: &[u8]) -> Result<HashMap<String, DiffSummary>> {
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
