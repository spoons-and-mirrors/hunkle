use super::*;

pub(crate) fn checkout_branch(root: &Path, branch: &str, remote: bool) -> Result<CommandOutput> {
    let output = if remote {
        run(root, &["switch", "--track", "--", branch])?
    } else {
        run(root, &["switch", "--no-guess", "--", branch])?
    };
    Ok(command_output(output))
}

pub(crate) fn checkout_pull_request(
    root: &Path,
    repository_url: &str,
    number: u64,
    expected_head: &str,
) -> Result<CommandOutput> {
    let remote = format!("{}.git", repository_url.trim_end_matches('/'));
    checkout_pull_request_from_remote(root, &remote, number, expected_head)
}

pub(super) fn checkout_pull_request_from_remote(
    root: &Path,
    remote: &str,
    number: u64,
    expected_head: &str,
) -> Result<CommandOutput> {
    let pull_ref = format!("+refs/pull/{number}/head:refs/hunkle/pull/{number}");
    let output = run(root, &["fetch", "--no-tags", "--", remote, &pull_ref])?;
    if !output.status.success() {
        return Ok(command_output(output));
    }

    let local_ref = format!("refs/hunkle/pull/{number}");
    let revision = run(root, &["rev-parse", "--verify", &local_ref])?;
    if !revision.status.success() {
        return Ok(command_output(revision));
    }
    let actual_head = String::from_utf8_lossy(&revision.stdout).trim().to_owned();
    if actual_head != expected_head {
        return Err(anyhow::anyhow!(
            "pull request changed; reopen Issues before checking it out"
        ));
    }

    let short_head = &expected_head[..expected_head.len().min(12)];
    let branch = format!("hunkle/pr-{number}-{short_head}");
    let branch_ref = format!("refs/heads/{branch}");
    let existing = run(root, &["show-ref", "--verify", "--quiet", &branch_ref])?;
    let output = if existing.status.success() {
        let revision = run(root, &["rev-parse", "--verify", &branch_ref])?;
        let branch_head = String::from_utf8_lossy(&revision.stdout).trim().to_owned();
        if !revision.status.success() || branch_head != expected_head {
            return Err(anyhow::anyhow!(
                "local pull request branch changed; rename it before checking out this revision"
            ));
        }
        run(root, &["switch", "--no-guess", "--", &branch])?
    } else {
        run(
            root,
            &["switch", "--no-guess", "--create", &branch, expected_head],
        )?
    };
    Ok(command_output(output))
}

pub(crate) fn create_branch(root: &Path, branch: &str, base: &str) -> Result<CommandOutput> {
    let output = run(root, &["switch", "--no-guess", "--create", branch, base])?;
    Ok(command_output(output))
}

pub(crate) fn delete_branch(root: &Path, branch: &str) -> Result<CommandOutput> {
    let output = run(root, &["branch", "--delete", "--", branch])?;
    Ok(command_output(output))
}

pub(crate) fn branch_name(root: &Path) -> Result<String> {
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
