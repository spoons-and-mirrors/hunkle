use std::{process::Command, thread};

use super::*;

#[test]
fn control_c_clears_a_raw_text_field_instead_of_quitting() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.mode = Mode::Command;
    app.actions.input = "draft command".to_owned();

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.actions.input.is_empty());
    assert!(!app.should_quit);
}

#[test]
fn control_c_still_quits_outside_a_text_field() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit);
}

#[test]
fn clearing_targets_removes_overlaps_but_keeps_adjacent_targets() {
    let mut regions = Regions::default();
    regions.register_hit_target(HitTarget::HeaderRepository, Rect::new(0, 0, 4, 1));
    regions.register_hit_target(HitTarget::RenderedPreviewToggle, Rect::new(3, 0, 4, 1));
    regions.register_hit_target(
        HitTarget::Graph(GraphHitTarget::AuthorHeader),
        Rect::new(7, 0, 2, 1),
    );
    regions.register_scroll_target(ScrollTarget::Preview, Rect::new(3, 0, 4, 1));
    regions.register_scroll_target(ScrollTarget::Graph, Rect::new(7, 0, 2, 1));

    regions.clear_targets_in(Rect::new(4, 0, 3, 1));

    assert!(
        regions
            .hit_target_rect(HitTarget::HeaderRepository)
            .is_some()
    );
    assert!(
        regions
            .hit_target_rect(HitTarget::RenderedPreviewToggle)
            .is_none()
    );
    assert!(
        regions
            .hit_target_rect(HitTarget::Graph(GraphHitTarget::AuthorHeader))
            .is_some()
    );
    assert_eq!(regions.scroll_target_at(Position::new(5, 0)), None);
    assert_eq!(
        regions.scroll_target_at(Position::new(7, 0)),
        Some(ScrollTarget::Graph)
    );
}

#[test]
fn opens_a_nested_directory_as_a_local_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let nested = root.join("nested/config");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("settings.toml"), "theme = 'test'\n").unwrap();

    let app = App::new(nested.clone());

    let repo = app.session.data().unwrap();
    assert!(repo.is_local());
    assert_eq!(repo.root, fs::canonicalize(&nested).unwrap());
    assert_eq!(repo.branch, "local");
    assert_eq!(repo.files, ["settings.toml"]);
    assert_eq!(app.changes.pane, LeftPane::Files);
}

#[test]
fn startup_file_opens_its_parent_workspace_and_selects_it() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    let file = nested.join("auth.json");
    fs::write(&file, "{}\n").unwrap();

    let mut app = App::opening(file);
    wait_for_state(&mut app, |app| !app.session.open_running());

    let repo = app.repository().unwrap().clone();
    assert_eq!(repo.root, fs::canonicalize(&nested).unwrap());
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(
        app.changes.selected_explorer_file_path(&repo),
        Some(&RepoPath::from("auth.json"))
    );
}

#[test]
fn workspace_open_errors_leave_the_current_workspace_active() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("file.txt"), "content\n").unwrap();
    let mut app = App::new(root.to_path_buf());

    app.open_repository(root.join("missing"));
    wait_for_state(&mut app, |app| !app.session.open_running());

    assert!(
        app.notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("Could not open workspace:"))
    );
    assert_eq!(
        app.repository().unwrap().root,
        fs::canonicalize(root).unwrap()
    );
}

#[test]
fn local_workspaces_reload_files_and_reject_git_actions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("one.txt"), "one\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    assert!(app.repository().unwrap().is_local());

    for key in ['x', 'g', 'c', 'u', 'b'] {
        app.mode = Mode::Normal;
        app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal, "{key}");
        assert_eq!(app.notice.as_deref(), Some("Not a Git repository"), "{key}");
    }

    fs::write(root.join("two.txt"), "two\n").unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.repository().unwrap().files == ["one.txt", "two.txt"]
    });
}

#[test]
fn local_workspaces_can_start_norm_tabs_from_their_root() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("notes.txt"), "notes\n").unwrap();
    let app = App::new(directory.path().to_path_buf());

    assert_eq!(
        app.norm_destination().unwrap(),
        fs::canonicalize(directory.path()).unwrap()
    );
}

#[test]
fn graph_visibility_survives_repository_refresh() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.notice.as_deref() != Some("Refreshing...")
    });
    assert_eq!(app.view(), View::Graph);
}

#[test]
fn general_settings_edit_and_persist() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let path = directory.path().join("config");
    let mut app = App::new(directory.path().to_path_buf());
    app.settings = Settings::default();
    app.settings_store = SettingsStore::at(path);
    app.mode = Mode::Settings;

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.settings.auto_fetch);
    assert_eq!(app.settings.fetch_interval_minutes, 6);
    assert!(!app.settings.format_on_save);
    assert_eq!(app.settings_store.load(), app.settings);
}

#[test]
fn shortcut_settings_rebind_reset_and_persist_commands() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    let mut app = App::new(directory.path().to_path_buf());
    app.settings_store = SettingsStore::at(path);
    app.mode = Mode::Settings;
    app.settings_state.page = SettingsPage::Shortcuts;
    app.settings_state.shortcut_selection = Shortcuts::definitions()
        .position(|definition| definition.action == ShortcutAction::OpenExplorer)
        .unwrap();

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    assert_eq!(
        app.settings.shortcuts.label(ShortcutAction::OpenExplorer),
        "Alt+v"
    );
    assert_eq!(app.settings_store.load(), app.settings);

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(
        app.settings.shortcuts.label(ShortcutAction::OpenExplorer),
        "o"
    );
}

#[cfg(unix)]
#[test]
fn norm_tab_switch_follows_the_active_workspace_directly() {
    fn presence(active_tab_id: u64, first: &Path, second: &Path) -> String {
        serde_json::json!({
            "Presence": {
                "version": 2,
                "daemon_epoch": "epoch-a",
                "revision": active_tab_id,
                "instances": [{
                    "instance_id": "terminal-a",
                    "active_tab_id": active_tab_id,
                    "tabs": [
                        { "tab_id": 1, "workspace": first },
                        { "tab_id": 2, "workspace": second }
                    ]
                }]
            }
        })
        .to_string()
    }

    let current = tempfile::tempdir().unwrap();
    let next = tempfile::tempdir().unwrap();
    initialize_repository(current.path());
    initialize_repository(next.path());
    let mut app = App::new(current.path().to_path_buf());

    app.norm_presence
        .set_snapshot_for_test(&presence(1, current.path(), next.path()));
    assert!(!app.follow_norm_workspace_changes());
    app.norm_presence
        .set_snapshot_for_test(&presence(2, current.path(), next.path()));
    assert!(app.follow_norm_workspace_changes());
    wait_for_state(&mut app, |app| !app.session.open_running());

    assert_eq!(
        app.repository().unwrap().root,
        fs::canonicalize(next.path()).unwrap()
    );
}

#[test]
fn restores_commit_drafts_and_removes_them_after_commit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("next.txt"), "next\n").unwrap();
    run_git(root, &["add", "next.txt"]);
    let draft_path = git::commit_draft_path(root).unwrap();

    let mut app = App::new(root.to_path_buf());
    wait_for_state(&mut app, |app| app.commit_draft_path.is_some());
    app.mode = Mode::Commit;
    app.handle_paste("persisted subject\npersisted body");
    app.flush_commit_draft();
    drop(app);

    let mut restored = App::new(root.to_path_buf());
    wait_for_state(&mut restored, |app| !app.commit_input.is_empty());
    assert_eq!(
        restored.commit_input.text(),
        "persisted subject\npersisted body"
    );
    restored.mode = Mode::Commit;
    restored.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    wait_for_state(&mut restored, |app| {
        app.repository().is_some_and(|repo| repo.commits.len() == 2)
    });
    assert!(!draft_path.exists());
}

#[test]
fn dirty_inline_editor_blocks_restart_and_workspace_switch() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    let mut editor = FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap();
    editor.insert("edited ").unwrap();
    app.file_editor = Some(editor);

    assert!(!app.can_restart());
    assert!(!app.start_repository_open(root.join("other"), false));
    assert_eq!(
        app.notice.as_deref(),
        Some("Save or close the editor before opening a workspace")
    );
}

#[test]
fn branch_hide_filters_git_graph_end_to_end() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);

    // Create feature branch with a commit
    run_git(root, &["checkout", "-b", "feature-secret"]);
    fs::write(root.join("secret.txt"), "secret\n").unwrap();
    run_git(root, &["add", "secret.txt"]);
    run_git(root, &["commit", "-m", "secret feature commit"]);

    // Go back to main and make a commit
    run_git(root, &["checkout", "main"]);
    fs::write(root.join("main.txt"), "main update\n").unwrap();
    run_git(root, &["add", "main.txt"]);
    run_git(root, &["commit", "-m", "main branch commit"]);

    let mut app = App::new(root.to_path_buf());
    wait_for_state(&mut app, |app| {
        app.repository()
            .map_or(false, |repo| repo.details_ready && repo.commits.len() >= 3)
    });

    let commits = app.repository().unwrap().commits.clone();
    assert_eq!(commits.len(), 3);
    assert_eq!(app.visible_graph_indices().len(), 3);

    // Open branch hide mode
    app.open_branch_hide();
    assert_eq!(app.mode, Mode::BranchHide);

    // Type the branch to hide
    app.handle_paste("feature-secret");
    assert_eq!(app.branch_filter.input.text(), "feature-secret");

    // Add to hidden branches
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.branch_filter.hidden_branches(), &["feature-secret"]);
    assert_eq!(app.branch_filter.input.text(), "");

    // Verify Git graph visible indices now only show the 2 main commits
    let visible = app.visible_graph_indices();
    assert_eq!(visible.len(), 2);
    let secret_commit_idx = commits
        .iter()
        .position(|c| c.subject.contains("secret feature commit"))
        .unwrap();
    assert!(!visible.contains(&secret_commit_idx));

    // Remove the branch from hidden branches
    app.remove_hidden_branch(0);
    assert!(app.branch_filter.hidden_branches().is_empty());
    assert_eq!(app.visible_graph_indices().len(), 3);

    // Close branch hide
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
}

fn initialize_repository(root: &Path) {
    for args in [
        &["init", "-b", "main"][..],
        &["config", "core.autocrlf", "false"][..],
        &["config", "user.name", "App Test"][..],
        &["config", "user.email", "app@example.com"][..],
    ] {
        run_git(root, args);
    }
    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "initial"]);
}

fn wait_for_state(app: &mut App, predicate: impl Fn(&App) -> bool) {
    for _ in 0..1_000 {
        let _ = app.poll_worker();
        if predicate(app) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("application state did not update");
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
