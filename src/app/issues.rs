use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::process::{self, Limits};

const ISSUE_LIMIT: &str = "100";
const CATALOG_FRESHNESS: Duration = Duration::from_secs(60);
const COMMAND_LIMITS: Limits = Limits::new(8 * 1024 * 1024, 32 * 1024, Duration::from_secs(30));

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum IssueScope {
    #[default]
    Open,
    Closed,
}

impl IssueScope {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Open => Self::Closed,
            Self::Closed => Self::Open,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Closed => "CLOSED",
        }
    }

    fn gh_state(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Issue {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) author: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) pull_request: bool,
    pub(crate) state: String,
    updated_at: String,
}

impl Issue {
    pub(crate) fn kind_label(&self) -> &'static str {
        if self.pull_request { "PULL" } else { "ISSUE" }
    }

    pub(crate) fn detail(&self) -> String {
        let state = if self.state.eq_ignore_ascii_case("merged") {
            "merged"
        } else if self.state.eq_ignore_ascii_case("closed") {
            "closed"
        } else {
            "open"
        };
        let kind = if self.pull_request { "PR" } else { "issue" };
        self.author.as_ref().map_or_else(
            || format!("{state} {kind}"),
            |author| format!("{state} {kind} · @{author}"),
        )
    }
}

struct IssueRequest {
    generation: u64,
    root: PathBuf,
    scope: IssueScope,
}

struct IssueCompletion {
    generation: u64,
    root: PathBuf,
    scope: IssueScope,
    result: Result<Vec<Issue>, String>,
}

pub(crate) struct IssueCatalog {
    root: Option<PathBuf>,
    generation: u64,
    scope: IssueScope,
    issues: HashMap<IssueScope, Vec<Issue>>,
    loaded_at: HashMap<IssueScope, Instant>,
    pending: HashSet<IssueScope>,
    errors: HashMap<IssueScope, String>,
    request_sender: Option<Sender<IssueRequest>>,
    receiver: Receiver<IssueCompletion>,
    worker: Option<JoinHandle<()>>,
}

impl Default for IssueCatalog {
    fn default() -> Self {
        let (request_sender, request_receiver) = mpsc::channel::<IssueRequest>();
        let (result_sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("hunkle-github-issues".to_owned())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let result = load_issues(&request.root, request.scope);
                    if result_sender
                        .send(IssueCompletion {
                            generation: request.generation,
                            root: request.root,
                            scope: request.scope,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("could not start GitHub issue worker");
        Self {
            root: None,
            generation: 0,
            scope: IssueScope::Open,
            issues: HashMap::new(),
            loaded_at: HashMap::new(),
            pending: HashSet::new(),
            errors: HashMap::new(),
            request_sender: Some(request_sender),
            receiver,
            worker: Some(worker),
        }
    }
}

impl IssueCatalog {
    pub(crate) fn scope(&self) -> IssueScope {
        self.scope
    }

    pub(crate) fn toggle_scope(&mut self) -> IssueScope {
        self.scope = self.scope.toggle();
        self.scope
    }

    pub(crate) fn request(&mut self, root: &Path) {
        self.activate(root);
        let scope = self.scope;
        let fresh = self.loaded_at.get(&scope).is_some_and(|loaded| {
            Instant::now().saturating_duration_since(*loaded) < CATALOG_FRESHNESS
        });
        if fresh || !self.pending.insert(scope) {
            return;
        }
        self.errors.remove(&scope);
        let request = IssueRequest {
            generation: self.generation,
            root: root.to_owned(),
            scope,
        };
        if self
            .request_sender
            .as_ref()
            .is_none_or(|sender| sender.send(request).is_err())
        {
            self.pending.remove(&scope);
            self.errors
                .insert(scope, "GitHub issue worker stopped".to_owned());
        }
    }

    pub(crate) fn issues(&self) -> Option<&[Issue]> {
        self.issues.get(&self.scope).map(Vec::as_slice)
    }

    pub(crate) fn issue(&self, number: u64) -> Option<&Issue> {
        self.issues()?.iter().find(|issue| issue.number == number)
    }

    pub(crate) fn loading(&self) -> bool {
        self.pending.contains(&self.scope)
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.errors.get(&self.scope).map(String::as_str)
    }

    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.receiver.try_recv() {
            if done.generation != self.generation || self.root.as_deref() != Some(&done.root) {
                continue;
            }
            self.pending.remove(&done.scope);
            match done.result {
                Ok(issues) => {
                    self.issues.insert(done.scope, issues);
                    self.loaded_at.insert(done.scope, Instant::now());
                    self.errors.remove(&done.scope);
                }
                Err(error) => {
                    self.errors.insert(done.scope, error);
                }
            }
            changed = true;
        }
        changed
    }

    pub(crate) fn shutdown(&mut self) {
        self.request_sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn activate(&mut self, root: &Path) {
        if self.root.as_deref() == Some(root) {
            return;
        }
        self.root = Some(root.to_owned());
        self.generation = self.generation.wrapping_add(1);
        self.issues.clear();
        self.loaded_at.clear();
        self.pending.clear();
        self.errors.clear();
    }
}

impl Drop for IssueCatalog {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssue {
    number: u64,
    title: String,
    body: String,
    author: Option<RawAuthor>,
    #[serde(default)]
    labels: Vec<RawLabel>,
    state: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct RawAuthor {
    login: String,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

fn load_issues(root: &Path, scope: IssueScope) -> Result<Vec<Issue>, String> {
    let (issues, pull_requests) = thread::scope(|threads| {
        let issues = threads.spawn(|| load_issue_kind(root, scope, false));
        let pull_requests = threads.spawn(|| load_issue_kind(root, scope, true));
        (
            issues
                .join()
                .map_err(|_| "GitHub issue command panicked".to_owned()),
            pull_requests
                .join()
                .map_err(|_| "GitHub pull request command panicked".to_owned()),
        )
    });
    let mut issues = issues??;
    issues.extend(pull_requests??);
    issues.sort_unstable_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.number.cmp(&left.number))
    });
    Ok(issues)
}

fn load_issue_kind(
    root: &Path,
    scope: IssueScope,
    pull_request: bool,
) -> Result<Vec<Issue>, String> {
    let noun = if pull_request {
        "pull requests"
    } else {
        "issues"
    };
    let mut command = Command::new("gh");
    command
        .current_dir(root)
        .env("GH_PROMPT_DISABLED", "1")
        .arg(if pull_request { "pr" } else { "issue" })
        .arg("list")
        .args(["--state", scope.gh_state()])
        .args(["--limit", ISSUE_LIMIT])
        .args(["--json", "number,title,body,author,labels,state,updatedAt"]);
    let output = process::run(&mut command, COMMAND_LIMITS).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "GitHub CLI (`gh`) is required to browse issues".to_owned()
        } else {
            format!("Could not start GitHub CLI: {error}")
        }
    })?;
    if output.timed_out {
        return Err(format!("Loading GitHub {noun} timed out"));
    }
    if output.stdout_truncated {
        return Err(format!("GitHub {noun} response was too large"));
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr)
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("GitHub CLI returned an error")
            .trim()
            .to_owned();
        return Err(detail);
    }
    let raw = serde_json::from_slice::<Vec<RawIssue>>(&output.stdout)
        .map_err(|error| format!("Could not read GitHub {noun}: {error}"))?;
    Ok(raw
        .into_iter()
        .map(|issue| Issue {
            number: issue.number,
            title: issue.title,
            body: issue.body,
            author: issue.author.map(|author| author.login),
            labels: issue.labels.into_iter().map(|label| label.name).collect(),
            pull_request,
            state: issue.state,
            updated_at: issue.updated_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nullable_authors_and_labels() {
        let raw = br#"[{"number":7,"title":"Document it","body":"Body","author":null,"labels":[{"name":"docs"}],"state":"CLOSED","updatedAt":"2026-01-01T00:00:00Z"}]"#;
        let parsed: Vec<RawIssue> = serde_json::from_slice(raw).unwrap();

        assert_eq!(parsed[0].number, 7);
        assert!(parsed[0].author.is_none());
        assert_eq!(parsed[0].labels[0].name, "docs");
    }

    #[test]
    fn closed_scope_follows_open_scope() {
        assert_eq!(IssueScope::Open.toggle(), IssueScope::Closed);
        assert_eq!(IssueScope::Closed.toggle(), IssueScope::Open);
    }
}
