use std::{
    ffi::OsString,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    diagnostics,
    filesystem::same_path,
    process::{self, Limits},
};
use serde_json::{Map, Value, json};

use super::commit_message::{
    concise_error, delete_opencode_session, opencode_working_directory, parse_opencode_events,
};

const MAX_OPENCODE_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 256 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) struct MagicCommitCompletion {
    pub(crate) root: PathBuf,
    pub(crate) result: Result<(), String>,
}

pub(crate) struct MagicCommitRunner {
    available: bool,
    running: bool,
    running_root: Option<PathBuf>,
    started_at: Option<Instant>,
    cancel: Option<Arc<AtomicBool>>,
    spinner_frame: usize,
    next_spinner: Instant,
    sender: Sender<MagicCommitCompletion>,
    receiver: Receiver<MagicCommitCompletion>,
}

impl MagicCommitRunner {
    pub(crate) fn detect() -> Self {
        #[cfg(test)]
        let available = false;
        #[cfg(not(test))]
        let available = command_available("opencode");

        diagnostics::event(format!("magic commit available={available}"));
        Self::new(available)
    }

    fn new(available: bool) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            available,
            running: false,
            running_root: None,
            started_at: None,
            cancel: None,
            spinner_frame: 0,
            next_spinner: Instant::now(),
            sender,
            receiver,
        }
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test() -> Self {
        Self::new(true)
    }

    pub(crate) fn is_available(&self) -> bool {
        self.available
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    pub(crate) fn is_running_for(&self, root: &Path) -> bool {
        self.running_root
            .as_deref()
            .is_some_and(|running_root| same_path(running_root, root))
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.started_at
            .map_or(Duration::ZERO, |started| started.elapsed())
    }

    pub(crate) fn is_cancelling(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Acquire))
    }

    pub(crate) fn cancel(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| !cancel.swap(true, Ordering::AcqRel))
    }

    pub(crate) fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    pub(crate) fn poll_spinner(&mut self, now: Instant) -> bool {
        if !self.running {
            self.spinner_frame = 0;
            self.next_spinner = now;
            return false;
        }
        if now < self.next_spinner {
            return false;
        }
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
        self.next_spinner = now + SPINNER_INTERVAL;
        true
    }

    pub(crate) fn start(
        &mut self,
        root: PathBuf,
        model: String,
        variant: Option<String>,
    ) -> Result<(), String> {
        if !self.available {
            return Err("OpenCode is not installed".to_owned());
        }
        if self.running {
            return Err("Magic Commit is already running".to_owned());
        }

        let sender = self.sender.clone();
        let worker_root = root.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        thread::Builder::new()
            .name("hunkle-magic-commit".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_magic_commit(&worker_root, &model, variant.as_deref(), worker_cancel)
                }))
                .unwrap_or_else(|_| Err("Magic Commit panicked".to_owned()));
                let _ = sender.send(MagicCommitCompletion {
                    root: worker_root,
                    result,
                });
            })
            .map_err(|error| format!("Could not start Magic Commit: {error}"))?;

        self.running = true;
        self.running_root = Some(root.clone());
        self.started_at = Some(Instant::now());
        self.cancel = Some(cancel);
        self.spinner_frame = 0;
        self.next_spinner = Instant::now() + SPINNER_INTERVAL;
        diagnostics::event(format!("magic commit requested root={}", root.display()));
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Option<MagicCommitCompletion> {
        let completion = self.receiver.try_recv().ok()?;
        self.running = false;
        self.running_root = None;
        self.started_at = None;
        self.cancel = None;
        Some(completion)
    }
}

fn run_magic_commit(
    root: &Path,
    model: &str,
    variant: Option<&str>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let started = Instant::now();
    let working_directory = opencode_working_directory()?;
    let prompt = magic_commit_prompt(root);
    let permissions = opencode_permissions(root);
    let output = process::run_cancellable(
        Command::new("opencode")
            .args(opencode_args(model, variant, &prompt))
            .env("OPENCODE_PERMISSION", permissions)
            .current_dir(&working_directory),
        Limits::new(MAX_OPENCODE_OUTPUT_BYTES, MAX_ERROR_BYTES, COMMAND_TIMEOUT),
        cancel,
    )
    .map_err(|error| format!("Could not run OpenCode for Magic Commit: {error}"))?;
    let events = parse_opencode_events(&output.stdout);
    let cleanup = events
        .session_id
        .as_deref()
        .map(|session_id| delete_opencode_session(&working_directory, session_id));
    diagnostics::event(format!(
        "magic commit finished root={} working_directory={} session_id={} elapsed_ms={} success={} timed_out={} cancelled={} output_truncated={} cleaned_up={}",
        root.display(),
        working_directory.display(),
        events.session_id.as_deref().unwrap_or("unknown"),
        started.elapsed().as_millis(),
        output.status.success(),
        output.timed_out,
        output.cancelled,
        output.stdout_truncated,
        cleanup.as_ref().is_some_and(Result::is_ok)
    ));
    if let Some(Err(error)) = cleanup {
        diagnostics::event(error);
    }
    if output.timed_out {
        return Err("Magic Commit timed out and may have created partial commits".to_owned());
    }
    if output.cancelled {
        return Err("Magic Commit cancelled; it may have created partial commits".to_owned());
    }
    if !output.status.success() {
        return Err(format!(
            "Magic Commit failed and may have created partial commits: {}",
            concise_error(&output.stderr)
        ));
    }
    Ok(())
}

fn opencode_args(model: &str, variant: Option<&str>, prompt: &str) -> Vec<OsString> {
    let mut args = vec![
        "run".into(),
        "--pure".into(),
        "--model".into(),
        model.into(),
    ];
    if let Some(variant) = variant {
        args.extend([OsString::from("--variant"), variant.into()]);
    }
    args.extend([
        OsString::from("--format"),
        "json".into(),
        "--title".into(),
        "Hunkle magic commit".into(),
        prompt.into(),
    ]);
    args
}

fn magic_commit_prompt(root: &Path) -> String {
    let git = git_command_prefix(root);
    format!(
        r#"Commit all current changes in the target Git worktree as a small sequence of logical, coherent commits.

The target worktree is not your OpenCode working directory. Every command must begin with this exact prefix: `{git}`. Commands without that exact prefix are denied.

You may only inspect `{git} status` and `{git} diff`, modify its index with `{git} add -N -- <path>` or `{git} apply --cached`, and create commits with `{git} commit -m <message>`. Never edit, create, delete, format, or generate a worktree file. Never use another repository. Never discard, reset, clean, stash, checkout, switch, merge, rebase, revert, fetch, pull, push, run tests, search source files, or run project tooling.

Inspect both staged and unstaged diffs before deciding the boundaries. Include untracked files with intent-to-add so their content appears in the diff. Stage only complete patches for one logical commit at a time with index-only patches. If an already-staged hunk belongs later, unstage only that patch with `{git} apply --cached --reverse`. Do not broadly stage files or the whole worktree.

After each commit, inspect status and diffs again. Continue until every original change is committed, unless the restricted Git-only operations cannot safely do so. Preserve every existing change exactly. Use concise, accurate commit messages. Do not ask questions; perform the commits directly."#
    )
}

fn opencode_permissions(root: &Path) -> String {
    let git = git_command_prefix(root);
    let mut bash = Map::new();
    bash.insert("*".to_owned(), Value::String("deny".to_owned()));
    for suffix in [
        "status*",
        "diff*",
        "add -N -- *",
        "apply --cached*",
        "commit -m *",
    ] {
        bash.insert(format!("{git} {suffix}"), Value::String("allow".to_owned()));
    }
    let mut external_directory = Map::new();
    external_directory.insert("*".to_owned(), Value::String("deny".to_owned()));
    external_directory.insert(
        format!("{}/*", root.display()),
        Value::String("allow".to_owned()),
    );
    json!({
        "edit": "deny",
        "read": "deny",
        "glob": "deny",
        "grep": "deny",
        "list": "deny",
        "task": "deny",
        "skill": "deny",
        "webfetch": "deny",
        "question": "deny",
        "external_directory": external_directory,
        "bash": bash,
    })
    .to_string()
}

fn git_command_prefix(root: &Path) -> String {
    format!("git -C {}", shell_quote(&root.to_string_lossy()))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(not(test))]
fn command_available(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        #[cfg(windows)]
        let candidates = [
            directory.join(format!("{name}.exe")),
            directory.join(format!("{name}.cmd")),
            directory.join(format!("{name}.bat")),
            directory.join(name),
        ];
        #[cfg(not(windows))]
        let candidates = [directory.join(name)];
        candidates.into_iter().any(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricts_opencode_to_index_only_git_operations() {
        let root = Path::new("/tmp/a linked worktree");
        let permissions: serde_json::Value =
            serde_json::from_str(&opencode_permissions(root)).unwrap();
        let git = git_command_prefix(root);

        assert_eq!(permissions["edit"], "deny");
        assert_eq!(permissions["read"], "deny");
        assert_eq!(permissions["bash"]["*"], "deny");
        assert_eq!(
            permissions["bash"][format!("{git} apply --cached*")],
            "allow"
        );
        assert_eq!(permissions["bash"][format!("{git} commit -m *")], "allow");
        assert!(permissions["bash"].get("git add *").is_none());
        assert_eq!(permissions["external_directory"]["*"], "deny");
    }

    #[test]
    fn builds_a_one_shot_opencode_command() {
        let prompt = magic_commit_prompt(Path::new("/tmp/worktree"));
        let args = opencode_args("provider/model", Some("low"), &prompt)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            &args[..10],
            [
                "run",
                "--pure",
                "--model",
                "provider/model",
                "--variant",
                "low",
                "--format",
                "json",
                "--title",
                "Hunkle magic commit",
            ]
        );
        assert!(args[10].contains("git -C '/tmp/worktree'"));
    }

    #[test]
    fn shell_quotes_the_target_worktree() {
        assert_eq!(
            git_command_prefix(Path::new("/tmp/it's linked")),
            "git -C '/tmp/it'\\''s linked'"
        );
    }

    #[test]
    fn scopes_running_state_to_the_target_worktree() {
        let mut runner = MagicCommitRunner::new(true);
        runner.running = true;
        runner.running_root = Some(PathBuf::from("/tmp/first"));

        assert!(runner.is_running_for(Path::new("/tmp/first")));
        assert!(!runner.is_running_for(Path::new("/tmp/second")));
    }
}
