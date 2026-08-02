use std::{fs, path::PathBuf, thread, time::Duration};

use crate::git::{Change, InventoryRefresh, RepositoryKind};

use super::*;

fn repository_data() -> RepositoryData {
    RepositoryData {
        root: PathBuf::new(),
        common_dir: None,
        kind: RepositoryKind::Git,
        branch: "main".to_owned(),
        branches: Vec::new(),
        worktree_signature: None,
        changes: vec![Change {
            path: "src/main.rs".into(),
            original_path: None,
            code: 'M',
            staged: false,
            additions: 0,
            deletions: 0,
        }],
        files: vec![
            "src/app/mod.rs".into(),
            "src/main.rs".into(),
            "README.md".into(),
        ],
        directories: Vec::new(),
        history: Vec::new(),
        commits: Vec::new(),
        files_fingerprint: 1,
        inventory_truncated: false,
        changes_fingerprint: 1,
        change_counts: (0, 1),
        graph_width: 0,
        graph_truncated: false,
        details_ready: true,
    }
}

#[test]
fn starts_files_collapsed_but_keeps_worktree_expanded() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/app")).unwrap();
    fs::write(directory.path().join("src/app/mod.rs"), "").unwrap();
    fs::write(directory.path().join("src/main.rs"), "").unwrap();
    fs::write(directory.path().join("README.md"), "").unwrap();
    let mut repo = repository_data();
    repo.root = directory.path().to_owned();

    let mut state = ChangesState::new(Some(&repo));
    assert!(state.collapsed_directories.is_empty());
    assert!(state.expanded_explorer_directories.is_empty());
    assert_eq!(
        state
            .explorer_rows()
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        ["src", "README.md"]
    );
    assert_eq!(state.explorer_state.selected(), Some(1));

    state.explorer_state.select(Some(0));
    state.expand_or_descend_explorer(Some(&repo));
    for _ in 0..100 {
        if state.poll_directories(Some(&repo)) {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        state
            .explorer_rows()
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        ["src", "app", "main.rs", "README.md"]
    );
    assert_eq!(state.explorer_rows()[1].directory_expanded, Some(false));
}

#[test]
fn explorer_uses_the_filesystem_instead_of_the_capped_inventory() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("jobs/k3-max-sleeve")).unwrap();
    fs::write(
        directory.path().join("jobs/k3-max-sleeve/result.db"),
        "result",
    )
    .unwrap();
    let mut repo = repository_data();
    repo.root = directory.path().to_owned();
    repo.files.clear();
    repo.directories.clear();
    repo.inventory_truncated = true;

    let mut state = ChangesState::new(Some(&repo));
    assert!(state.explorer_rows().iter().any(|row| {
        row.directory_path
            .as_ref()
            .is_some_and(|path| path == "jobs")
    }));

    assert!(state.select_explorer_path(&repo, &"jobs/k3-max-sleeve/result.db".into(), 20));
    for _ in 0..100 {
        state.poll_directories(Some(&repo));
        if state
            .selected_explorer_file_path(&repo)
            .is_some_and(|path| path == "jobs/k3-max-sleeve/result.db")
        {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("deep filesystem path did not load");
}

#[test]
fn explicit_explorer_selection_cancels_a_pending_deep_reveal() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/nested")).unwrap();
    fs::write(directory.path().join("src/nested/main.rs"), "").unwrap();
    fs::write(directory.path().join("README.md"), "readme").unwrap();
    let mut repo = repository_data();
    repo.root = directory.path().to_owned();
    repo.files = vec!["README.md".into(), "src/nested/main.rs".into()];

    let mut state = ChangesState::new(Some(&repo));
    assert!(state.select_explorer_path(&repo, &"src/nested/main.rs".into(), 20));
    let readme = state
        .explorer_rows()
        .iter()
        .position(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path == "README.md")
        })
        .unwrap();
    assert!(state.select_explorer_row(&repo, readme));

    for _ in 0..100 {
        state.poll_directories(Some(&repo));
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        state.selected_explorer_file_path(&repo),
        Some(&RepoPath::from("README.md"))
    );
}

#[test]
fn boundary_navigation_keeps_the_current_preview() {
    let repo = repository_data();
    let mut state = ChangesState::new(Some(&repo));
    state.pane = LeftPane::Files;
    state.explorer_state.select(Some(0));

    state.select_first(Some(&repo), 10, 10);
    let first_generation = state.preview_content_generation;
    state.select_first(Some(&repo), 10, 10);
    state.move_selection(Some(&repo), -1, 10, 10);
    assert_eq!(state.preview_content_generation, first_generation);

    state.select_last(Some(&repo), 10, 10);
    let last_generation = state.preview_content_generation;
    state.select_last(Some(&repo), 10, 10);
    state.move_selection(Some(&repo), 1, 10, 10);
    assert_eq!(state.preview_content_generation, last_generation);
}

#[test]
fn repository_refresh_preserves_an_active_branch_comparison() {
    let mut repo = repository_data();
    repo.branches = vec![
        Branch {
            name: "main".to_owned(),
            remote: false,
            current: true,
            default: true,
        },
        Branch {
            name: "topic".to_owned(),
            remote: false,
            current: false,
            default: false,
        },
    ];
    let mut state = ChangesState::new(Some(&repo));
    state.preview_branch_diff(
        &repo.root,
        "main".to_owned(),
        "topic".to_owned(),
        "refs/heads/main".to_owned(),
        "refs/heads/topic".to_owned(),
    );

    let selection = state.capture_selection(&repo);
    state.restore_selection(&repo, selection, InventoryRefresh::All);

    assert_eq!(
        state
            .branch_comparison()
            .map(|comparison| (comparison.current.as_str(), comparison.target.as_str())),
        Some(("main", "topic"))
    );

    let stale_selection = state.capture_selection(&repo);
    state.refresh_diff(Some(&repo));
    state.restore_selection(&repo, stale_selection, InventoryRefresh::All);
    assert!(state.branch_comparison().is_none());
}

#[test]
fn worktree_only_refresh_keeps_explorer_rows_allocated() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("README.md"), "readme").unwrap();
    let mut repo = repository_data();
    repo.root = directory.path().to_owned();
    let mut state = ChangesState::new(Some(&repo));
    let rows = state.explorer_rows().as_ptr();
    let selection = state.capture_selection(&repo);

    repo.changes_fingerprint += 1;
    state.restore_selection(&repo, selection, InventoryRefresh::Unchanged);

    assert_eq!(state.explorer_rows().as_ptr(), rows);
}

#[test]
fn inventory_refresh_requests_only_loaded_affected_directories() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src")).unwrap();
    fs::create_dir_all(directory.path().join("docs")).unwrap();
    let mut repo = repository_data();
    repo.root = directory.path().to_owned();
    let mut state = ChangesState::new(Some(&repo));
    let tree = state.file_tree.as_mut().unwrap();
    let _ = tree.replace_directory("src".into(), Vec::new());
    let _ = tree.replace_directory("docs".into(), Vec::new());

    state.refresh_explorer_directories(
        &repo,
        InventoryRefresh::Directories(vec!["src".into(), "unloaded".into()]),
    );

    assert_eq!(
        state.loading_directories,
        HashSet::from([RepoPath::from("src")])
    );
}

#[test]
fn owns_semantic_worktree_target_transitions() {
    let repo = repository_data();
    let mut state = ChangesState::new(Some(&repo));
    let file_row = state
        .worktree_rows(&repo)
        .iter()
        .position(|row| row.change_index.is_some())
        .unwrap();

    assert_eq!(
        state.activate_target(state.worktree_row_target(file_row), &repo),
        Some(ChangesEffect::WorktreeFileSelected {
            path: "src/main.rs".into(),
            staged: false,
        })
    );
    assert_eq!(state.worktree_state.selected(), Some(file_row));
    assert_eq!(
        state.stage_target(state.worktree_row_target(file_row), &repo),
        Some(ChangesEffect::ToggleSelectedStage)
    );

    let stale_file_target = state.worktree_row_target(file_row);
    let directory_row = state
        .worktree_rows(&repo)
        .iter()
        .position(|row| row.directory_path.is_some())
        .unwrap();
    assert_eq!(
        state.activate_target(state.worktree_row_target(directory_row), &repo),
        Some(ChangesEffect::WorktreeDirectoryActivated)
    );
    assert_eq!(state.activate_target(stale_file_target, &repo), None);

    state.set_diff("@@ -1 +1 @@\n-old\n+new\n".to_owned());
    let stale_hunk_target = state.hunk_action_target(0);
    state.set_diff("Loading preview…".to_owned());
    assert_eq!(state.activate_target(stale_hunk_target, &repo), None);

    assert_eq!(
        state.activate_target(ChangesHitTarget::FilesTab, &repo),
        Some(ChangesEffect::SidebarPaneActivated)
    );
    assert_eq!(state.pane, LeftPane::Files);
    assert_eq!(
        state.activate_target(ChangesHitTarget::WorktreeTab, &repo),
        Some(ChangesEffect::SidebarPaneActivated)
    );
    assert_eq!(state.pane, LeftPane::Worktree);
}

#[test]
fn remembers_independent_markdown_source_and_preview_scrolls() {
    let mut state = ChangesState::new(None);
    state.diff_scroll = 80;

    state.toggle_markdown_rendered();
    assert!(state.markdown_rendered);
    assert_eq!(state.diff_scroll, 80);

    state.diff_scroll = 12;
    state.toggle_markdown_rendered();
    assert!(!state.markdown_rendered);
    assert_eq!(state.diff_scroll, 80);
    state.toggle_markdown_rendered();
    assert_eq!(state.diff_scroll, 12);

    state.refresh_diff(None);
    state.diff_scroll = 5;
    state.toggle_markdown_rendered();
    assert_eq!(state.diff_scroll, 5);
}
