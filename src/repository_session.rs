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
        self, Change, CommandOutput, InventoryRefresh, RefreshScope, RepositoryData,
        RepositoryKind, RepositoryUpdate,
    },
    repo_path::RepoPath,
    tree::PreparedFileTree,
};

mod operation_state;

use operation_state::{Operation, OperationState};

const MIN_STATUS_INTERVAL: Duration = Duration::from_millis(800);
const MAX_STATUS_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) enum WorkerOutcome {
    Commit(Result<CommandOutput, String>),
    Fetch(Result<CommandOutput, String>),
    Command(CommandCompletion),
    Mutation(Result<(), String>),
    FileOperation(FileOperationCompletion),
    DiscardUnstaged(DiscardUnstagedCompletion),
    Format(FormatCompletion),
    BranchCheckout(BranchCheckoutCompletion),
    BranchCreate(BranchCreateCompletion),
    BranchDelete(BranchDeleteCompletion),
}

pub(crate) struct WorkerCompletion {
    pub(crate) outcome: WorkerOutcome,
    invalidation: Option<RefreshScope>,
    refresh_request: Option<RefreshRequest>,
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
            // A formatter may rewrite a file before returning a failure.
            WorkerOutcome::Format(_) => Some(RefreshScope::WORKTREE),
            // A failed switch can still update the index or working tree.
            WorkerOutcome::BranchCheckout(_) => Some(RefreshScope::ALL),
            WorkerOutcome::BranchCreate(_) => Some(RefreshScope::ALL),
            WorkerOutcome::BranchDelete(done)
                if done.result.as_ref().is_ok_and(|output| output.success) =>
            {
                Some(RefreshScope::HISTORY_AND_REFS)
            }
            WorkerOutcome::Commit(_)
            | WorkerOutcome::Fetch(_)
            | WorkerOutcome::Command(_)
            | WorkerOutcome::Mutation(_)
            | WorkerOutcome::FileOperation(_)
            | WorkerOutcome::BranchDelete(_) => None,
        };
        Self {
            outcome,
            invalidation,
            refresh_request: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn invalidation(&self) -> Option<RefreshScope> {
        self.invalidation
    }

    pub(crate) fn refresh_request(&self) -> Option<RefreshRequest> {
        self.refresh_request
    }
}

pub(crate) struct BranchCheckoutCompletion {
    pub(crate) branch: String,
    pub(crate) result: Result<CommandOutput, String>,
}

pub(crate) struct BranchCreateCompletion {
    pub(crate) branch: String,
    pub(crate) result: Result<CommandOutput, String>,
}

pub(crate) struct BranchDeleteCompletion {
    pub(crate) branch: String,
    pub(crate) result: Result<CommandOutput, String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshRequest {
    Started(RefreshScope),
    Queued(RefreshScope),
}

impl RefreshRequest {
    pub(crate) fn scope(self) -> RefreshScope {
        match self {
            Self::Started(scope) | Self::Queued(scope) => scope,
        }
    }

    pub(crate) fn started(self) -> bool {
        matches!(self, Self::Started(_))
    }
}

pub(crate) struct LoadCompletion {
    pub(crate) kind: LoadKind,
    pub(crate) scope: RefreshScope,
    pub(crate) inventory_refresh: InventoryRefresh,
    pub(crate) result: Result<(), String>,
    pub(crate) prepared_file_tree: Option<PreparedFileTree>,
    pub(crate) follow_up_refresh: Option<RefreshRequest>,
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
    result: Result<git::WorktreeStatus, String>,
}

struct LoadResult {
    generation: u64,
    kind: LoadKind,
    scope: RefreshScope,
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
    BranchCheckout {
        branch: String,
    },
    BranchCreate {
        branch: String,
    },
    BranchDelete {
        branch: String,
    },
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
    active_refresh_scope: Option<RefreshScope>,
    queued_refresh: Option<RefreshScope>,
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
            active_refresh_scope: None,
            queued_refresh: None,
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
            .filter(|repository| !repository.is_local() && repository.details_ready)
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
        self.operations.is_idle()
    }

    pub(crate) fn start_open(&mut self, path: PathBuf, fetch_interval: Duration) -> bool {
        let started = self.start_load(
            path,
            LoadKind::Open,
            RefreshScope::ALL,
            None,
            None,
            fetch_interval,
        );
        if started {
            self.active_refresh_scope = None;
            self.queued_refresh = None;
        }
        started
    }

    pub(crate) fn request_refresh(
        &mut self,
        scope: RefreshScope,
        fetch_interval: Duration,
    ) -> Option<RefreshRequest> {
        self.request_refresh_with_status(scope, None, fetch_interval)
    }

    fn request_refresh_with_status(
        &mut self,
        scope: RefreshScope,
        worktree_status: Option<git::WorktreeStatus>,
        fetch_interval: Duration,
    ) -> Option<RefreshRequest> {
        if self.open_running() {
            return None;
        }
        if self
            .operations
            .is_running(Operation::Load(LoadKind::Reload))
        {
            self.queued_refresh = Some(
                self.queued_refresh
                    .map_or(scope, |queued| queued.union(scope)),
            );
            return Some(RefreshRequest::Queued(scope));
        }
        let (root, kind, details_ready) = self.data.as_ref().map(|repository| {
            (
                repository.root.clone(),
                repository.kind,
                repository.details_ready,
            )
        })?;
        let scope = if details_ready {
            scope
        } else {
            RefreshScope::ALL
        };
        if self.start_load(
            root,
            LoadKind::Reload,
            scope,
            Some(kind),
            worktree_status,
            fetch_interval,
        ) {
            self.active_refresh_scope = Some(scope);
            Some(RefreshRequest::Started(scope))
        } else {
            self.queued_refresh = Some(
                self.queued_refresh
                    .map_or(scope, |queued| queued.union(scope)),
            );
            Some(RefreshRequest::Queued(scope))
        }
    }

    pub(crate) fn next_load_completion(&mut self) -> Option<LoadCompletion> {
        while let Ok(done) = self.load_rx.try_recv() {
            if done.generation != self.load_generation {
                continue;
            }
            self.operations.finish(Operation::Load(done.kind));
            if done.kind == LoadKind::Reload {
                self.active_refresh_scope = None;
            }
            let mut inventory_refresh = InventoryRefresh::Unchanged;
            let (result, prepared_file_tree) = match done.result {
                Ok((payload, signature, prepared_file_tree)) => {
                    if done.kind == LoadKind::Open {
                        self.status_signature = signature;
                        self.next_fetch_at = Instant::now() + done.fetch_interval;
                    } else if signature.is_some() {
                        self.status_signature = signature;
                    }
                    if done.kind == LoadKind::Open || done.scope.includes_worktree() {
                        self.reset_status_interval();
                    }
                    match payload {
                        LoadPayload::Open(data) => {
                            if let Some(previous) = self.data.replace(data) {
                                diagnostics::drop_in_background("repository-data", previous);
                            }
                        }
                        LoadPayload::Refresh(update) => {
                            let repository = self
                                .data
                                .as_mut()
                                .expect("refresh completed without repository data");
                            inventory_refresh = update.inventory_refresh(repository);
                            repository.apply(update);
                        }
                    }
                    (Ok(()), prepared_file_tree)
                }
                Err(error) => (Err(error), None),
            };
            let follow_up_scope = match done.kind {
                LoadKind::Open if result.is_ok() => Some(RefreshScope::ALL),
                LoadKind::Open
                    if self
                        .data
                        .as_ref()
                        .is_some_and(|repository| !repository.details_ready) =>
                {
                    Some(RefreshScope::ALL)
                }
                LoadKind::Open => None,
                LoadKind::Reload => self.queued_refresh.take().map(|scope| {
                    if self
                        .data
                        .as_ref()
                        .is_some_and(|repository| repository.details_ready)
                    {
                        scope
                    } else {
                        RefreshScope::ALL
                    }
                }),
            };
            let follow_up_refresh =
                follow_up_scope.and_then(|scope| self.request_refresh(scope, done.fetch_interval));
            return Some(LoadCompletion {
                kind: done.kind,
                scope: done.scope,
                inventory_refresh,
                result,
                prepared_file_tree,
                follow_up_refresh,
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

    pub(crate) fn start_branch_checkout(&mut self, branch: String, remote: bool) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let sender = self.worker_tx.clone();
        let worker_branch = branch.clone();
        thread::spawn(move || {
            let result = git::checkout_branch(&root, &worker_branch, remote)
                .map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::BranchCheckout { branch },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_branch_create(&mut self, branch: String, base: String) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let sender = self.worker_tx.clone();
        let worker_branch = branch.clone();
        thread::spawn(move || {
            let result =
                git::create_branch(&root, &worker_branch, &base).map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::BranchCreate { branch },
                root,
                result,
            });
        });
        true
    }

    pub(crate) fn start_branch_delete(&mut self, branch: String) -> bool {
        if !self.operations.can_start(Operation::Mutation) {
            return false;
        }
        let Some(root) = self.git_root() else {
            return false;
        };

        self.operations.start(Operation::Mutation);
        let sender = self.worker_tx.clone();
        let worker_branch = branch.clone();
        thread::spawn(move || {
            let result =
                git::delete_branch(&root, &worker_branch).map_err(|error| error.to_string());
            let _ = sender.send(WorkerResult {
                kind: WorkerKind::BranchDelete { branch },
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
            let result = git::worktree_status(&root).map_err(|error| error.to_string());
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
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::Commit(done.result)),
                            fetch_interval,
                        ));
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
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::Fetch(done.result)),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::Command { label } => {
                    self.operations.finish(Operation::Command);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::Command(CommandCompletion {
                                label,
                                result: done.result,
                            })),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::Mutation => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::Mutation(done.result.map(|_| ()))),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::FileOperation { selection, message } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::FileOperation(
                                FileOperationCompletion {
                                    result: done.result.map(|_| selection),
                                    message,
                                },
                            )),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::DiscardUnstaged { path } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::DiscardUnstaged(
                                DiscardUnstagedCompletion {
                                    path,
                                    result: done.result.map(|_| ()),
                                },
                            )),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::Format { path, formatter } => {
                    self.operations.finish(Operation::Format);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::Format(FormatCompletion {
                                path,
                                formatter,
                                result: done.result,
                            })),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::BranchCheckout { branch } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::BranchCheckout(
                                BranchCheckoutCompletion {
                                    branch,
                                    result: done.result,
                                },
                            )),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::BranchCreate { branch } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::BranchCreate(
                                BranchCreateCompletion {
                                    branch,
                                    result: done.result,
                                },
                            )),
                            fetch_interval,
                        ));
                    }
                }
                WorkerKind::BranchDelete { branch } => {
                    self.operations.finish(Operation::Mutation);
                    if active {
                        return Some(self.schedule_completion_refresh(
                            WorkerCompletion::new(WorkerOutcome::BranchDelete(
                                BranchDeleteCompletion {
                                    branch,
                                    result: done.result,
                                },
                            )),
                            fetch_interval,
                        ));
                    }
                }
            }
        }
        None
    }

    fn schedule_completion_refresh(
        &mut self,
        mut completion: WorkerCompletion,
        fetch_interval: Duration,
    ) -> WorkerCompletion {
        completion.refresh_request = completion
            .invalidation
            .and_then(|scope| self.request_refresh(scope, fetch_interval));
        completion
    }

    pub(crate) fn next_worktree_change(
        &mut self,
        fetch_interval: Duration,
    ) -> Option<RefreshRequest> {
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
            if let Ok(status) = done.result {
                let signature = status.signature();
                let previous = self.status_signature.replace(signature);
                if let Some(previous) = previous.filter(|previous| *previous != signature) {
                    self.reset_status_interval();
                    let scope = signature.refresh_scope_since(previous);
                    if let Some(request) =
                        self.request_refresh_with_status(scope, Some(status), fetch_interval)
                    {
                        return Some(request);
                    }
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

    #[cfg(test)]
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
        worktree_status: Option<git::WorktreeStatus>,
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
                None => git::bootstrap_or_local(&path).map(LoadPayload::Open),
                Some(repository_kind) => git::refresh_repository_with_status(
                    &path,
                    repository_kind,
                    scope,
                    worktree_status,
                )
                .map(LoadPayload::Refresh),
            }
            .map(|payload| {
                let prepared_file_tree = match &payload {
                    LoadPayload::Open(data) => Some(PreparedFileTree::new(&data.root)),
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
                scope,
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
mod tests;
