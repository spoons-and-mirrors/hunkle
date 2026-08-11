use super::*;

pub(crate) fn run(root: &Path, args: &[&str]) -> Result<Output> {
    let output = run_limited(root, args, git_limits())?;
    ensure_complete(&output, &format!("git {}", args.join(" ")))?;
    Ok(output)
}

pub(crate) fn run_limited(root: &Path, args: &[&str], limits: Limits) -> Result<Output> {
    process::run(base_command(root).args(args), limits)
        .with_context(|| format!("could not run git {}", args.join(" ")))
}

pub(crate) fn run_limited_cancellable(
    root: &Path,
    args: &[&str],
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<Output> {
    process::run_cancellable(base_command(root).args(args), limits, cancelled)
        .with_context(|| format!("could not run git {}", args.join(" ")))
}

pub(crate) fn run_path_command(root: &Path, args: &[&str], paths: &[&RepoPath]) -> Result<Output> {
    let output = run_path_command_limited(root, args, paths, git_limits())?;
    ensure_complete(&output, &format!("git {}", args.join(" ")))?;
    Ok(output)
}

pub(crate) fn run_path_command_limited(
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

pub(crate) fn git_limits() -> Limits {
    Limits::new(GIT_STDOUT_LIMIT, GIT_STDERR_LIMIT, GIT_TIMEOUT)
}

pub(crate) fn ensure_complete(output: &Output, label: &str) -> Result<()> {
    if output.timed_out {
        bail!("{label} timed out");
    }
    if output.stdout_truncated {
        bail!("{label} produced more than {GIT_STDOUT_LIMIT} bytes");
    }
    Ok(())
}

pub(crate) fn base_command(root: &Path) -> Command {
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

pub(crate) fn run_ok(root: &Path, args: &[&str]) -> Result<()> {
    let output = run(root, args)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

pub(crate) fn run_path_command_ok(root: &Path, args: &[&str], paths: &[&RepoPath]) -> Result<()> {
    let output = run_path_command(root, args, paths)?;
    if !output.status.success() {
        bail!("{}", clean_stderr(&output));
    }
    Ok(())
}

pub(crate) fn clean_stderr(output: &Output) -> String {
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

pub(crate) fn command_output(output: Output) -> CommandOutput {
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

pub(crate) fn preview_text(bytes: Vec<u8>, truncated: bool) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str(&format!(
            "\n\n[Preview truncated at {DIFF_PREVIEW_LIMIT} bytes]"
        ));
    }
    text
}

pub(crate) fn command_text(bytes: Vec<u8>, truncated: bool, stream: &str) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str(&format!("\n[{stream} truncated]"));
    }
    text
}

pub(crate) fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}
