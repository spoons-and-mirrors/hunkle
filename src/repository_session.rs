use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::{
    diagnostics,
    filesystem::{self, FileOperation},
    formatter::{self, FormatCommand},
    git::{
        self, Change, CommandOutput, RefreshScope, RepositoryData, RepositoryKind, RepositoryUpdate,
    },
    repo_path::RepoPath,
    tree::PreparedFileTree,
};

const MIN_STATUS_INTERVAL: Duration = Duration::from_millis(800);
const MAX_STATUS_INTERVAL: Duration = Duration::from_secs(10);

pub(crate) enum WorkerOutcome {
    Commit(Result<CommandOutput, String>),
    Fetch(Result<CommandOutput, String>),
    Command(CommandCompletion),
    Mutation(Result<(), String>),
    FileOperation(FileOperationCompletion),
    DiscardUnstaged(DiscardUnstagedCompletion),
    Format(FormatCompletion),
    BranchDelete(BranchDeleteCompletion),
}

pub(crate) struct WorkerCompletion {
    pub(crate) outcome: WorkerOutcome,
    invalidation: Option<RefreshScope>,
}

impl WorkerCompletion {
    fn new(outcome: WorkerOutcome) -> Self {
        let invalidation = match &outcome {
            WorkerOutcome::Commit(Ok(output)) if output.success => {
                Some(RefreshScope::WORKTREE.union(RefreshScope::HISTORY_AND_REFS))
            }
            WorkerOutcome::Fetch(Ok(output)) if output.success => {
                Some(RefreshScope::HISTORY_AND_REFS)
            }
            WorkerOutcome::Command(done)
                if done.result.as_ref().is_ok_and(|output| output.success) =>
            {
                Some(RefreshScope::ALL)
            }
            WorkerOutcome::Mutation(Ok(())) => Some(RefreshScope::WORKTREE),
            WorkerOutcome::FileOperation(done) if done.result.is_ok() => {
                Some(RefreshScope::WORKTREE_AND_INVENTORY)
            }
            WorkerOutcome::DiscardUnstaged(_) => Some(RefreshScope::WORKTREE_AND_INVENTORY),
            WorkerOutcome::Format(done)
                if done.result.as_ref().is_ok_and(|output| output.success) =>
            {
                Some(RefreshScope::WORKTREE)
            }
            WorkerOutcome::BranchDelete(_) => Some(RefreshScope::HISTORY_AND_REFS),
            WorkerOutcome::Commit(_)
            | WorkerOutcome::Fetch(_)
            | WorkerOutcome::Command(_)
            | WorkerOutcome::Mutation(_)
            | WorkerOutcome::FileOperation(_)
            | WorkerOutcome::Format(_) => None,
        };
        Self {
            outcome,
            invalidation,
        }
    }

    pub(crate) fn invalidation(&self) -> Option<RefreshScope> {
        self.invalidation
    }
}

pub(crate) struct BranchDeleteCompletion {
    pub(crate) branch: String,
    pub(crate) remote: Option<(String, String)>,
    pub(crate) force: bool,
    pub(crate) result: Result<(), String>,
}

pub(crate) enum Mutation {
    Stage(Change),
    Unstage(Change),
    StageAll,
    UnstageAll,
    StageHunk { patch: String, index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadKind {
    Open,
    Reload,
}

pub(crate) struct LoadCompletion {
    pub(crate) kind: LoadKind,
    pub(crate) result: Result<(), String>,
    pub(crate) prepared_file_tree: Option<PreparedFileTree>,
}

pub(crate) struct CommandCompletion {
    pub(crate) label: String,
    pub(crate) result: Result<CommandOutput, String>,
}

pub(crate) struct FileOperationCompletion {
    pub(crate) result: Result<Option<RepoPath>, String>,
    pub(crate) message: String,
}

pub(crate) struct DiscardUnstagedCompletion {
    pub(crate) path: RepoPath,
    pub(crate) result: Result<(), String>,
}

pub(crate) struct FormatCompletion {
    pub(crate) path: RepoPath,
    pub(crate) formatter: &'static str,
    pub(crate) result: Result<CommandOutput, String>,
}

#[derive(Debug)]
struct WorkerResult {
    kind: WorkerKind,
    root: PathBuf,
    result: Result<CommandOutput, String>,
}

#[derive(Debug)]
struct StatusResult {
    root: PathBuf,
    repository_generation: u64,
    baseline: Option<git::WorktreeSignature>,
    activity_generation: u64,
    result: Result<git::WorktreeSignature, String>,
}

struct LoadResult {
    generation: u64,
    kind: LoadKind,
    fetch_interval: Duration,
    result: Result<
        (
            LoadPayload,
            Option<git::WorktreeSignature>,
            Option<PreparedFileTree>,
        ),
        String,
    >,
}

enum LoadPayload {
    Open(RepositoryData),
    Refresh(RepositoryUpdate),
}

#[derive(Debug)]
enum WorkerKind {
    Commit,
    Fetch {
        repository_generation: u64,
    },
    Command {
        label: String,
    },
    Mutation,
    FileOperation {
        selection: Option<RepoPath>,
        message: String,
    },
    DiscardUnstaged {
        path: RepoPath,
    },
    Format {
        path: RepoPath,
        formatter: &'static str,
    },
    BranchDelete {
        branch: String,
        remote: Option<(String, String)>,
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Commit,
    Fetch,
    Command,
    Mutation,
    Format,
    StatusCheck,
    Load(LoadKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForegroundOperation {
    Commit,
    Command,
    Mutation,
    Format,
}

#[derive(Debug, Default)]
struct OperationState {
    foreground: Option<ForegroundOperation>,
    fetching: bool,
    checking_status: bool,
    loading: Option<LoadKind>,
}

impl OperationState {
    fn can_start(&self, operation: Operation) -> bool {
        match operation {
            Operation::Commit => self.foreground.is_none() && self.loading != Some(LoadKind::Open),
            Operation::Fetch => {
                self.loading.is_none()
                    && !self.fetching
                    && !matches!(
                        self.foreground,
                        Some(
                            ForegroundOperation::Command
                                | ForegroundOperation::Mutation
                                | ForegroundOperation::Format
                        )
                    )
            }
            Operation::Command => {
                self.foreground.is_none() && !self.fetching && self.loading != Some(LoadKind::Open)
            }
            Operation::Mutation => {
                self.foreground.is_none() && !self.fetching && self.loading.is_none()
            }
            Operation::Format => {
                self.foreground.is_none() && !self.fetching && self.loading.is_none()
            }
            Operation::StatusCheck => {
                self.foreground.is_none()
                    && !self.fetching
                    && !self.checking_status
                    && self.loading.is_none()
            }
            Operation::Load(LoadKind::Open) => self.foreground.is_none() && self.loading.is_none(),
            Operation::Load(LoadKind::Reload) => self.loading.is_none(),
        }
    }

    fn start(&mut self, operation: Operation) -> bool {
        if !self.can_start(operation) {
            return false;
        }
        match operation {
            Operation::Commit => self.foreground = Some(ForegroundOperation::Commit),
            Operation::Fetch => self.fetching = true,
            Operation::Command => self.foreground = Some(ForegroundOperation::Command),
            Operation::Mutation => self.foreground = Some(ForegroundOperation::Mutation),
            Operation::Format => self.foreground = Some(ForegroundOperation::Format),
            Operation::StatusCheck => self.checking_status = true,
            Operation::Load(kind) => self.loading = Some(kind),
        }
        true
    }

    fn finish(&mut self, operation: Operation) {
        match operation {
            Operation::Commit if self.foreground == Some(ForegroundOperation::Commit) => {
                self.foreground = None;
            }
            Operation::Command if self.foreground == Some(ForegroundOperation::Command) => {
                self.foreground = None;
            }
            Operation::Mutation if self.foreground == Some(ForegroundOperation::Mutation) => {
                self.foreground = None;
            }
            Operation::Format if self.foreground == Some(ForegroundOperation::Format) => {
                self.foreground = None;
            }
            Operation::Fetch => self.fetching = false,
            Operation::StatusCheck => self.checking_status = false,
            Operation::Load(kind) if self.loading == Some(kind) => self.loading = None,
            Operation::Commit | Operation::Command | Operation::Mutation | Operation::Format => {}
            Operation::Load(_) => {}
        }
    }

    fn is_running(&self, operation: Operation) -> bool {
        match operation {
            Operation::Commit => self.foreground == Some(ForegroundOperation::Commit),
            Operation::Fetch => self.fetching,
            Operation::Command => self.foreground == Some(ForegroundOperation::Command),
            Operation::Mutation => self.foreground == Some(ForegroundOperation::Mutation),
            Operation::Format => self.foreground == Some(ForegroundOperation::Format),
            Operation::StatusCheck => self.checking_status,
            Operation::Load(kind) => self.loading == Some(kind),
        }
    }
}

pub(crate) struct RepositorySession {
    data: Option<RepositoryData>,
    operations: OperationState,
    worker_tx: Sender<WorkerResult>,
    worker_rx: Receiver<WorkerResult>,
    status_tx: Sender<StatusResult>,
    status_rx: Receiver<StatusResult>,
    status_signature: Option<git::WorktreeSignature>,
    next_fetch_at: Instant,
    next_status_check: Instant,
    status_interval: Duration,
    status_activity_generation: u64,
    repository_generation: u64,
    load_generation: u64,
    load_tx: Sender<LoadResult>,
    load_rx: Receiver<LoadResult>,
}

impl RepositorySession {
    pub(crate) fn new(path: &Path, fetch_interval: Duration) -> Self {
        Self::with_data(git::load_or_local(path).ok(), fetch_interval)
    }

    pub(crate) fn opening(path: PathBuf, fetch_interval: Duration) -> Self {
        let mut session = Self::with_data(None, fetch_interval);
        let _ = session.start_open(path, fetch_interval);
        session
    }

    fn with_data(data: Option<RepositoryData>, fetch_interval: Duration) -> Self {
        let (worker_tx, worker_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();
        let (load_tx, load_rx) = mpsc::channel();
        let status_signature = data
            .as_ref()
            .and_then(|repository| repository.worktree_signature);

        Self {
            data,
            operations: OperationState::default(),
            worker_tx,
            worker_rx,
            status_tx,
            status_rx,
            status_signature,
            next_fetch_at: Instant::now() + fetch_interval,
            next_status_check: Instant::now() + MIN_STATUS_INTERVAL,
            status_interval: MIN_STATUS_INTERVAL,
            status_activity_generation: 0,
            repository_generation: 0,
            load_generation: 0,
            load_tx,
            load_rx,
        }
    }

    pub(crate) fn data(&self) -> Option<&RepositoryData> {
        self.data.as_ref()
    }

    fn git_root(&self) -> Option<PathBuf> {
        self.data
            .as_ref()
            .filter(|repository| !repository.is_local())
            .map(|repository| repository.root.clone())
    }

    pub(crate) fn commit_running(&self) -> bool {
        self.operations.is_running(Operation::Commit)
    }

    pub(crate) fn fetch_running(&self) -> bool {
        self.operations.is_running(Operation::Fetch)
    }

    pub(crate) fn command_running(&self) -> bool {
        self.operations.is_running(Operation::Command)
    }

    pub(crate) fn format_running(&self) -> bool {
        self.operations.is_running(Operation::Format)
    }

    pub(crate) fn open_running(&self) -> bool {
        self.operations.is_running(Operation::Load(LoadKind::Open))
    }

    pub(crate) fn can_start_open(&self) -> bool {
        self.operations.can_start(Operation::Load(LoadKind::Open))
    }

    pub(crate) fn can_start_mutation(&self) -> bool {
        self.operations.can_start(Operation::Mutation)
    }

    pub(crate) fn can_restart(&self) -> bool {
        self.operations.foreground.is_none()
            && !self.operations.fetching
            && !self.operations.checking_status
            && self.operations.loading.is_none()
    }

    pub(crate) fn start_open(&mut self, path: PathBuf, fetch_interval: Duration) -> bool {
        self.start_load(
            path,
            LoadKind::Open,
            RefreshScope::ALL,
            None,
            fetch_interval,
        )
    }

    pub(crate) fn start_reload(&mut self, scope: RefreshScope, fetch_interval: Duration) -> bool {
        let Some((root, kind)) = self
            .data
            .as_ref()
            .map(|repository| (repository.root.clone(), repository.kind))
        else {
            return false;
        };
        self.start_load(root, LoadKind::Reload, scope, Some(kind), fetch_interval)
    }

    pub(crate) fn next_load_completion(&mut self) -> Option<LoadCompletion> {
        while let Ok(done) = self.load_rx.try_recv() {
            if done.generation != self.load_generation {
                continue;
            }
            self.operations.finish(Operation::Load(done.kind));
            let (result, prepared_file_tree) = match done.result {
                Ok((payload, signature, prepared_file_tree)) => {
                    if done.kind == LoadKind::Open {
                        self.status_signature = signature;
                        self.next_fetch_at = Instant::now() + done.fetch_interval;
                    } else if signature.is_some() {
                        self.status_signature = signature;
                    }
                    self.reset_status_interval();
                    match payload {
                        LoadPayload::Open(data) => {
                            if let Some(previous) = self.data.replace(data) {
                                diagnostics::drop_in_background("repository-data", previous);
                            }
                        }
                        LoadPayload::Refresh(update) => self
                            .data
                            .as_mut()
                            .expect("refresh completed without repository data")
                            .apply(update),
                    }
                    (Ok(()), prepared_file_tree)
                }
                Err(error) => (Err(error), None),
            };
            return Some(LoadCompletion {
                kind: done.kind,
                result,
                prepared_file_tree,
            });
        }
        None
    }

    pub(crate) fn reset_fetch_deadline(&mut self, fetch_interval: Duration) {
        self.next_fetch_at = Instant::now() + fetch_interval;
    }

    pub(crate) fn start_commit(&mut self, message: String) -> bool {
        if !self.operations.can_start(Operation::Commit) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Commit);
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = git::commit(&root, &message).map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::Commit,
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_command(&mut self, label: String, args: Vec<String>) -> bool {
        if !self.operations.can_start(Operation::Command) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Command);
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = git::run_command(&root, &args).map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::Command { label },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_mutation(&mut self, mutation: Mutation) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = match &mutation {
                Mutation::Stage(change) => git::stage(&root, change),
                Mutation::Unstage(change) => git::unstage(&root, change),
                Mutation::StageAll => git::stage_all(&root),
                Mutation::UnstageAll => git::unstage_all(&root),
                Mutation::StageHunk { patch, index } => git::stage_hunk(&root, patch, *index),
            }
            .map(|()| CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                success: true,
                exit_code: Some(0),
            })
            .map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::Mutation,
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_file_operation(&mut self, operation: FileOperation) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.data.as_ref().map(|repository| repository.root.clone()) else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let selection = operation.selection_after();
        let message = operation.success_message();
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = filesystem::perform(&root, &operation)
                .map(|()| CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    success: true,
                    exit_code: Some(0),
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::FileOperation { selection, message },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_discard_unstaged(&mut self, change: Change) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let path = change.path.clone();
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = git::discard_unstaged(&root, &change)
                .map(|()| CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    success: true,
                    exit_code: Some(0),
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::DiscardUnstaged { path },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_branch_delete(
        &mut self,
        branch: String,
        remote: Option<(String, String)>,
        force: bool,
    ) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let sender = self.worker_tx.clone();
        let worker_branch = branch.clone();
        let worker_remote = remote.clone();
        thread::spawn(move || {
            let remote_ref = worker_remote
                .as_ref()
                .map(|(remote, branch)| (remote.as_str(), branch.as_str()));
            let result = git::delete_branch(&root, &worker_branch, remote_ref, force)
                .map(|()| CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    success: true,
                    exit_code: Some(0),
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::BranchDelete {
                    branch,
                    remote,
                    force,
                },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_format(&mut self, path: RepoPath, command: FormatCommand) -> bool {
        if !self.operations.can_start(Operation::Format) {
            return false;
        }
        let Some(root) = self.data.as_ref().map(|repository| repository.root.clone()) else {
            return false;
        };

        self.operations.start(Operation::Format);
        let formatter = command.label;
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = formatter::run(&root, &path, &command).map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::Format { path, formatter },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn maybe_start_fetch(&mut self, enabled: bool, fetch_interval: Duration) {
        if !enabled || Instant::now() < self.next_fetch_at {
            return;
        }
        self.start_fetch(fetch_interval);
    }

    pub(crate) fn start_fetch(&mut self, fetch_interval: Duration) -> bool {
        if !self.operations.can_start(Operation::Fetch) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Fetch);
        self.next_fetch_at = Instant::now() + fetch_interval;
        let repository_generation = self.repository_generation;
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = git::fetch(&root).map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::Fetch {
                    repository_generation,
                },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn maybe_start_status_check(&mut self) {
        let now = Instant::now();
        if !self.operations.can_start(Operation::StatusCheck) || now < self.next_status_check {
            return;
        }
        let Some(root) = self.git_root() else {
            return;
        };

        self.operations.start(Operation::StatusCheck);
        self.next_status_check = now + self.status_interval;
        let baseline = self.status_signature;
        let activity_generation = self.status_activity_generation;
        let repository_generation = self.repository_generation;
        let sender = self.status_tx.clone();
        thread::spawn(move || {
            let result = git::worktree_signature(&root).map_err(|error| error.to_string());
            let _ = sender.send(StatusResult {
                root,
                repository_generation,
                baseline,
                activity_generation,
                result,
            });
        });
    }

    pub(crate) fn next_worker_completion(
        &mut self,
        fetch_interval: Duration,
    ) -> Option<WorkerCompletion> {
        while let Ok(done) = self.worker_rx.try_recv() {
            let active = self
                .data
                .as_ref()
                .is_some_and(|repository| repository.root == done.root);
            match done.kind {
                WorkerKind::Commit => {
                    self.operations.finish(Operation::Commit);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::Commit(done.result)));
                    }
                }
                WorkerKind::Fetch {
                    repository_generation,
                } => {
                    if repository_generation != self.repository_generation {
                        continue;
                    }
                    self.operations.finish(Operation::Fetch);
                    self.next_fetch_at = Instant::now() + fetch_interval;
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::Fetch(done.result)));
                    }
                }
                WorkerKind::Command { label } => {
                    self.operations.finish(Operation::Command);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::Command(
                            CommandCompletion {
                                label,
                                result: done.result,
                            },
                        )));
                    }
                }
                WorkerKind::Mutation => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::Mutation(
                            done.result.map(|_| ()),
                        )));
                    }
                }
                WorkerKind::FileOperation { selection, message } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::FileOperation(
                            FileOperationCompletion {
                                result: done.result.map(|_| selection),
                                message,
                            },
                        )));
                    }
                }
                WorkerKind::DiscardUnstaged { path } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::DiscardUnstaged(
                            DiscardUnstagedCompletion {
                                path,
                                result: done.result.map(|_| ()),
                            },
                        )));
                    }
                }
                WorkerKind::Format { path, formatter } => {
                    self.operations.finish(Operation::Format);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::Format(
                            FormatCompletion {
                                path,
                                formatter,
                                result: done.result,
                            },
                        )));
                    }
                }
                WorkerKind::BranchDelete {
                    branch,
                    remote,
                    force,
                } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(WorkerCompletion::new(WorkerOutcome::BranchDelete(
                            BranchDeleteCompletion {
                                branch,
                                remote,
                                force,
                                result: done.result.map(|_| ()),
                            },
                        )));
                    }
                }
            }
        }
        None
    }

    pub(crate) fn next_worktree_change(&mut self) -> Option<RefreshScope> {
        while let Ok(done) = self.status_rx.try_recv() {
            if done.repository_generation != self.repository_generation {
                continue;
            }
            self.operations.finish(Operation::StatusCheck);
            let active = self
                .data
                .as_ref()
                .is_some_and(|repository| repository.root == done.root);
            if !active || self.status_signature != done.baseline {
                continue;
            }
            if let Ok(signature) = done.result {
                let previous = self.status_signature.replace(signature);
                if let Some(previous) = previous.filter(|previous| *previous != signature) {
                    self.reset_status_interval();
                    return Some(signature.refresh_scope_since(previous));
                }
            }
            if done.activity_generation == self.status_activity_generation {
                self.status_interval = self
                    .status_interval
                    .saturating_mul(2)
                    .min(MAX_STATUS_INTERVAL);
            } else {
                self.status_interval = MIN_STATUS_INTERVAL;
            }
            self.next_status_check = Instant::now() + self.status_interval;
        }
        None
    }

    pub(crate) fn note_activity(&mut self) {
        self.status_activity_generation = self.status_activity_generation.wrapping_add(1);
        self.status_interval = MIN_STATUS_INTERVAL;
        let now = Instant::now();
        let active_deadline = now + MIN_STATUS_INTERVAL;
        self.next_status_check = if self.next_status_check <= now {
            active_deadline
        } else {
            self.next_status_check.min(active_deadline)
        };
    }

    fn reset_status_interval(&mut self) {
        self.status_interval = MIN_STATUS_INTERVAL;
        self.next_status_check = Instant::now() + MIN_STATUS_INTERVAL;
    }

    fn start_load(
        &mut self,
        path: PathBuf,
        kind: LoadKind,
        scope: RefreshScope,
        repository_kind: Option<RepositoryKind>,
        fetch_interval: Duration,
    ) -> bool {
        if !self.operations.start(Operation::Load(kind)) {
            diagnostics::event(format!(
                "load rejected kind={kind:?} path={}",
                path.display()
            ));
            return false;
        }
        if kind == LoadKind::Open {
            self.repository_generation = self.repository_generation.wrapping_add(1);
            self.operations.finish(Operation::Fetch);
            self.operations.finish(Operation::StatusCheck);
        }
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        let sender = self.load_tx.clone();
        diagnostics::event(format!(
            "load started generation={generation} kind={kind:?} path={}",
            path.display()
        ));
        thread::spawn(move || {
            let started = Instant::now();
            let result = match repository_kind {
                None => git::load_or_local(&path).map(LoadPayload::Open),
                Some(repository_kind) => {
                    git::refresh_repository(&path, repository_kind, scope).map(LoadPayload::Refresh)
                }
            }
            .map(|payload| {
                let prepared_file_tree = match &payload {
                    LoadPayload::Open(data) => {
                        Some(PreparedFileTree::new(&data.files, &data.directories))
                    }
                    LoadPayload::Refresh(_) => None,
                };
                let signature = match &payload {
                    LoadPayload::Open(data) => data.worktree_signature,
                    LoadPayload::Refresh(update) => update.worktree_signature(),
                };
                (payload, signature, prepared_file_tree)
            })
            .map_err(|error| error.to_string());
            diagnostics::event(format!(
                "load finished generation={generation} kind={kind:?} path={} elapsed_ms={} success={}",
                path.display(),
                started.elapsed().as_millis(),
                result.is_ok()
            ));
            let _ = sender.send(LoadResult {
                generation,
                kind,
                fetch_interval,
                result,
            });
        });
        true
    }

    #[cfg(test)]
    pub(crate) fn schedule_fetch_now(&mut self) {
        self.next_fetch_at = Instant::now();
    }

    #[cfg(test)]
    pub(crate) fn schedule_status_check_now(&mut self) {
        self.next_status_check = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_output(success: bool) -> CommandOutput {
        CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            success,
            exit_code: Some(if success { 0 } else { 1 }),
        }
    }

    #[test]
    fn completions_declare_repository_invalidation_policy() {
        assert_eq!(
            WorkerCompletion::new(WorkerOutcome::Commit(Ok(command_output(true)))).invalidation(),
            Some(RefreshScope::WORKTREE.union(RefreshScope::HISTORY_AND_REFS))
        );
        assert_eq!(
            WorkerCompletion::new(WorkerOutcome::Command(CommandCompletion {
                label: "command".to_owned(),
                result: Ok(command_output(false)),
            }))
            .invalidation(),
            None
        );
        assert_eq!(
            WorkerCompletion::new(WorkerOutcome::FileOperation(FileOperationCompletion {
                result: Ok(None),
                message: "saved".to_owned(),
            }))
            .invalidation(),
            Some(RefreshScope::WORKTREE_AND_INVENTORY)
        );
        assert_eq!(
            WorkerCompletion::new(WorkerOutcome::DiscardUnstaged(DiscardUnstagedCompletion {
                path: "file".into(),
                result: Err("failed".to_owned()),
            }))
            .invalidation(),
            Some(RefreshScope::WORKTREE_AND_INVENTORY)
        );
        assert_eq!(
            WorkerCompletion::new(WorkerOutcome::BranchDelete(BranchDeleteCompletion {
                branch: "topic".to_owned(),
                remote: None,
                force: false,
                result: Err("failed".to_owned()),
            }))
            .invalidation(),
            Some(RefreshScope::HISTORY_AND_REFS)
        );
    }

    #[test]
    fn ignores_worker_completion_from_a_previous_repository() {
        let mut session = session("/active", Some(10));
        session.operations.start(Operation::Commit);
        session
            .worker_tx
            .send(WorkerResult {
                kind: WorkerKind::Commit,
                root: PathBuf::from("/previous"),
                result: Err("old result".to_owned()),
            })
            .unwrap();

        assert!(
            session
                .next_worker_completion(Duration::from_secs(60))
                .is_none()
        );
        assert!(!session.commit_running());
    }

    #[test]
    fn ignores_command_completion_from_a_previous_repository() {
        let mut session = session("/active", Some(10));
        session.operations.start(Operation::Command);
        session
            .worker_tx
            .send(WorkerResult {
                kind: WorkerKind::Command {
                    label: "Push".to_owned(),
                },
                root: PathBuf::from("/previous"),
                result: Err("old result".to_owned()),
            })
            .unwrap();

        assert!(
            session
                .next_worker_completion(Duration::from_secs(60))
                .is_none()
        );
        assert!(!session.command_running());
    }

    #[test]
    fn ignores_superseded_repository_loads() {
        let mut session = session("/active", Some(7));
        session.load_generation = 2;
        session.operations.start(Operation::Load(LoadKind::Open));
        let mut stale_data = session.data.clone().unwrap();
        stale_data.root = PathBuf::from("/stale");
        session
            .load_tx
            .send(LoadResult {
                generation: 1,
                kind: LoadKind::Open,
                fetch_interval: Duration::ZERO,
                result: Ok((LoadPayload::Open(stale_data), Some(signature(99, 1)), None)),
            })
            .unwrap();

        assert!(session.next_load_completion().is_none());
        assert_eq!(session.data().unwrap().root, Path::new("/active"));
        assert_eq!(session.status_signature, Some(signature(7, 1)));
        assert!(
            session
                .operations
                .is_running(Operation::Load(LoadKind::Open))
        );
    }

    #[test]
    fn ignores_status_result_from_a_previous_repository() {
        let mut session = session("/active", Some(10));
        session.operations.start(Operation::StatusCheck);
        session
            .status_tx
            .send(StatusResult {
                root: PathBuf::from("/previous"),
                repository_generation: 0,
                baseline: Some(signature(10, 1)),
                activity_generation: 0,
                result: Ok(signature(20, 1)),
            })
            .unwrap();

        assert!(session.next_worktree_change().is_none());
        assert_eq!(session.status_signature, Some(signature(10, 1)));
        assert!(!session.operations.is_running(Operation::StatusCheck));
    }

    #[test]
    fn ignores_status_result_with_a_superseded_baseline() {
        let mut session = session("/active", Some(20));
        session.operations.start(Operation::StatusCheck);
        session
            .status_tx
            .send(StatusResult {
                root: PathBuf::from("/active"),
                repository_generation: 0,
                baseline: Some(signature(10, 1)),
                activity_generation: 0,
                result: Ok(signature(30, 1)),
            })
            .unwrap();

        assert!(session.next_worktree_change().is_none());
        assert_eq!(session.status_signature, Some(signature(20, 1)));
        assert!(!session.operations.is_running(Operation::StatusCheck));
    }

    #[test]
    fn stale_background_results_do_not_clear_current_repository_operations() {
        let mut session = session("/active", Some(10));
        session.repository_generation = 2;
        session.operations.start(Operation::Fetch);
        session
            .worker_tx
            .send(WorkerResult {
                kind: WorkerKind::Fetch {
                    repository_generation: 1,
                },
                root: PathBuf::from("/active"),
                result: Err("stale fetch".to_owned()),
            })
            .unwrap();
        assert!(
            session
                .next_worker_completion(Duration::from_secs(60))
                .is_none()
        );
        assert!(session.operations.is_running(Operation::Fetch));

        session.operations.finish(Operation::Fetch);
        session.operations.start(Operation::StatusCheck);
        session
            .status_tx
            .send(StatusResult {
                root: PathBuf::from("/active"),
                repository_generation: 1,
                baseline: Some(signature(10, 1)),
                activity_generation: 0,
                result: Err("stale status".to_owned()),
            })
            .unwrap();

        assert!(session.next_worktree_change().is_none());
        assert!(session.operations.is_running(Operation::StatusCheck));
        assert_eq!(session.status_signature, Some(signature(10, 1)));
    }

    #[test]
    fn local_workspaces_do_not_schedule_git_background_work() {
        let mut session = session("/local", None);
        session.data.as_mut().unwrap().kind = git::RepositoryKind::Local;
        session.schedule_fetch_now();
        session.schedule_status_check_now();

        session.maybe_start_fetch(true, Duration::ZERO);
        session.maybe_start_status_check();

        assert!(!session.operations.is_running(Operation::Fetch));
        assert!(!session.operations.is_running(Operation::StatusCheck));
        assert!(!session.start_commit("local".to_owned()));
        assert!(!session.start_command("Status".to_owned(), vec!["status".to_owned()]));
        assert!(!session.start_mutation(Mutation::StageAll));
    }

    #[test]
    fn activity_keeps_status_checks_at_a_bounded_interval() {
        let mut session = session("/active", Some(10));
        session.next_status_check = Instant::now();
        session.note_activity();
        assert!(session.next_status_check > Instant::now());

        session.operations.start(Operation::StatusCheck);
        session
            .status_tx
            .send(StatusResult {
                root: PathBuf::from("/active"),
                repository_generation: 0,
                baseline: Some(signature(10, 1)),
                activity_generation: 0,
                result: Ok(signature(10, 1)),
            })
            .unwrap();
        assert!(session.next_worktree_change().is_none());
        assert_eq!(session.status_interval, MIN_STATUS_INTERVAL);
        assert!(session.next_status_check > Instant::now());
    }

    #[test]
    fn scopes_external_refreshes_by_branch_identity() {
        let mut session = session("/active", Some(10));
        session.operations.start(Operation::StatusCheck);
        session
            .status_tx
            .send(StatusResult {
                root: PathBuf::from("/active"),
                repository_generation: 0,
                baseline: Some(signature(10, 1)),
                activity_generation: 0,
                result: Ok(signature(20, 1)),
            })
            .unwrap();
        assert_eq!(
            session.next_worktree_change(),
            Some(RefreshScope::WORKTREE_AND_INVENTORY)
        );

        session.operations.start(Operation::StatusCheck);
        session
            .status_tx
            .send(StatusResult {
                root: PathBuf::from("/active"),
                repository_generation: 0,
                baseline: Some(signature(20, 1)),
                activity_generation: 0,
                result: Ok(signature(30, 2)),
            })
            .unwrap();
        assert_eq!(session.next_worktree_change(), Some(RefreshScope::ALL));
    }

    #[test]
    fn operation_state_preserves_repository_concurrency_rules() {
        let mut committing = OperationState::default();
        assert!(committing.start(Operation::Commit));
        assert!(committing.can_start(Operation::Fetch));
        assert!(committing.can_start(Operation::Load(LoadKind::Reload)));
        assert!(!committing.can_start(Operation::Command));
        assert!(!committing.can_start(Operation::Mutation));
        assert!(!committing.can_start(Operation::StatusCheck));

        let mut fetching = OperationState::default();
        assert!(fetching.start(Operation::Fetch));
        assert!(fetching.can_start(Operation::Commit));
        assert!(fetching.can_start(Operation::Load(LoadKind::Reload)));
        assert!(!fetching.can_start(Operation::Command));
        assert!(!fetching.can_start(Operation::Mutation));
        assert!(!fetching.can_start(Operation::StatusCheck));

        let mut mutating = OperationState::default();
        assert!(mutating.start(Operation::Mutation));
        assert!(mutating.can_start(Operation::Load(LoadKind::Reload)));
        assert!(!mutating.can_start(Operation::Load(LoadKind::Open)));
        assert!(!mutating.can_start(Operation::Commit));
        assert!(!mutating.can_start(Operation::Fetch));

        let mut formatting = OperationState::default();
        assert!(formatting.start(Operation::Format));
        assert!(!formatting.can_start(Operation::Commit));
        assert!(!formatting.can_start(Operation::Fetch));
        assert!(!formatting.can_start(Operation::Mutation));
        assert!(!formatting.can_start(Operation::Load(LoadKind::Open)));

        let mut loading = OperationState::default();
        assert!(loading.start(Operation::Load(LoadKind::Open)));
        assert!(!loading.can_start(Operation::Commit));
        assert!(!loading.can_start(Operation::Command));
        assert!(!loading.can_start(Operation::Load(LoadKind::Open)));
        assert!(!loading.can_start(Operation::Load(LoadKind::Reload)));
        assert!(!loading.can_start(Operation::Fetch));
        assert!(!loading.can_start(Operation::Mutation));
        assert!(!loading.can_start(Operation::StatusCheck));

        let mut reloading = OperationState::default();
        assert!(reloading.start(Operation::Load(LoadKind::Reload)));
        assert!(reloading.can_start(Operation::Commit));
        assert!(reloading.can_start(Operation::Command));

        let mut checking_status = OperationState::default();
        assert!(checking_status.start(Operation::StatusCheck));
        assert!(checking_status.can_start(Operation::Commit));
        assert!(checking_status.can_start(Operation::Fetch));
        assert!(checking_status.can_start(Operation::Command));
        assert!(checking_status.can_start(Operation::Mutation));
        assert!(checking_status.can_start(Operation::Load(LoadKind::Open)));
        assert!(!checking_status.can_start(Operation::StatusCheck));
    }

    fn signature(state: u64, branch: u64) -> git::WorktreeSignature {
        git::WorktreeSignature::for_test(state, branch)
    }

    fn session(root: &str, status_signature: Option<u64>) -> RepositorySession {
        let status_signature = status_signature.map(|state| signature(state, 1));
        let (worker_tx, worker_rx) = mpsc::channel();
        let (status_tx, status_rx) = mpsc::channel();
        let (load_tx, load_rx) = mpsc::channel();
        RepositorySession {
            data: Some(RepositoryData {
                root: PathBuf::from(root),
                common_dir: None,
                kind: git::RepositoryKind::Git,
                branch: "main".to_owned(),
                branches: Vec::new(),
                github_remote: false,
                changes: Vec::new(),
                files: Vec::new(),
                directories: Vec::new(),
                history: Vec::new(),
                commits: Vec::new(),
                files_fingerprint: 0,
                inventory_truncated: false,
                changes_fingerprint: 0,
                change_counts: (0, 0),
                graph_width: 0,
                graph_truncated: false,
                worktree_signature: status_signature,
            }),
            operations: OperationState::default(),
            worker_tx,
            worker_rx,
            status_tx,
            status_rx,
            status_signature,
            next_fetch_at: Instant::now(),
            next_status_check: Instant::now(),
            status_interval: MIN_STATUS_INTERVAL,
            status_activity_generation: 0,
            repository_generation: 0,
            load_generation: 0,
            load_tx,
            load_rx,
        }
    }
}
