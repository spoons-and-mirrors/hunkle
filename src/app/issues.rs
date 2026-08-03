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

const ISSUE_LIMIT: usize = 1000;
const CATALOG_FRESHNESS: Duration = Duration::from_secs(60);
const COMMAND_LIMITS: Limits = Limits::new(4 * 1024 * 1024, 32 * 1024, Duration::from_secs(30));

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
    pub(crate) is_draft: bool,
    pub(crate) changed_files: Option<u64>,
    pub(crate) additions: Option<u64>,
    pub(crate) deletions: Option<u64>,
    merged_at: Option<String>,
    updated_at: String,
}

impl Issue {
    pub(crate) fn kind_label(&self) -> &'static str {
        if self.pull_request { "PULL" } else { "ISSUE" }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        if !self.pull_request {
            return if self.state.eq_ignore_ascii_case("closed") {
                "CLOSED"
            } else {
                "OPEN"
            };
        }
        if self.state.eq_ignore_ascii_case("merged") || self.merged_at.is_some() {
            "MERGED"
        } else if self.state.eq_ignore_ascii_case("closed") {
            "CLOSED"
        } else if self.is_draft {
            "DRAFT"
        } else {
            "READY"
        }
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
    update: IssueUpdate,
}

enum IssueUpdate {
    Batch(Vec<Issue>),
    Finished(Result<(), String>),
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
                    if load_issues(&request, &result_sender).is_err() {
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
        self.issues.remove(&scope);
        self.loaded_at.remove(&scope);
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
            match done.update {
                IssueUpdate::Batch(batch) => {
                    let issues = self.issues.entry(done.scope).or_default();
                    issues.extend(batch);
                    issues.sort_unstable_by(|left, right| {
                        right
                            .updated_at
                            .cmp(&left.updated_at)
                            .then_with(|| right.number.cmp(&left.number))
                    });
                    issues.dedup_by_key(|issue| issue.number);
                }
                IssueUpdate::Finished(result) => {
                    self.pending.remove(&done.scope);
                    match result {
                        Ok(()) => {
                            self.loaded_at.insert(done.scope, Instant::now());
                            self.errors.remove(&done.scope);
                        }
                        Err(error) => {
                            self.errors.insert(done.scope, error);
                        }
                    }
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
struct RawGraphIssue {
    number: u64,
    title: String,
    body: String,
    author: Option<RawAuthor>,
    labels: RawLabels,
    state: String,
    updated_at: String,
    #[serde(default)]
    is_draft: bool,
    changed_files: Option<u64>,
    additions: Option<u64>,
    deletions: Option<u64>,
    merged_at: Option<String>,
}

#[derive(Deserialize)]
struct RawAuthor {
    login: String,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

#[derive(Deserialize)]
struct RawLabels {
    nodes: Vec<RawLabel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConnection {
    nodes: Vec<RawGraphIssue>,
    page_info: RawPageInfo,
}

#[derive(Deserialize)]
struct RawRepository {
    items: RawConnection,
}

#[derive(Deserialize)]
struct RawGraphData {
    repository: Option<RawRepository>,
}

#[derive(Deserialize)]
struct RawGraphError {
    message: String,
}

#[derive(Deserialize)]
struct RawGraphResponse {
    data: Option<RawGraphData>,
    #[serde(default)]
    errors: Vec<RawGraphError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepositoryIdentity {
    name_with_owner: String,
}

fn load_issues(
    request: &IssueRequest,
    sender: &Sender<IssueCompletion>,
) -> Result<(), mpsc::SendError<IssueCompletion>> {
    let result = repository_identity(&request.root).and_then(|(owner, name)| {
        let (issues, pull_requests) = thread::scope(|threads| {
            let issues = threads.spawn(|| load_issue_pages(request, sender, &owner, &name, false));
            let pull_requests =
                threads.spawn(|| load_issue_pages(request, sender, &owner, &name, true));
            (
                issues
                    .join()
                    .map_err(|_| "GitHub issue command panicked".to_owned()),
                pull_requests
                    .join()
                    .map_err(|_| "GitHub pull request command panicked".to_owned()),
            )
        });
        issues??;
        pull_requests??;
        Ok(())
    });
    sender.send(IssueCompletion {
        generation: request.generation,
        root: request.root.clone(),
        scope: request.scope,
        update: IssueUpdate::Finished(result),
    })
}

fn repository_identity(root: &Path) -> Result<(String, String), String> {
    let mut command = Command::new("gh");
    command
        .current_dir(root)
        .env("GH_PROMPT_DISABLED", "1")
        .args(["repo", "view", "--json", "nameWithOwner"]);
    let output = run_gh(&mut command, "repository")?;
    let identity = serde_json::from_slice::<RawRepositoryIdentity>(&output)
        .map_err(|error| format!("Could not identify GitHub repository: {error}"))?;
    identity
        .name_with_owner
        .split_once('/')
        .map(|(owner, name)| (owner.to_owned(), name.to_owned()))
        .ok_or_else(|| "GitHub CLI returned an invalid repository name".to_owned())
}

fn load_issue_pages(
    request: &IssueRequest,
    sender: &Sender<IssueCompletion>,
    owner: &str,
    name: &str,
    pull_request: bool,
) -> Result<(), String> {
    let noun = if pull_request {
        "pull requests"
    } else {
        "issues"
    };
    let collection = if pull_request {
        "pullRequests"
    } else {
        "issues"
    };
    let states = match (pull_request, request.scope) {
        (true, IssueScope::Closed) => "[CLOSED,MERGED]",
        (_, IssueScope::Closed) => "CLOSED",
        _ => "OPEN",
    };
    let pull_request_fields = if pull_request {
        "isDraft changedFiles additions deletions mergedAt"
    } else {
        ""
    };
    let query = format!(
        "query($owner:String!,$name:String!,$after:String){{repository(owner:$owner,name:$name){{items:{collection}(first:100,after:$after,states:{states},orderBy:{{field:UPDATED_AT,direction:DESC}}){{nodes{{number title body author{{login}} labels(first:20){{nodes{{name}}}} state updatedAt {pull_request_fields}}}pageInfo{{hasNextPage endCursor}}}}}}}}"
    );
    let mut cursor = None;
    let mut loaded = 0;
    loop {
        let mut command = Command::new("gh");
        command
            .current_dir(&request.root)
            .env("GH_PROMPT_DISABLED", "1")
            .args(["api", "graphql", "-f"])
            .arg(format!("query={query}"))
            .args(["-F", &format!("owner={owner}")])
            .args(["-F", &format!("name={name}")]);
        if let Some(cursor) = cursor.as_deref() {
            command.args(["-F", &format!("after={cursor}")]);
        }
        let output = run_gh(&mut command, noun)?;
        let (batch, page_info) = parse_graph_page(&output, pull_request, noun)?;
        loaded += batch.len();
        if !batch.is_empty() {
            sender
                .send(IssueCompletion {
                    generation: request.generation,
                    root: request.root.clone(),
                    scope: request.scope,
                    update: IssueUpdate::Batch(batch),
                })
                .map_err(|_| "GitHub issue worker stopped".to_owned())?;
        }
        if !page_info.has_next_page || loaded >= ISSUE_LIMIT {
            return Ok(());
        }
        cursor = page_info.end_cursor;
        if cursor.is_none() {
            return Err(format!("GitHub {noun} pagination cursor was missing"));
        }
    }
}

fn parse_graph_page(
    output: &[u8],
    pull_request: bool,
    noun: &str,
) -> Result<(Vec<Issue>, RawPageInfo), String> {
    let response = serde_json::from_slice::<RawGraphResponse>(output)
        .map_err(|error| format!("Could not read GitHub {noun}: {error}"))?;
    if let Some(error) = response.errors.first() {
        return Err(error.message.clone());
    }
    let connection = response
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| "GitHub repository was not found".to_owned())?
        .items;
    let issues = connection
        .nodes
        .into_iter()
        .map(|issue| Issue {
            number: issue.number,
            title: issue.title,
            body: issue.body,
            author: issue.author.map(|author| author.login),
            labels: issue
                .labels
                .nodes
                .into_iter()
                .map(|label| label.name)
                .collect(),
            pull_request,
            state: issue.state,
            is_draft: issue.is_draft,
            changed_files: issue.changed_files,
            additions: issue.additions,
            deletions: issue.deletions,
            merged_at: issue.merged_at,
            updated_at: issue.updated_at,
        })
        .collect();
    Ok((issues, connection.page_info))
}

fn run_gh(command: &mut Command, noun: &str) -> Result<Vec<u8>, String> {
    let output = process::run(command, COMMAND_LIMITS).map_err(|error| {
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
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nullable_authors_and_labels() {
        let raw = br#"{"data":{"repository":{"items":{"nodes":[{"number":7,"title":"Document it","body":"Body","author":null,"labels":{"nodes":[{"name":"docs"}]},"state":"CLOSED","updatedAt":"2026-01-01T00:00:00Z"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#;
        let (parsed, page_info) = parse_graph_page(raw, false, "issues").unwrap();

        assert_eq!(parsed[0].number, 7);
        assert!(parsed[0].author.is_none());
        assert_eq!(parsed[0].labels[0], "docs");
        assert!(!page_info.has_next_page);
    }

    #[test]
    fn closed_scope_follows_open_scope() {
        assert_eq!(IssueScope::Open.toggle(), IssueScope::Closed);
        assert_eq!(IssueScope::Closed.toggle(), IssueScope::Open);
    }

    #[test]
    fn pull_request_status_distinguishes_lifecycle_states() {
        let issue = |state: &str, is_draft, merged_at: Option<&str>| Issue {
            number: 1,
            title: String::new(),
            body: String::new(),
            author: None,
            labels: Vec::new(),
            pull_request: true,
            state: state.to_owned(),
            is_draft,
            changed_files: None,
            additions: None,
            deletions: None,
            merged_at: merged_at.map(str::to_owned),
            updated_at: String::new(),
        };

        assert_eq!(issue("OPEN", false, None).status_label(), "READY");
        assert_eq!(issue("OPEN", true, None).status_label(), "DRAFT");
        assert_eq!(issue("CLOSED", false, None).status_label(), "CLOSED");
        assert_eq!(
            issue("CLOSED", false, Some("2026-01-01T00:00:00Z")).status_label(),
            "MERGED"
        );
    }
}
