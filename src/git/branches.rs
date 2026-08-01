use super::*;

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

pub(crate) fn create_branch(root: &Path, branch: &str, base: &str) -> Result<CommandOutput> {
    let output = run(root, &["switch", "--no-guess", "--create", branch, base])?;
    Ok(command_output(output))
}

pub(super) fn branch_name(root: &Path) -> Result<String> {
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
