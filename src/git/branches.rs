use super::*;

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
