use super::*;

#[test]
fn sums_change_line_counts() {
    let changes = [
        Change {
            path: RepoPath::from("staged.txt"),
            original_path: None,
            code: 'M',
            staged: true,
            additions: 4,
            deletions: 2,
        },
        Change {
            path: RepoPath::from("unstaged.txt"),
            original_path: None,
            code: 'M',
            staged: false,
            additions: 3,
            deletions: 5,
        },
    ];

    assert_eq!(change_line_counts(&changes), (7, 7));
}

#[test]
fn parses_main_worktree_with_spaces() {
    let parsed = parse_worktrees(
        b"worktree /repo/main worktree\0HEAD 0123456789abcdef\0branch refs/heads/main\0\0",
    )
    .unwrap();

    assert_eq!(
        parsed,
        [LinkedWorktree {
            path: PathBuf::from("/repo/main worktree"),
            head: Some("0123456789abcdef".to_owned()),
            branch: Some("refs/heads/main".to_owned()),
            is_main: true,
            is_detached: false,
            is_bare: false,
            locked: false,
            locked_reason: None,
            prunable: false,
            prunable_reason: None,
        }]
    );
}

#[test]
fn parses_detached_locked_and_prunable_worktrees() {
    let parsed = parse_worktrees(
            b"worktree /repo\0HEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0branch refs/heads/main\0\0worktree /repo/detached\0HEAD bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\0detached\0locked reason with spaces\0prunable gitdir file points to non-existent location\0\0",
        )
        .unwrap();

    assert_eq!(parsed.len(), 2);
    assert!(parsed[0].is_main);
    assert!(!parsed[1].is_main);
    assert!(parsed[1].is_detached);
    assert_eq!(parsed[1].branch, None);
    assert!(parsed[1].locked);
    assert_eq!(
        parsed[1].locked_reason.as_deref(),
        Some("reason with spaces")
    );
    assert!(parsed[1].prunable);
    assert_eq!(
        parsed[1].prunable_reason.as_deref(),
        Some("gitdir file points to non-existent location")
    );
}

#[test]
fn parses_bare_and_reasonless_locked_worktrees() {
    let parsed = parse_worktrees(
        b"worktree /srv/repository.git\0bare\0locked\0prunable metadata missing\0\0",
    )
    .unwrap();

    assert!(parsed[0].is_main);
    assert!(parsed[0].is_bare);
    assert_eq!(parsed[0].head, None);
    assert_eq!(parsed[0].branch, None);
    assert!(parsed[0].locked);
    assert_eq!(parsed[0].locked_reason, None);
    assert_eq!(
        parsed[0].prunable_reason.as_deref(),
        Some("metadata missing")
    );
}

#[test]
fn rejects_malformed_worktree_records() {
    for malformed in [
        b"HEAD abc\0branch refs/heads/main\0\0".as_slice(),
        b"worktree /repo\0HEAD abc\0branch refs/heads/main\0".as_slice(),
        b"worktree /repo\0HEAD abc\0\0".as_slice(),
        b"worktree /repo\0HEAD abc\0branch refs/heads/main\0detached\0\0".as_slice(),
        b"worktree /repo.git\0bare\0HEAD abc\0detached\0\0".as_slice(),
    ] {
        assert!(parse_worktrees(malformed).is_err(), "{malformed:?}");
    }
}

#[cfg(unix)]
#[test]
fn preserves_non_utf8_worktree_paths() {
    use std::os::unix::ffi::OsStrExt;

    let parsed =
        parse_worktrees(b"worktree /repo/linked-\xff\0HEAD 0123456789abcdef\0detached\0\0")
            .unwrap();

    assert_eq!(parsed[0].path.as_os_str().as_bytes(), b"/repo/linked-\xff");
}

#[test]
fn lists_a_real_linked_worktree() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main repository");
    let linked = directory.path().join("linked worktree");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Test Author"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "-m", "base"]);
    git(
        &root,
        &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    );

    let common = common_git_dir(&root).unwrap();
    assert_eq!(common_git_dir(&linked).unwrap(), common);
    let listed = list_worktrees(&common).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].path, fs::canonicalize(&root).unwrap());
    assert!(listed[0].is_main);
    assert_eq!(listed[0].branch.as_deref(), Some("refs/heads/main"));
    assert_eq!(listed[1].path, fs::canonicalize(&linked).unwrap());
    assert!(!listed[1].is_main);
    assert_eq!(listed[1].branch.as_deref(), Some("refs/heads/topic"));
}

#[test]
fn creates_a_linked_worktree_without_a_runtime_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main repository");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Test Author"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "-m", "base"]);

    let storage = directory.path().join("data/hunkle/worktrees");
    let created = create_worktree_in(&root, "feature/direct", "main", &storage).unwrap();

    assert_eq!(created.parent().unwrap().parent().unwrap(), storage);
    let repository_directory = created
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy();
    assert!(repository_directory.starts_with("main repository-"));
    assert_eq!(repository_directory.len(), "main repository-".len() + 5);
    assert_eq!(created.file_name().unwrap(), "feature-direct");
    assert_eq!(
        fs::read_to_string(created.join("tracked.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        list_worktrees(&root).unwrap()[1].branch.as_deref(),
        Some("refs/heads/feature/direct")
    );
}

#[test]
fn worktree_removal_refuses_uncommitted_changes_and_never_forces_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main repository");
    let linked = directory.path().join("linked worktree");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Test Author"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "-m", "base"]);
    git(
        &root,
        &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    );

    fs::write(linked.join("uncommitted.txt"), "keep me\n").unwrap();
    assert!(remove_worktree(&root, &linked).is_err());
    assert!(linked.exists());
    assert_eq!(list_worktrees(&root).unwrap().len(), 2);

    fs::remove_file(linked.join("uncommitted.txt")).unwrap();
    remove_worktree(&root, &linked).unwrap();
    assert!(!linked.exists());
    assert_eq!(list_worktrees(&root).unwrap().len(), 1);
}

#[test]
fn parses_staged_and_unstaged_status_entries() {
    let parsed = parse_status(b"M  staged.rs\0 M changed.rs\0?? new.rs\0MM both.rs\0").unwrap();
    assert_eq!(parsed.len(), 5);
    assert!(
        parsed
            .iter()
            .any(|change| change.path == "staged.rs" && change.staged)
    );
    assert!(
        parsed
            .iter()
            .any(|change| change.path == "new.rs" && !change.staged)
    );
    assert_eq!(
        parsed
            .iter()
            .filter(|change| change.path == "both.rs")
            .count(),
        2
    );
}

#[test]
fn preserves_both_paths_for_renames() {
    let parsed = parse_status(b"R  new.rs\0old.rs\0").unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "new.rs");
    assert_eq!(parsed[0].original_path.as_ref().unwrap(), "old.rs");
}

#[test]
fn parses_branch_tracking_divergence() {
    let sync =
        parse_branch_sync(b"## feature...origin/feature [ahead 12, behind 3]\0 M tracked.txt\0");

    assert_eq!(sync.ahead, 12);
    assert_eq!(sync.behind, 3);
    assert_eq!(parse_branch_sync(b"## feature\0").ahead, 0);
    assert_eq!(parse_branch_sync(b"## HEAD (no branch)\0").behind, 0);
}

#[test]
fn untracked_line_counts_respect_the_read_budget() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new.txt");
    fs::write(&path, "a\nb\n").unwrap();

    assert_eq!(count_file_lines(&path, 4).unwrap(), (2, 4));
    assert_eq!(count_file_lines(&path, 3).unwrap(), (0, 0));
}

#[test]
fn section_diffs_include_every_file_in_the_selected_section() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Section Test"]);
    git(root, &["config", "user.email", "section@example.com"]);
    fs::write(root.join("staged.txt"), "before\n").unwrap();
    fs::write(root.join("unstaged.txt"), "before\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    fs::write(root.join("staged.txt"), "after\n").unwrap();
    fs::write(root.join("unstaged.txt"), "after\n").unwrap();
    fs::write(root.join("untracked.txt"), "new\n").unwrap();
    git(root, &["add", "staged.txt"]);
    let (changes, _, _) = status(root).unwrap();

    let staged = section_diff(root, &changes, true).unwrap();
    assert!(staged.contains("diff --git a/staged.txt b/staged.txt"));
    assert!(!staged.contains("unstaged.txt"));
    assert!(!staged.contains("untracked.txt"));

    let unstaged = section_diff(root, &changes, false).unwrap();
    assert!(unstaged.contains("diff --git a/unstaged.txt b/unstaged.txt"));
    assert!(unstaged.contains("untracked.txt"));
    assert_eq!(unstaged.matches("diff --git").count(), 2);
}

#[test]
fn branch_diffs_show_changes_unique_to_the_current_branch() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Branch Diff Test"]);
    git(root, &["config", "user.email", "branch-diff@example.com"]);
    fs::write(root.join("shared.txt"), "base\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["switch", "-c", "feature"]);
    fs::write(root.join("feature.txt"), "feature only\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "feature"]);
    git(root, &["switch", "main"]);
    fs::write(root.join("target.txt"), "target only\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "target"]);
    git(root, &["switch", "feature"]);
    git(root, &["tag", "main", "feature"]);
    fs::write(root.join("feature.txt"), "feature only\nunstaged\n").unwrap();
    fs::write(root.join("staged.txt"), "staged\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
    git(root, &["add", "staged.txt"]);

    let preview = branch_diff(root, "refs/heads/main", "refs/heads/feature").unwrap();

    assert!(preview.contains("diff --git a/feature.txt b/feature.txt"));
    assert!(preview.contains("+feature only"));
    assert!(preview.contains("+unstaged"));
    assert!(preview.contains("diff --git a/staged.txt b/staged.txt"));
    assert!(preview.contains("+staged"));
    assert!(preview.contains("diff --git \"a/untracked.txt\" \"b/untracked.txt\""));
    assert!(preview.contains("+untracked"));
    assert!(!preview.contains("target.txt"));
    assert_eq!(branch_name(root).unwrap(), "feature");
}

#[cfg(unix)]
#[test]
fn untracked_symlink_preview_does_not_read_its_target() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "outside secret").unwrap();
    symlink(outside.path(), workspace.path().join("link")).unwrap();
    let change = Change {
        path: RepoPath::from("link"),
        original_path: None,
        code: '?',
        staged: false,
        additions: 0,
        deletions: 0,
    };

    let preview = diff(workspace.path(), &change).unwrap();

    assert!(preview.contains("Untracked symbolic link"));
    assert!(!preview.contains("outside secret"));
    let section = section_diff(workspace.path(), &[change], false).unwrap();
    assert!(section.contains("diff --git \"a/link\" \"b/link\""));
    assert!(section.contains("Untracked symbolic link"));
    assert!(!section.contains("outside secret"));
}

#[cfg(unix)]
#[test]
fn special_files_are_rejected_before_opening() {
    let workspace = tempfile::tempdir().unwrap();
    let fifo = workspace.path().join("pipe");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let path = RepoPath::from("pipe");
    let change = Change {
        path: path.clone(),
        original_path: None,
        code: '?',
        staged: false,
        additions: 0,
        deletions: 0,
    };

    assert!(
        file_content(workspace.path(), &path)
            .unwrap()
            .contains("special file")
    );
    assert!(
        diff(workspace.path(), &change)
            .unwrap()
            .contains("Untracked special file")
    );
    assert!(
        section_diff(workspace.path(), &[change], false)
            .unwrap()
            .contains("Untracked special file")
    );
    assert!(count_file_lines(&fifo, u64::MAX).is_err());
}

#[test]
fn does_not_climb_to_an_enclosing_repository() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    let nested = root.join("nested/config");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("settings.toml"), "theme = 'test'\n").unwrap();

    assert_eq!(discover(root).unwrap(), fs::canonicalize(root).unwrap());
    let error = discover(&nested).unwrap_err().to_string();
    assert!(error.contains("not a repository root"));
    assert!(error.contains(&fs::canonicalize(root).unwrap().display().to_string()));
    assert!(load(&nested).is_err());

    let workspace = load_or_local(&nested).unwrap();
    assert_eq!(workspace.kind, RepositoryKind::Local);
    assert_eq!(workspace.root, fs::canonicalize(&nested).unwrap());
    assert_eq!(workspace.branch, "local");
    assert_eq!(workspace.files, ["settings.toml"]);
    assert!(workspace.changes.is_empty());
    assert!(workspace.history.is_empty());
    assert!(workspace.commits.is_empty());
}

#[test]
fn loads_a_plain_directory_as_a_local_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src/components")).unwrap();
    fs::write(root.join("README.md"), "local\n").unwrap();
    fs::write(root.join("src/components/card.rs"), "component\n").unwrap();

    let workspace = load_or_local(root).unwrap();
    assert_eq!(workspace.kind, RepositoryKind::Local);
    assert_eq!(workspace.root, fs::canonicalize(root).unwrap());
    assert_eq!(workspace.branch, "local");
    assert_eq!(workspace.files, ["README.md", "src/components/card.rs"]);
    assert!(workspace.changes.is_empty());
    assert!(workspace.history.is_empty());
    assert!(workspace.commits.is_empty());
}

#[test]
fn bootstrap_defers_repository_facets_until_full_refresh() {
    let git_directory = tempfile::tempdir().unwrap();
    let git_root = git_directory.path();
    git(git_root, &["init", "-b", "main"]);
    git(git_root, &["config", "user.name", "Test Author"]);
    git(git_root, &["config", "user.email", "test@example.com"]);
    fs::write(git_root.join("tracked.txt"), "tracked\n").unwrap();
    git(git_root, &["add", "tracked.txt"]);
    git(git_root, &["commit", "-m", "initial"]);

    let mut repository = bootstrap_or_local(git_root).unwrap();
    assert_eq!(repository.kind, RepositoryKind::Git);
    assert!(repository.common_dir.is_some());
    assert!(!repository.details_ready);
    assert!(repository.files.is_empty());
    assert!(repository.history.is_empty());
    assert!(repository.commits.is_empty());

    let update = refresh_repository(&repository.root, repository.kind, RefreshScope::ALL).unwrap();
    repository.apply(update);
    assert!(repository.details_ready);
    assert_eq!(repository.branch, "main");
    assert_eq!(repository.files, ["tracked.txt"]);
    assert_eq!(repository.history.len(), 1);
    assert_eq!(repository.commits.len(), 1);

    let local_directory = tempfile::tempdir().unwrap();
    fs::write(local_directory.path().join("local.txt"), "local\n").unwrap();
    let mut workspace = bootstrap_or_local(local_directory.path()).unwrap();
    assert_eq!(workspace.kind, RepositoryKind::Local);
    assert!(!workspace.details_ready);
    assert!(workspace.files.is_empty());

    let update = refresh_repository(&workspace.root, workspace.kind, RefreshScope::ALL).unwrap();
    workspace.apply(update);
    assert!(workspace.details_ready);
    assert_eq!(workspace.files, ["local.txt"]);
}

#[test]
fn parses_complete_multiline_commit_messages() {
    let commits = parse_log(
            b"abc\0parent\0HEAD -> main\0Ada\x002026-08-07 18:29\0Subject\0Subject\n\nBody \x1f line\n\nFinal \x1e note\0",
        );

    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].date, "07 Aug 18:29");
    assert_eq!(commits[0].subject, "Subject");
    assert_eq!(
        commits[0].message,
        "Subject\n\nBody \u{1f} line\n\nFinal \u{1e} note"
    );
}

#[test]
fn loads_a_real_repository_with_a_merge_and_worktree_change() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("base.txt"), "base\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["checkout", "-b", "feature"]);
    fs::write(root.join("feature.txt"), "feature\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "feature"]);
    git(root, &["checkout", "main"]);
    fs::write(root.join("main.txt"), "main\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "main work"]);
    git(
        root,
        &["merge", "--no-ff", "feature", "-m", "merge feature"],
    );
    fs::write(root.join("main.txt"), "changed\n").unwrap();
    fs::create_dir(root.join("ignored")).unwrap();
    fs::write(root.join("ignored/cache.txt"), "generated\n").unwrap();

    let repo = load(root).unwrap();
    assert_eq!(repo.branch, "main");
    assert_eq!(
        repo.branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.current))
            .collect::<Vec<_>>(),
        [("main", true), ("feature", false)]
    );
    assert_eq!(repo.commits.len(), 4);
    assert_eq!(repo.history.len(), 4);
    assert_eq!(repo.commits[0].date.len(), 12);
    assert_eq!(repo.commits[0].date.as_bytes().get(2), Some(&b' '));
    assert_eq!(repo.commits[0].date.as_bytes().get(6), Some(&b' '));
    assert_eq!(repo.commits[0].date.as_bytes().get(9), Some(&b':'));
    assert!(
        repo.history[0]
            .refs
            .iter()
            .any(|name| name.contains("HEAD"))
    );
    assert_eq!(repo.commits[0].parents.len(), 2);
    assert!(repo.commits[0].graph.iter().any(|cell| cell.symbol == '─'));
    assert_eq!(repo.changes.len(), 1);
    assert_eq!(repo.changes[0].path, "main.txt");
    assert_eq!(
        (repo.changes[0].additions, repo.changes[0].deletions),
        (1, 1)
    );
    assert!(repo.files.iter().any(|path| path == "base.txt"));
    assert!(repo.files.iter().any(|path| path == "feature.txt"));
    assert!(repo.files.iter().any(|path| path == "ignored/cache.txt"));
    assert!(
        !repo
            .changes
            .iter()
            .any(|change| change.path == "ignored/cache.txt")
    );
    assert_eq!(
        file_content(root, &RepoPath::from("main.txt")).unwrap(),
        "changed\n"
    );
    let selected_commit_diff = commit_diff(root, &repo.history[0].oid).unwrap();
    assert!(selected_commit_diff.contains("diff --git"));

    stage(root, &repo.changes[0]).unwrap();
    let staged = load(root).unwrap();
    assert!(staged.changes[0].staged);
    assert_eq!(
        (staged.changes[0].additions, staged.changes[0].deletions),
        (1, 1)
    );

    unstage(root, &staged.changes[0]).unwrap();
    let unstaged = load(root).unwrap();
    assert!(!unstaged.changes[0].staged);

    stage(root, &unstaged.changes[0]).unwrap();
    let output = super::commit(root, "update main").unwrap();
    assert!(output.success, "{}", output.stderr);
    let committed = load(root).unwrap();
    assert!(committed.changes.is_empty());
    assert_eq!(committed.commits.len(), 5);
    assert_eq!(committed.history.len(), 5);

    let fetched = super::fetch(root).unwrap();
    assert!(fetched.success, "{}", fetched.stderr);
}

#[cfg(unix)]
#[test]
fn preserves_invalid_utf8_inventory_status_and_whole_file_operations() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);

    let first_name = OsString::from_vec(b"collision-\x80.txt".to_vec());
    let second_name = OsString::from_vec(b"collision-\x81.txt".to_vec());
    let first_path = RepoPath::from(PathBuf::from(&first_name));
    let second_path = RepoPath::from(PathBuf::from(&second_name));
    assert_eq!(first_name.to_string_lossy(), second_name.to_string_lossy());

    fs::write(root.join(&first_name), "first original\n").unwrap();
    fs::write(root.join(&second_name), "second original\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "invalid byte paths"]);
    fs::write(root.join(&first_name), "first changed\n").unwrap();
    fs::write(root.join(&second_name), "second changed\n").unwrap();

    let repo = load(root).unwrap();
    assert!(repo.files.contains(&first_path));
    assert!(repo.files.contains(&second_path));
    let first = repo
        .changes
        .iter()
        .find(|change| change.path == first_path)
        .unwrap()
        .clone();
    let second = repo
        .changes
        .iter()
        .find(|change| change.path == second_path)
        .unwrap();
    assert_ne!(first.path, second.path);
    let first_diff = diff(root, &first).unwrap();
    assert!(first_diff.contains("first changed"));
    assert!(!first_diff.contains("second changed"));

    stage(root, &first).unwrap();
    let staged = load(root).unwrap();
    assert!(
        staged
            .changes
            .iter()
            .any(|change| change.path == first_path && change.staged)
    );
    assert!(
        staged
            .changes
            .iter()
            .any(|change| change.path == second_path && !change.staged)
    );

    let staged_first = staged
        .changes
        .iter()
        .find(|change| change.path == first_path && change.staged)
        .unwrap();
    unstage(root, staged_first).unwrap();
    let unstaged = load(root).unwrap();
    let unstaged_first = unstaged
        .changes
        .iter()
        .find(|change| change.path == first_path && !change.staged)
        .unwrap()
        .clone();
    discard_unstaged(root, &unstaged_first).unwrap();

    assert_eq!(
        fs::read(root.join(&first_name)).unwrap(),
        b"first original\n"
    );
    assert_eq!(
        fs::read(root.join(&second_name)).unwrap(),
        b"second changed\n"
    );
}

#[test]
fn git_files_include_untracked_and_ignored_files_but_exclude_deleted_tracked_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    fs::write(root.join("untracked.txt"), "new\n").unwrap();
    fs::create_dir_all(root.join("empty/nested")).unwrap();
    fs::create_dir_all(root.join("empty/ignored")).unwrap();
    fs::create_dir(root.join("config")).unwrap();
    fs::create_dir(root.join("logs")).unwrap();
    fs::write(root.join("logs/debug.log"), "debug\n").unwrap();
    fs::write(
        root.join(".gitignore"),
        "empty/ignored/\n.env*\nconfig/\nlogs/*.log\n",
    )
    .unwrap();
    fs::write(root.join(".env"), "SECRET=value\n").unwrap();
    fs::write(root.join(".env.local"), "SECRET=local\n").unwrap();
    fs::write(root.join(".envrc"), "not an env file\n").unwrap();
    fs::write(root.join("config/.env.production"), "SECRET=prod\n").unwrap();
    fs::remove_file(root.join("tracked.txt")).unwrap();

    let (files, directories, ignored_files, truncated) = inventory::git_entries(root).unwrap();
    assert!(!truncated);
    assert_eq!(
        files,
        [
            ".env",
            ".env.local",
            ".envrc",
            ".gitignore",
            "config/.env.production",
            "logs/debug.log",
            "untracked.txt"
        ]
    );
    assert_eq!(
        directories,
        ["config", "empty", "empty/ignored", "empty/nested", "logs"]
    );
    assert_eq!(
        ignored_files,
        [
            ".env",
            ".env.local",
            ".envrc",
            "config/.env.production",
            "logs/debug.log"
        ]
    );
}

#[test]
fn gitlinks_are_exposed_as_directories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked"), "content").unwrap();
    git(root, &["add", "tracked"]);
    git(root, &["commit", "-m", "initial"]);
    let oid_output = run(root, &["rev-parse", "HEAD"]).unwrap();
    let oid = String::from_utf8_lossy(&oid_output.stdout);
    let cache_info = format!("160000,{},{path}", oid.trim(), path = "module");
    git(root, &["update-index", "--add", "--cacheinfo", &cache_info]);
    fs::create_dir(root.join("module")).unwrap();

    let (files, directories, _ignored_files, truncated) = inventory::git_entries(root).unwrap();
    assert!(!truncated);
    assert!(!files.iter().any(|path| path == "module"));
    assert!(directories.iter().any(|path| path == "module"));
}

#[test]
fn git_files_exclude_absent_sparse_checkout_entries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    fs::write(root.join("sparse.txt"), "tracked\n").unwrap();
    git(root, &["add", "sparse.txt"]);
    git(root, &["update-index", "--skip-worktree", "sparse.txt"]);
    fs::remove_file(root.join("sparse.txt")).unwrap();

    assert!(inventory::git_entries(root).unwrap().0.is_empty());
}

#[test]
fn truncates_large_untracked_previews() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("large.txt"), vec![b'x'; 256 * 1024]).unwrap();
    let change = Change {
        path: "large.txt".into(),
        original_path: None,
        code: '?',
        staged: false,
        additions: 0,
        deletions: 0,
    };

    let preview = diff(root, &change).unwrap();
    assert!(preview.contains("Preview truncated; file is 262144 bytes"));
    assert!(preview.len() < 140 * 1024);
}

#[test]
fn scoped_refresh_updates_only_requested_facets() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-m", "base"]);

    let mut repository = load(root).unwrap();
    let original_commit = repository.commits[0].oid.clone();
    let original_files = repository.files.clone();
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();
    fs::write(root.join("new.txt"), "new\n").unwrap();

    let update = refresh_repository(
        &repository.root,
        RepositoryKind::Git,
        RefreshScope::WORKTREE,
    )
    .unwrap();
    assert!(update.worktree.is_some());
    assert!(update.inventory.is_none());
    assert!(update.history.is_none());
    assert!(update.graph.is_none());
    assert!(update.refs.is_none());
    repository.apply(update);
    assert_eq!(repository.files, original_files);
    assert_eq!(repository.commits[0].oid, original_commit);
    assert!(
        repository
            .changes
            .iter()
            .any(|change| change.path == "tracked.txt")
    );

    let update = refresh_repository(
        &repository.root,
        RepositoryKind::Git,
        RefreshScope::WORKTREE_AND_INVENTORY,
    )
    .unwrap();
    assert!(update.worktree.is_some());
    assert!(update.inventory.is_some());
    repository.apply(update);
    assert!(repository.files.iter().any(|file| file == "new.txt"));
    assert_eq!(repository.commits[0].oid, original_commit);
}

#[test]
fn refresh_can_reuse_an_already_parsed_worktree_status() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-m", "base"]);

    let status = worktree_status(root).unwrap();
    fs::write(root.join("after-status.txt"), "new\n").unwrap();
    let cached = refresh_repository_with_status(
        root,
        RepositoryKind::Git,
        RefreshScope::WORKTREE,
        Some(status),
    )
    .unwrap();
    assert!(cached.worktree.unwrap().changes.is_empty());

    let fresh = refresh_repository(root, RepositoryKind::Git, RefreshScope::WORKTREE).unwrap();
    assert!(
        fresh
            .worktree
            .unwrap()
            .changes
            .iter()
            .any(|change| change.path == "after-status.txt")
    );
}

#[test]
fn external_content_changes_do_not_refresh_the_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-m", "base"]);

    let clean = worktree_signature(root).unwrap();
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();
    let modified = worktree_signature(root).unwrap();
    assert_eq!(modified.refresh_scope_since(clean), RefreshScope::WORKTREE);
}

#[test]
fn external_path_changes_refresh_the_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);

    let clean = worktree_signature(root).unwrap();
    fs::write(root.join("new.txt"), "new\n").unwrap();
    let added = worktree_signature(root).unwrap();
    assert_eq!(
        added.refresh_scope_since(clean),
        RefreshScope::WORKTREE_AND_INVENTORY
    );
}

#[test]
fn inventory_changes_identify_only_affected_parent_directories() {
    let mut directories = HashSet::new();
    collect_changed_parents(
        &["README.md".into(), "src/old.rs".into()],
        &["README.md".into(), "src/new.rs".into()],
        &mut directories,
    );

    assert_eq!(directories, HashSet::from([RepoPath::from("src")]));
}

#[test]
fn parses_batched_commit_change_summaries() {
    let summaries = parse_commit_summaries(
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0\0\n12\t3\tsrc/app.rs\0-\t-\tassets/logo\x1e.png\0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\0\0\n4\t0\tREADME.md\0",
        )
        .unwrap();

    assert_eq!(
        summaries["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        DiffSummary {
            files: vec!["src/app.rs".into(), "assets/logo\u{1e}.png".into()],
            files_truncated: false,
            additions: 12,
            deletions: 3,
        }
    );
    assert_eq!(
        summaries["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
        DiffSummary {
            files: vec!["README.md".into()],
            files_truncated: false,
            additions: 4,
            deletions: 0,
        }
    );
}

#[test]
fn bounds_paths_retained_by_commit_summaries() {
    let mut output = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0\0\n".to_vec();
    for index in 0..=2_000 {
        output.extend_from_slice(format!("1\t2\tfile-{index}\0").as_bytes());
    }

    let summaries = parse_commit_summaries(&output).unwrap();
    let summary = &summaries["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"];
    assert_eq!(summary.files.len(), 2_000);
    assert!(summary.files_truncated);
    assert_eq!(summary.additions, 2_001);
    assert_eq!(summary.deletions, 4_002);
}

#[test]
fn stages_only_the_selected_hunk() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    let original = (1..=20)
        .map(|line| format!("line {line:02}"))
        .collect::<Vec<_>>();
    fs::write(root.join("split.txt"), original.join("\n") + "\n").unwrap();
    git(root, &["add", "split.txt"]);
    git(root, &["commit", "-m", "base"]);

    let mut changed = original;
    changed[1] = "changed first".to_owned();
    changed[18] = "changed second".to_owned();
    fs::write(root.join("split.txt"), changed.join("\n") + "\n").unwrap();
    let change = load(root).unwrap().changes.remove(0);
    let patch = diff(root, &change).unwrap();
    assert_eq!(
        patch.lines().filter(|line| line.starts_with("@@")).count(),
        2
    );

    stage_hunk(root, &patch, 0).unwrap();

    let staged = String::from_utf8(
        run(root, &["diff", "--cached", "--", "split.txt"])
            .unwrap()
            .stdout,
    )
    .unwrap();
    let unstaged =
        String::from_utf8(run(root, &["diff", "--", "split.txt"]).unwrap().stdout).unwrap();
    assert!(staged.contains("changed first"));
    assert!(!staged.contains("changed second"));
    assert!(!unstaged.contains("changed first"));
    assert!(unstaged.contains("changed second"));
    let changes = load(root).unwrap().changes;
    assert!(
        changes
            .iter()
            .any(|change| change.path == "split.txt" && change.staged)
    );
    assert!(
        changes
            .iter()
            .any(|change| change.path == "split.txt" && !change.staged)
    );
}

#[test]
fn discards_only_selected_unstaged_changes_and_preserves_the_index() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    fs::write(root.join("other.txt"), "other base\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);

    fs::write(root.join("tracked.txt"), "staged\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    fs::write(root.join("tracked.txt"), "unstaged\n").unwrap();
    fs::write(root.join("other.txt"), "other unstaged\n").unwrap();
    let change = load(root)
        .unwrap()
        .changes
        .into_iter()
        .find(|change| change.path == "tracked.txt" && !change.staged)
        .unwrap();

    discard_unstaged(root, &change).unwrap();

    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "staged\n"
    );
    assert_eq!(
        String::from_utf8(run(root, &["show", ":tracked.txt"]).unwrap().stdout).unwrap(),
        "staged\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("other.txt")).unwrap(),
        "other unstaged\n"
    );
    let changes = load(root).unwrap().changes;
    assert_eq!(
        changes
            .iter()
            .filter(|change| change.path == "tracked.txt")
            .count(),
        1
    );
    assert!(
        changes
            .iter()
            .any(|change| change.path == "tracked.txt" && change.staged)
    );
    assert!(
        changes
            .iter()
            .any(|change| change.path == "other.txt" && !change.staged)
    );
}

#[test]
fn discards_untracked_files_and_restores_deleted_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    fs::remove_file(root.join("tracked.txt")).unwrap();
    fs::write(root.join("remove.txt"), "remove\n").unwrap();
    fs::write(root.join("keep.txt"), "keep\n").unwrap();
    let changes = load(root).unwrap().changes;

    let deleted = changes
        .iter()
        .find(|change| change.path == "tracked.txt" && !change.staged)
        .unwrap();
    discard_unstaged(root, deleted).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "tracked\n"
    );

    let untracked = changes
        .iter()
        .find(|change| change.path == "remove.txt" && !change.staged)
        .unwrap();
    discard_unstaged(root, untracked).unwrap();
    assert!(!root.join("remove.txt").exists());
    assert!(root.join("keep.txt").exists());
}

#[test]
fn discards_an_unstaged_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("old.txt"), "content\n").unwrap();
    git(root, &["add", "old.txt"]);
    git(root, &["commit", "-m", "base"]);
    fs::rename(root.join("old.txt"), root.join("new.txt")).unwrap();
    let change = Change {
        path: "new.txt".into(),
        original_path: Some("old.txt".into()),
        code: 'R',
        staged: false,
        additions: 0,
        deletions: 0,
    };

    discard_unstaged(root, &change).unwrap();

    assert_eq!(
        fs::read_to_string(root.join("old.txt")).unwrap(),
        "content\n"
    );
    assert!(!root.join("new.txt").exists());
    assert!(load(root).unwrap().changes.is_empty());
}

#[test]
fn refuses_to_discard_an_unresolved_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("conflict.txt"), "base\n").unwrap();
    git(root, &["add", "conflict.txt"]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["switch", "-c", "side"]);
    fs::write(root.join("conflict.txt"), "side\n").unwrap();
    git(root, &["commit", "-am", "side"]);
    git(root, &["switch", "main"]);
    fs::write(root.join("conflict.txt"), "main\n").unwrap();
    git(root, &["commit", "-am", "main"]);
    let merge = run(root, &["merge", "side"]).unwrap();
    assert!(!merge.status.success());
    let before = fs::read_to_string(root.join("conflict.txt")).unwrap();
    let change = load(root)
        .unwrap()
        .changes
        .into_iter()
        .find(|change| change.path == "conflict.txt" && !change.staged)
        .unwrap();

    assert!(discard_unstaged(root, &change).is_err());
    assert_eq!(
        fs::read_to_string(root.join("conflict.txt")).unwrap(),
        before
    );
    assert!(
        !run(root, &["ls-files", "--unmerged"])
            .unwrap()
            .stdout
            .is_empty()
    );
}

#[test]
fn checks_out_local_and_remote_branches_without_overriding_git_safety() {
    let directory = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    let occupied = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test Author"]);
    git(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("tracked.txt"), "main\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-m", "initial"]);
    git(root, &["branch", "topic"]);

    let output = checkout_branch(root, "topic", false).unwrap();
    assert!(output.success, "{}", output.stderr);
    assert_eq!(branch_name(root).unwrap(), "topic");
    git(root, &["switch", "main"]);

    git(
        root,
        &[
            "worktree",
            "add",
            occupied.path().to_str().unwrap(),
            "topic",
        ],
    );
    let output = checkout_branch(root, "topic", false).unwrap();
    assert!(!output.success);
    assert_eq!(branch_name(root).unwrap(), "main");

    git(remote.path(), &["init", "--bare"]);
    git(
        root,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(root, &["branch", "remote-topic"]);
    git(root, &["push", "origin", "remote-topic"]);
    git(root, &["branch", "-D", "remote-topic"]);
    let output = checkout_branch(root, "origin/remote-topic", true).unwrap();
    assert!(output.success, "{}", output.stderr);
    assert_eq!(branch_name(root).unwrap(), "remote-topic");
    let upstream = run(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&upstream.stdout).trim(),
        "origin/remote-topic"
    );
}

#[cfg(test)]
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if args.first() == Some(&"init") {
        git(root, &["config", "core.autocrlf", "false"]);
    }
}
