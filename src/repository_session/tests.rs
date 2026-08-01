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
    assert_eq!(
        WorkerCompletion::new(WorkerOutcome::BranchCheckout(BranchCheckoutCompletion {
            branch: "topic".to_owned(),
            result: Err("failed".to_owned()),
        }))
        .invalidation(),
        Some(RefreshScope::ALL)
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
    assert!(reloading.can_start(Operation::Load(LoadKind::Open)));

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
            details_ready: true,
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
