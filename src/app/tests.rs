use std::{process::Command, thread};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::media::MediaPreviewProtocol;

use super::*;

fn enable_herdr(app: &mut App) {
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [],
            "agents": [],
            "panes": []
        } }
    }));
}

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
fn control_c_quits_from_the_non_editable_add_file_choice() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.open_add_dialog();

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit);
}

#[test]
fn clearing_targets_removes_overlaps_but_keeps_adjacent_targets() {
    let mut regions = Regions::default();
    regions.register_hit_target(HitTarget::CommitMessageGenerate, Rect::new(0, 0, 4, 1));
    regions.register_hit_target(HitTarget::MarkdownPreviewToggle, Rect::new(3, 0, 4, 1));
    regions.register_hit_target(
        HitTarget::Graph(GraphHitTarget::AuthorHeader),
        Rect::new(7, 0, 2, 1),
    );
    regions.register_scroll_target(ScrollTarget::Preview, Rect::new(3, 0, 4, 1));
    regions.register_scroll_target(ScrollTarget::Graph, Rect::new(7, 0, 2, 1));

    regions.clear_targets_in(Rect::new(4, 0, 3, 1));

    assert!(
        regions
            .hit_target_rect(HitTarget::CommitMessageGenerate)
            .is_some()
    );
    assert!(
        regions
            .hit_target_rect(HitTarget::MarkdownPreviewToggle)
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
fn graph_scrolling_moves_the_viewport_without_moving_selection() {
    let mut state = TableState::default().with_selected(4);

    scroll_table(&mut state, 20, 5, 3);
    assert_eq!(state.selected(), Some(4));
    assert_eq!(state.offset(), 3);

    scroll_table(&mut state, 20, 5, 30);
    assert_eq!(state.selected(), Some(4));
    assert_eq!(state.offset(), 15);
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
    assert_eq!(repo.change_counts, (0, 0));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(app.repository_picker_details()[0].root, repo.root);
}

#[test]
fn opening_a_file_from_the_explorer_selects_it_in_the_new_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("auth.json"), "{}\n").unwrap();
    let mut app = App::new(root.to_path_buf());

    app.apply_explorer_command(PickerCommand::OpenFile(nested.join("auth.json")));
    wait_for_state(&mut app, |app| !app.session.open_running());

    let repo = app.repository().unwrap().clone();
    assert_eq!(repo.root, fs::canonicalize(&nested).unwrap());
    assert!(repo.is_local());
    assert_eq!(app.view(), View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(
        app.changes.selected_explorer_file_path(&repo),
        Some(&RepoPath::from("auth.json"))
    );
    assert!(app.pending_file_selection.is_none());
}

#[test]
fn repeated_open_keeps_the_first_workspace_request_active() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("file.txt"), "content\n").unwrap();
    let mut app = App::new(root.to_path_buf());

    app.open_repository(root.to_path_buf());
    assert!(app.session.open_running());
    app.open_repository(root.to_path_buf());

    assert_eq!(
        app.notice.as_deref(),
        Some("A workspace is already opening")
    );
    wait_for_state(&mut app, |app| !app.session.open_running());
    assert_eq!(
        app.repository().unwrap().root,
        fs::canonicalize(root).unwrap()
    );
    wait_for_state(&mut app, |app| {
        app.repository().is_some_and(|repo| repo.details_ready)
    });
    assert_eq!(app.notice.as_deref(), Some("Workspace ready"));
}

#[test]
fn successful_agent_creation_opens_its_destination() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(first.path().join("first.txt"), "first\n").unwrap();
    fs::write(second.path().join("second.txt"), "second\n").unwrap();
    let second_path = fs::canonicalize(second.path()).unwrap();
    let mut app = App::new(first.path().to_path_buf());
    app.mode = Mode::HerdrPrompt;
    app.herdr_prompt.complete_for_test(
        "Started agent in Herdr pane test-pane",
        Some(second.path().to_path_buf()),
    );

    app.poll_worker();
    wait_for_state(&mut app, |app| {
        !app.session.open_running()
            && app
                .repository()
                .is_some_and(|repository| repository.root == second_path)
    });

    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn background_agent_creation_opens_its_preview_without_changing_layout() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE" }],
            "agents": [{
                "agent": "opencode",
                "agent_status": "idle",
                "pane_id": "w1:p2"
            }],
            "panes": [{
                "pane_id": "w1:p2",
                "tab_id": "w1:t2",
                "workspace_id": "w1",
                "cwd": directory.path()
            }]
        } }
    }));
    app.herdr.set_background_attached_for_test("w1");
    app.herdr_prompt.complete_background_agent_for_test("w1:p2");

    app.poll_worker();

    assert_eq!(app.mode, Mode::AgentPreview);
    assert!(!app.herdr.agent_layout_running());
}

#[test]
fn background_agent_preview_handoff_expires_when_the_agent_disappears() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE" }],
            "agents": [],
            "panes": []
        } }
    }));
    app.herdr.set_background_attached_for_test("w1");
    app.pending_agent_preview_pane = Some((
        "w1:p2".to_owned(),
        std::time::Instant::now(),
        app.herdr.snapshot_request_generation(),
    ));

    app.poll_worker();

    assert!(app.pending_agent_preview_pane.is_some());
    assert_ne!(
        app.notice.as_deref(),
        Some("The new Herdr agent exited before its preview opened")
    );

    app.pending_agent_preview_pane = Some((
        "w1:p2".to_owned(),
        std::time::Instant::now(),
        app.herdr.snapshot_generation().saturating_sub(1),
    ));

    app.poll_worker();

    assert_eq!(app.pending_agent_preview_pane, None);
    assert_eq!(
        app.notice.as_deref(),
        Some("The new Herdr agent exited before its preview opened")
    );
}

#[test]
fn background_agent_activation_always_opens_the_preview() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut app = fullscreen_agent_app(first.path(), second.path());
    app.herdr.set_fullscreen_for_test(false);
    app.herdr.set_background_attached_for_test("w1");
    let key = app.herdr.agent_key(0).unwrap();

    assert!(!app.herdr_embedded());

    app.activate_agent_card(key, 0);

    assert_eq!(app.mode, Mode::AgentPreview);
    assert!(!app.herdr.agent_layout_running());
}

fn fullscreen_agent_app(source: &Path, destination: &Path) -> App {
    let mut app = App::new(source.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE" }],
            "agents": [{
                "agent": "opencode",
                "agent_session": {
                    "source": "env",
                    "agent": "opencode",
                    "kind": "session_id",
                    "value": "ses-live"
                },
                "agent_status": "idle",
                "pane_id": "w1:p2"
            }],
            "panes": [{
                "pane_id": "w1:p2",
                "tab_id": "w1:t2",
                "workspace_id": "w1",
                "cwd": destination
            }]
        } }
    }));
    app.herdr.set_host_for_test("w1", "w1:t1", "w1:p1");
    app.herdr.set_fullscreen_for_test(true);
    app
}

#[test]
fn fullscreen_agent_click_opens_the_agents_destination() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(first.path().join("first.txt"), "first\n").unwrap();
    fs::write(second.path().join("second.txt"), "second\n").unwrap();
    let destination = fs::canonicalize(second.path()).unwrap();
    let mut app = fullscreen_agent_app(first.path(), second.path());
    let key = app.herdr.agent_key(0).unwrap();

    app.activate_agent_card(key, 0);
    wait_for_state(&mut app, |app| {
        !app.session.open_running()
            && app
                .repository()
                .is_some_and(|repository| repository.root == destination)
    });

    assert!(app.herdr.fullscreen());
}

#[test]
fn fullscreen_agent_double_click_queues_layout_restore_and_selection() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut app = fullscreen_agent_app(first.path(), second.path());
    let key = app.herdr.agent_key(0).unwrap();

    app.activate_agent_card(key.clone(), 0);
    app.activate_agent_card(key.clone(), 0);

    assert!(app.herdr.fullscreen_running());
    assert_eq!(app.pending_fullscreen_agent, Some(key));
}

#[test]
fn restored_stash_waits_for_an_authoritative_live_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE" }],
            "agents": [],
            "panes": []
        } }
    }));
    app.herdr.set_stashed_agents_for_test(vec![StashedAgent {
        harness: "opencode".to_owned(),
        agent_name: "opencode".to_owned(),
        session_source: "env".to_owned(),
        session_kind: "session_id".to_owned(),
        session_id: "ses_restored".to_owned(),
        session_name: Some("Resume this session".to_owned()),
        repository: root.to_path_buf(),
        repository_label: "hunkle".to_owned(),
        worktree: root.to_path_buf(),
        branch: "feature/restore".to_owned(),
        workspace_id: "w1".to_owned(),
        tab_id: "w1:t1".to_owned(),
        pane_id: "w1:p2".to_owned(),
        cwd: Some(root.to_path_buf()),
        destination_cwd: Some(root.to_path_buf()),
        focused: false,
        status: AgentStatus::Idle,
        stashed_at_ms: 42,
    }]);
    app.herdr_prompt
        .complete_for_test("Started restored agent", None);

    app.poll_worker();

    assert_eq!(app.herdr.stashed_agents().len(), 1);
    app.herdr.apply_snapshot_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE" }],
            "agents": [{
                "agent": "opencode",
                "agent_session": {
                    "source": "env",
                    "agent": "opencode",
                    "kind": "session_id",
                    "value": "ses_restored"
                },
                "agent_status": "idle",
                "pane_id": "w1:p3",
                "tab_id": "w1:t1",
                "workspace_id": "w1"
            }],
            "panes": [{
                "pane_id": "w1:p3",
                "tab_id": "w1:t1",
                "workspace_id": "w1"
            }]
        } }
    }));

    assert!(app.herdr.stashed_agents().is_empty());
}

#[test]
fn workspace_open_errors_remain_visible_after_explorer_closes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("file.txt"), "content\n").unwrap();
    let mut app = App::new(root.to_path_buf());

    app.open_repository(root.join("missing"));
    app.mode = Mode::Normal;
    wait_for_state(&mut app, |app| !app.session.open_running());

    let notice = app.notice.as_deref().unwrap();
    assert!(notice.starts_with("Could not open workspace:"));
    assert_eq!(app.workspace_explorer.error.as_deref(), Some(notice));
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
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.agents_visible);
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Agents);
    assert!(!app.agents_pane_visible());
    assert_eq!(app.notice.as_deref(), Some("Not a Git repository"));

    fs::write(root.join("two.txt"), "two\n").unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    for _ in 0..100 {
        let _ = app.poll_worker();
        if app.repository().unwrap().files == ["one.txt", "two.txt"] {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.repository().unwrap().files, ["one.txt", "two.txt"]);
}

#[test]
fn local_workspaces_can_start_agents_from_their_root() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("notes.txt"), "notes\n").unwrap();
    let app = App::new(directory.path().to_path_buf());

    let path = app.agent_destination_for_start().unwrap();

    assert_eq!(path, fs::canonicalize(directory.path()).unwrap());
}

#[test]
fn creates_renames_drags_and_deletes_files_from_the_files_pane() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let renamed = " renamed.txt";
    fs::write(root.join("old.txt"), "content\n").unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    let mut app = App::new(root.to_path_buf());
    app.set_view_for_test(View::Changes);
    app.changes.pane = LeftPane::Files;

    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::SHIFT));
    assert_eq!(app.mode, Mode::Files);
    app.handle_paste(renamed);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.files.iter().any(|path| path == renamed))
            && app
                .selected_explorer_file_path()
                .is_some_and(|path| path == renamed)
    });
    assert!(root.join(renamed).is_file());
    assert_eq!(
        app.selected_explorer_file_path().map(RepoPath::display),
        Some(renamed.to_owned())
    );

    app.open_add_dialog();
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_paste("created");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.directories.iter().any(|path| path == "created"))
            && app.changes.explorer_rows().iter().any(|row| {
                row.directory_path
                    .as_ref()
                    .is_some_and(|path| path == "created")
            })
    });
    assert!(root.join("created").is_dir());

    let repo = app.session.data().unwrap();
    assert!(
        app.changes
            .select_explorer_path(repo, &RepoPath::from(renamed), 20)
    );
    app.regions.explorer_list = Some(Rect::new(0, 10, 30, 20));
    let source = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| row.file_path.as_ref().is_some_and(|path| path == renamed))
        .unwrap();
    let target = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "created")
        })
        .unwrap();
    assert!(app.begin_file_drag(Position::new(1, 10 + source as u16)));
    app.update_file_drag(Position::new(1, 10 + target as u16));
    app.finish_file_drag(Position::new(1, 10 + target as u16));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.files.iter().any(|path| path == "created/ renamed.txt"))
            && app
                .selected_explorer_file_path()
                .is_some_and(|path| path == "created/ renamed.txt")
    });
    assert!(root.join("created/ renamed.txt").is_file());

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL));
    assert!(matches!(
        app.file_dialog.as_ref().map(|dialog| &dialog.kind),
        Some(FileDialogKind::Delete { .. })
    ));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(root.join("created/ renamed.txt").is_file());
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for_state(&mut app, |_| !root.join("created/ renamed.txt").exists());
}

#[test]
fn confirms_and_discards_only_the_selected_unstaged_change() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("tracked.txt"), "staged\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    fs::write(root.join("tracked.txt"), "unstaged\n").unwrap();
    fs::write(root.join("other.txt"), "other unstaged\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    let change_index = app
        .repository()
        .unwrap()
        .changes
        .iter()
        .position(|change| change.path == "tracked.txt" && !change.staged)
        .unwrap();
    let row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.change_index == Some(change_index))
        .unwrap();
    let repo = app.repository().unwrap().clone();
    assert!(app.changes.select_worktree_row(&repo, row));

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(matches!(
        app.file_dialog.as_ref().map(|dialog| &dialog.kind),
        Some(FileDialogKind::DiscardUnstaged { change })
            if change.path == "tracked.txt" && !change.staged
    ));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "unstaged\n"
    );

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.repository().is_some_and(|repo| {
            repo.changes
                .iter()
                .filter(|change| change.path == "tracked.txt")
                .all(|change| change.staged)
                && repo
                    .changes
                    .iter()
                    .filter(|change| change.path == "tracked.txt")
                    .count()
                    == 1
        })
    });

    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "staged\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("other.txt")).unwrap(),
        "other unstaged\n"
    );
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert_eq!(
        app.repository()
            .and_then(|repo| app.changes.selected_change_index(repo))
            .and_then(|index| app.repository()?.changes.get(index))
            .map(|change| (change.path.display(), change.staged)),
        Some(("tracked.txt".to_owned(), true))
    );
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(app.file_dialog.is_none());
    assert_eq!(
        app.notice.as_deref(),
        Some("Select an unstaged change to discard")
    );
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "staged\n"
    );
}

#[cfg(unix)]
#[test]
fn refuses_hunk_actions_for_non_utf8_paths() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let name = OsString::from_vec(b"invalid-\x80.txt".to_vec());
    fs::write(root.join(&name), "original\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "add invalid path"]);
    fs::write(root.join(&name), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let change_index = app
        .repository()
        .unwrap()
        .changes
        .iter()
        .position(|change| change.path.as_path() == Path::new(&name) && !change.staged)
        .unwrap();
    let row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.change_index == Some(change_index))
        .unwrap();
    let repo = app.repository().unwrap().clone();
    assert!(app.changes.select_worktree_row(&repo, row));

    app.stage_hunk(0, false);

    assert_eq!(
        app.notice.as_deref(),
        Some("Hunk actions are unavailable for paths that are not valid UTF-8")
    );
    assert!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .any(|change| change.path.as_path() == Path::new(&name) && !change.staged)
    );
}

#[test]
fn graph_visibility_is_explicit_and_survives_reload() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);

    let mut app = App::new(root.to_path_buf());
    assert_eq!(app.view(), View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Files);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    wait_for_state(&mut app, |app| app.notice.as_deref() != Some("Refreshing…"));
    assert_eq!(app.view(), View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Changes);

    fs::write(root.join("tracked.txt"), "edited\n").unwrap();
    let mut dirty_app = App::new(root.to_path_buf());
    assert_eq!(dirty_app.view(), View::Changes);
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);

    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    dirty_app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    wait_for_state(&mut dirty_app, |app| {
        app.repository().is_some_and(|repo| repo.changes.is_empty())
    });
    assert_eq!(dirty_app.view(), View::Changes);
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);
}

#[test]
fn graph_can_be_hidden_after_returning_from_a_commit_diff() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let mut app = App::new(root.to_path_buf());
    let repo = app.repository().unwrap().clone();
    app.changes.set_pane(LeftPane::Worktree, Some(&repo));

    app.show_graph();
    app.open_selected_graph_commit();
    assert!(app.graph_commit_open());
    app.show_previous_panel();
    assert_eq!(app.visible_view(), View::Graph);

    app.toggle_graph();
    assert_eq!(app.view(), View::Changes);
    assert_eq!(app.visible_view(), View::Changes);
}

#[test]
fn background_startup_shows_the_bootstrap_before_selecting_the_final_pane() {
    let clean_directory = tempfile::tempdir().unwrap();
    initialize_repository(clean_directory.path());
    let mut clean_app = App::opening(clean_directory.path().to_path_buf());
    assert_eq!(clean_app.mode, Mode::Normal);
    assert!(clean_app.workspace_loading_initial_state());
    wait_for_state(&mut clean_app, |app| app.repository().is_some());
    assert!(!clean_app.workspace_loading_initial_state());
    assert_eq!(clean_app.changes.pane, LeftPane::Files);
    wait_for_state(&mut clean_app, |app| {
        app.repository().is_some_and(|repo| repo.details_ready)
    });
    assert_eq!(clean_app.changes.pane, LeftPane::Files);

    let dirty_directory = tempfile::tempdir().unwrap();
    initialize_repository(dirty_directory.path());
    fs::write(dirty_directory.path().join("tracked.txt"), "edited\n").unwrap();
    let mut dirty_app = App::opening(dirty_directory.path().to_path_buf());
    assert_eq!(dirty_app.mode, Mode::Normal);
    assert!(dirty_app.workspace_loading_initial_state());
    wait_for_state(&mut dirty_app, |app| app.repository().is_some());
    assert!(!dirty_app.workspace_loading_initial_state());
    assert_eq!(dirty_app.changes.pane, LeftPane::Files);
    wait_for_state(&mut dirty_app, |app| {
        app.repository().is_some_and(|repo| repo.details_ready)
    });
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);
}

#[test]
fn deferred_initial_pane_selection_does_not_hide_an_open_workspace() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());

    app.initial_pane_pending = true;

    assert!(!app.workspace_loading_initial_state());
}

#[test]
fn worktree_actions_preserve_the_visible_graph() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("tracked.txt"), "edited\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.set_view_for_test(View::Graph);
    app.stage_all();
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.changes.iter().all(|change| change.staged))
    });
    assert_eq!(app.view(), View::Graph);

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.changes.iter().all(|change| !change.staged))
    });
    assert_eq!(app.view(), View::Graph);

    let selected = app.changes.worktree_state.selected().unwrap() as u16;
    app.regions.worktree_list = Some(Rect::new(0, 0, 20, selected + 1));
    app.regions.register_hit_target(
        HitTarget::Changes(app.changes.worktree_stage_target(selected as usize)),
        Rect::new(18, selected, 2, 1),
    );
    app.handle_left_click(Position::new(19, selected));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.changes.iter().all(|change| change.staged))
    });
    assert_eq!(app.view(), View::Graph);
}

#[test]
fn explorer_captures_typing_instead_of_normal_shortcuts() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());

    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    let pane = app.changes.pane;
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.changes.pane, pane);
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));

    assert_eq!(app.mode, Mode::Explorer);
    assert!(app.workspace_explorer.editing_path);
    assert_eq!(app.workspace_explorer.path_input, "sh");
    assert_eq!(app.workspace_explorer.directory, directory.path());
}

#[test]
fn explorer_browse_finishes_while_the_explorer_is_hidden() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let nested = root.join("nested");
    let child = nested.join("ready");
    fs::create_dir_all(&child).unwrap();
    initialize_repository(root);
    let mut app = App::new(root.to_path_buf());
    wait_for_state(&mut app, |app| app.repository().is_some());
    app.mode = Mode::Normal;
    app.workspace_explorer.navigate(nested.clone());

    wait_for_state(&mut app, |app| !app.workspace_explorer.loading);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.workspace_explorer.directory, nested);
    assert!(
        app.workspace_explorer
            .entries
            .iter()
            .any(|entry| entry.path == child)
    );
}

#[test]
fn primary_navigation_has_stable_precedence_and_edits_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    initialize_repository(directory.path());
    fs::write(directory.path().join("tracked.txt"), "edited\n").unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.mode = Mode::Normal;
    app.settings = Settings::default();
    app.settings_store = SettingsStore::at(path.clone());

    assert!(app.agents_visible);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.agents_visible);
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Scheduled);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.agents_visible);
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Stash);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(!app.agents_visible);
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Agents);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.agents_visible);
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Agents);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.view(), View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("Could not toggle fullscreen:"))
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Graph);
    app.set_graph_commit_open_for_test(true);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.agents_pane_visible());
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert_eq!(app.view(), View::Graph);
    assert!(app.graph_commit_open());
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert_eq!(app.view(), View::Graph);
    assert!(app.graph_commit_open());

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Changes);
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Graph);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    app.mode = Mode::Commit;
    app.commit_input.clear();
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Commit);
    assert_eq!(app.commit_input.text(), "g");
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Commit);
    assert_eq!(app.commit_input.text(), "g");
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert_eq!(app.view(), View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Settings);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(
        app.settings,
        Settings {
            auto_fetch: true,
            fetch_interval_minutes: 6,
            format_on_save: true,
            worktree_width: 38,
            cross_workspace_agents: false,
            show_agent_harness: false,
            agent_card_click_action: settings::AgentCardClickAction::ChangeLayout,
            agent_time_display: settings::AgentTimeDisplay::LatestLoop,
            agents_height: 7,
            graph_lane_width: 0,
            graph_description_width: 0,
            graph_changes_width: 12,
            graph_date_width: 12,
            graph_author_width: 16,
            graph_commit_width: 7,
            explorer_left_pane_width: None,
            editor_command: None,
            opencode_model: "opencode/deepseek-v4-flash-free".to_owned(),
            opencode_reasoning: OpenCodeReasoning::Max,
            media_preview_protocol: MediaPreviewProtocol::Auto,
            shortcuts: Shortcuts::default(),
        }
    );
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.settings.format_on_save);
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.settings.cross_workspace_agents);
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.settings.show_agent_harness);
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.settings.agent_card_click_action,
        settings::AgentCardClickAction::OpenPreview
    );
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.settings.agent_time_display,
        settings::AgentTimeDisplay::AgentTotal
    );
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.notice.as_deref(), Some("Agent timing history cleared"));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.settings.media_preview_protocol,
        MediaPreviewProtocol::Halfblocks
    );
    assert_eq!(app.settings_store.load(), app.settings);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Editor);
    assert!(app.editor_configure_only);
    app.editor_input.clear();
    app.handle_paste("nvim");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Settings);
    assert_eq!(app.settings.editor_command.as_deref(), Some("nvim"));
    assert_eq!(app.settings_store.load(), app.settings);

    app.mode = Mode::Normal;
    app.set_view_for_test(View::Changes);
    app.changes.diff_scroll = 37;
    assert!(app.changes.diff_wrap);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(!app.changes.diff_wrap);
    assert_eq!(app.changes.diff_scroll, 37);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(app.changes.diff_wrap);
    assert_eq!(app.changes.diff_scroll, 37);
}

#[test]
fn function_keys_select_changes_files_and_agents() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());

    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert!(!app.agents_pane_visible());

    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert!(!app.agents_pane_visible());

    enable_herdr(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert!(app.agents_pane_visible());

    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert!(!app.agents_pane_visible());
}

#[test]
fn scheduler_f4_works_without_herdr_and_toggles_the_modal() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());

    app.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Scheduler);
    app.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn scheduler_composer_edits_fields_and_narrow_back_returns_to_tasks() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.layout_profile = LayoutProfile::Single;
    app.open_scheduler();
    app.begin_scheduled_task();

    app.handle_paste("Nightly review\nignored");
    assert_eq!(
        app.scheduler.composer.as_ref().unwrap().title.text(),
        "Nightly reviewignored"
    );
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_paste("Check open changes");
    assert_eq!(
        app.scheduler.composer.as_ref().unwrap().description.text(),
        "Check open changes"
    );

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_paste("Review the diff\nand summarize it");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let composer = app.scheduler.composer.as_ref().unwrap();
    assert_eq!(
        composer.prompt.text(),
        "Review the diff\nand summarize it\n"
    );
    assert!(!composer.prompt_expanded);
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert!(app.scheduler.composer.as_ref().unwrap().prompt_expanded);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        app.scheduler.composer.as_ref().unwrap().schedule.text(),
        "15"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(
        app.scheduler.composer.as_ref().unwrap().schedule.text(),
        "15"
    );

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.scheduler.composer.as_ref().unwrap().prompt_expanded);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.scheduler.composer.is_none());
    assert_eq!(app.scheduler.surface, SchedulerSurface::Tasks);
    assert_eq!(app.mode, Mode::Scheduler);
}

#[test]
fn scheduler_composer_selects_a_named_discord_webhook() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.discord_webhooks.push(DiscordWebhookConfig {
        id: "123456".to_owned(),
        server: "Hunkle".to_owned(),
        channel: "reports".to_owned(),
        webhook_name: "Scheduler".to_owned(),
        url: "https://discord.com/api/webhooks/123456/token".to_owned(),
    });
    app.open_scheduler();
    app.begin_scheduled_task();
    for _ in 0..5 {
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));

    let composer = app.scheduler.composer.as_ref().unwrap();
    assert_eq!(composer.field, SchedulerField::Discord);
    assert_eq!(composer.discord_webhook, 1);
    assert_eq!(
        composer.discord_webhook_label(),
        "Hunkle / #reports / Scheduler"
    );
}

#[test]
fn scheduler_edits_an_existing_task_in_the_shared_composer() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.scheduled_tasks.set_tasks_for_test(vec![ScheduledTask {
        id: 7,
        title: "Nightly review".to_owned(),
        description: "Check open changes".to_owned(),
        prompt: "Review the diff and summarize it.".to_owned(),
        model: "openai/gpt-5.6-sol".to_owned(),
        discord_webhook_id: String::new(),
        destination: directory.path().to_path_buf(),
        repository: "repo".to_owned(),
        branch: "main".to_owned(),
        enabled: false,
        interval_minutes: 90,
        next_run_ms: 1,
        source: None,
        project_status: None,
    }]);
    app.mode = Mode::Scheduler;
    app.scheduler.surface = SchedulerSurface::Detail;
    app.scheduler.selected_task_id = Some(7);

    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

    let composer = app.scheduler.composer.as_ref().unwrap();
    assert_eq!(composer.task_id, Some(7));
    assert_eq!(composer.title.text(), "Nightly review");
    assert_eq!(composer.description.text(), "Check open changes");
    assert_eq!(composer.prompt.text(), "Review the diff and summarize it.");
    assert_eq!(composer.model.text(), "openai/gpt-5.6-sol");
    assert_eq!(composer.schedule.text(), "90");
    assert!(!composer.enabled);
}

#[test]
fn scheduler_run_now_queues_headless_preview_without_herdr() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    app.scheduled_tasks.set_tasks_for_test(vec![ScheduledTask {
        id: 7,
        title: "Nightly review".to_owned(),
        description: String::new(),
        prompt: "Review the diff.".to_owned(),
        model: String::new(),
        discord_webhook_id: String::new(),
        destination: directory.path().to_path_buf(),
        repository: "repo".to_owned(),
        branch: "main".to_owned(),
        enabled: true,
        interval_minutes: 90,
        next_run_ms: 1,
        source: None,
        project_status: None,
    }]);
    app.mode = Mode::Scheduler;
    app.scheduler.surface = SchedulerSurface::Detail;
    app.scheduler.selected_task_id = Some(7);

    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));

    assert_eq!(app.scheduler.surface, SchedulerSurface::Detail);
    assert!(app.scheduler.preview_pending);
    assert_eq!(app.scheduler.selected_run_id, None);
    assert_eq!(app.scheduler.run_scroll, 0);
}

#[test]
fn scheduler_v_opens_shared_agent_preview_and_returns_to_scheduler() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.scheduled_tasks.set_tasks_for_test(vec![ScheduledTask {
        id: 7,
        title: "Nightly review".to_owned(),
        description: String::new(),
        prompt: "Review the diff.".to_owned(),
        model: String::new(),
        discord_webhook_id: String::new(),
        destination: directory.path().to_path_buf(),
        repository: "repo".to_owned(),
        branch: "main".to_owned(),
        enabled: true,
        interval_minutes: 90,
        next_run_ms: 1,
        source: None,
        project_status: None,
    }]);
    app.scheduled_tasks.set_runs_for_test(vec![ScheduledRun {
        id: 11,
        task_id: 7,
        created_at_ms: 1,
        completed_at_ms: Some(2),
        status: ScheduledRunStatus::Completed,
        pane_id: None,
        terminal_id: None,
        session_id: Some("ses-live".to_owned()),
        error: Some("No session".to_owned()),
    }]);
    app.mode = Mode::Scheduler;
    app.scheduler.surface = SchedulerSurface::Detail;
    app.scheduler.selected_task_id = Some(7);
    app.scheduler.selected_run_id = Some(11);

    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::AgentPreview);
    assert_eq!(app.agent_preview.scheduled_run, Some(11));

    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Scheduler);
    assert_eq!(app.agent_preview.scheduled_run, None);
}

#[test]
fn scheduled_run_promotion_preserves_destination_and_exact_session() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.scheduled_tasks.set_tasks_for_test(vec![ScheduledTask {
        id: 7,
        title: "Nightly review".to_owned(),
        description: String::new(),
        prompt: "Review the diff.".to_owned(),
        model: String::new(),
        discord_webhook_id: String::new(),
        destination: directory.path().join("scheduled-worktree"),
        repository: "repo".to_owned(),
        branch: "main".to_owned(),
        enabled: true,
        interval_minutes: 90,
        next_run_ms: 1,
        source: None,
        project_status: None,
    }]);
    app.scheduled_tasks.set_runs_for_test(vec![ScheduledRun {
        id: 11,
        task_id: 7,
        created_at_ms: 1,
        completed_at_ms: Some(2),
        status: ScheduledRunStatus::Completed,
        pane_id: None,
        terminal_id: None,
        session_id: Some("ses_exact_scheduled".to_owned()),
        error: None,
    }]);

    let (destination, session_id) = app.scheduled_run_promotion(11).unwrap();

    assert_eq!(destination, directory.path().join("scheduled-worktree"));
    assert_eq!(session_id, "ses_exact_scheduled");
}

#[test]
fn headless_scheduled_preview_can_focus_its_prompt_without_a_matching_pane() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE" }],
            "agents": [{
                "agent": "opencode",
                "agent_session": {
                    "source": "env",
                    "agent": "opencode",
                    "kind": "session_id",
                    "value": "ses-live"
                },
                "agent_status": "idle",
                "pane_id": "w1:p2"
            }],
            "panes": [{
                "pane_id": "w1:p2",
                "tab_id": "w1:t2",
                "workspace_id": "w1",
                "cwd": directory.path()
            }]
        } }
    }));
    app.scheduled_tasks.set_tasks_for_test(Vec::new());
    app.scheduled_tasks.set_runs_for_test(vec![ScheduledRun {
        id: 11,
        task_id: 7,
        created_at_ms: 1,
        completed_at_ms: None,
        status: ScheduledRunStatus::Completed,
        pane_id: Some("w1:p2".to_owned()),
        terminal_id: None,
        session_id: Some("ses-live".to_owned()),
        error: None,
    }]);
    app.agent_preview.open_scheduled_run(11, Mode::Normal);

    app.focus_agent_preview_prompt();

    assert!(app.agent_preview.prompt_focused);

    app.agent_preview.blur_prompt();
    app.scheduled_tasks.set_runs_for_test(vec![ScheduledRun {
        id: 11,
        task_id: 7,
        created_at_ms: 1,
        completed_at_ms: None,
        status: ScheduledRunStatus::Completed,
        pane_id: Some("w1:p2".to_owned()),
        terminal_id: None,
        session_id: Some("ses-stale".to_owned()),
        error: None,
    }]);
    assert!(app.agent_preview_index().is_none());
    assert!(
        app.scheduled_tasks
            .runs()
            .iter()
            .find(|run| run.id == 11)
            .is_some_and(|run| run.session_id.is_some())
    );
    app.focus_agent_preview_prompt();
    assert!(app.agent_preview.prompt_focused);
}

#[test]
fn scheduler_prompt_follows_the_cursor_and_accepts_wheel_scrolling() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.open_scheduler();
    app.begin_scheduled_task();
    let prompt = (0..30)
        .map(|line| format!("prompt line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.activate_scheduler_target(SchedulerHitTarget::Field(SchedulerField::Prompt));
    app.handle_paste(&prompt);
    app.regions.register_hit_target(
        HitTarget::Scheduler(SchedulerHitTarget::Field(SchedulerField::Prompt)),
        Rect::new(0, 0, 40, 5),
    );

    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    let cursor_scroll = app.scheduler.composer.as_ref().unwrap().prompt_scroll;
    assert!(cursor_scroll > 0);
    app.scroll_scheduler(ScrollTarget::SchedulerPrompt, -1);
    assert!(app.scheduler.composer.as_ref().unwrap().prompt_scroll < cursor_scroll);
}

#[test]
fn scheduler_mode_intercepts_underlying_header_pointer_target() {
    let directory = tempfile::tempdir().unwrap();
    initialize_repository(directory.path());
    let mut app = App::new(directory.path().to_path_buf());
    enable_herdr(&mut app);
    app.open_scheduler();
    let rect = Rect::new(2, 2, 8, 1);
    app.regions
        .register_hit_target(HitTarget::HeaderRepository, rect);

    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 2,
        modifiers: KeyModifiers::NONE,
    });

    assert_eq!(app.mode, Mode::Scheduler);
    assert!(!app.header_picker.is_open());
}

#[test]
fn shortcut_settings_rebind_reset_and_persist_commands() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    let mut app = App::new(directory.path().to_path_buf());
    app.settings = Settings::default();
    app.settings_store = SettingsStore::at(path);
    app.mode = Mode::Settings;
    app.settings_state.page = SettingsPage::Shortcuts;
    app.settings_state.shortcut_selection =
        Shortcuts::definitions(app.herdr_available(), app.herdr_embedded())
            .position(|definition| definition.action == ShortcutAction::OpenExplorer)
            .unwrap();

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    assert_eq!(
        app.settings.shortcuts.label(ShortcutAction::OpenExplorer),
        "Alt+v"
    );
    assert_eq!(app.settings_store.load(), app.settings);

    app.mode = Mode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    assert_eq!(app.mode, Mode::Explorer);

    app.mode = Mode::Settings;
    app.settings_state.page = SettingsPage::Shortcuts;
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(
        app.settings.shortcuts.label(ShortcutAction::OpenExplorer),
        "o"
    );
    assert_eq!(app.settings_store.load(), app.settings);
}

#[test]
fn opencode_settings_edit_model_reasoning_and_persist() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    let mut app = App::new(directory.path().to_path_buf());
    app.settings = Settings::default();
    app.settings_store = SettingsStore::at(path);
    app.mode = Mode::Settings;
    app.settings_state.page = SettingsPage::OpenCode;

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.handle_paste("provider/model");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.settings.opencode_model, "provider/model");
    assert_eq!(app.settings_store.load(), app.settings);

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.settings.opencode_reasoning, OpenCodeReasoning::High);
    assert_eq!(app.settings_store.load(), app.settings);

    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.handle_paste("not a model");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.settings.opencode_model, "provider/model");
    assert!(app.settings_state.opencode_error.is_some());
}

#[test]
fn discord_settings_paste_save_and_remove_the_webhook() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("discord-webhook");
    let mut app = App::new(directory.path().to_path_buf());
    app.discord_webhook_store = DiscordWebhookStore::at(path.clone());
    app.mode = Mode::Settings;
    app.settings_state.page = SettingsPage::Discord;

    app.settings_state.discord_selection = 1;
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let editor = app.settings_state.discord_webhook_editor.as_mut().unwrap();
    editor.server.set("Hunkle");
    editor.channel.set("reports");
    editor.webhook_name.set("Scheduler");
    editor
        .url
        .set("https://discord.com/api/webhooks/123456/token");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert!(path.exists());
    assert_eq!(
        app.discord_webhooks.first().map(|webhook| (
            webhook.server.as_str(),
            webhook.channel.as_str(),
            webhook.webhook_name.as_str(),
        )),
        Some(("Hunkle", "reports", "Scheduler"))
    );
    assert!(app.settings_state.discord_webhook_editor.is_none());

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!path.exists());
    assert!(app.discord_webhooks.is_empty());
}

#[test]
fn auto_fetch_runs_without_blocking_the_app() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for args in [
        &["init", "-b", "main"][..],
        &["config", "user.name", "Fetch Test"][..],
        &["config", "user.email", "fetch@example.com"][..],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
    for args in [&["add", "."][..], &["commit", "-m", "initial"][..]] {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    let mut app = App::new(root.to_path_buf());
    app.settings.auto_fetch = true;
    app.session.schedule_fetch_now();
    let _ = app.poll_worker();
    assert!(app.fetch_running());

    for _ in 0..100 {
        thread::sleep(Duration::from_millis(10));
        let _ = app.poll_worker();
        if !app.fetch_running() {
            break;
        }
    }
    assert!(!app.fetch_running());
    assert_eq!(app.notice.as_deref(), Some("Fetched remotes"));
}

#[test]
fn history_only_refresh_restores_graph_without_refreshing_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("tracked.txt"), "second\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "second"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.auto_fetch = false;
    wait_for_state(&mut app, |app| {
        app.changes
            .preview
            .text()
            .is_some_and(|text| text.contains("second"))
    });
    app.graph_state.select(Some(1));
    let selected_oid = app.selected_graph_commit().unwrap().oid.clone();
    app.changes.preview_branch_diff(
        root,
        "main".to_owned(),
        "previous".to_owned(),
        "refs/heads/main".to_owned(),
        "HEAD~".to_owned(),
    );
    let rows_generation = app.changes.worktree_rows_generation_for_test();
    let preview_generation = app.changes.preview_request_generation_for_test();

    fs::write(root.join("tracked.txt"), "third\n").unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "third"]);
    app.reload(RefreshScope::HISTORY_AND_REFS);

    let restoration = app.pending_reload.as_ref().unwrap();
    assert!(restoration.changes.is_none());
    assert_eq!(
        restoration.selected_graph_oid.as_deref(),
        Some(selected_oid.as_str())
    );
    wait_for_state(&mut app, |app| {
        app.pending_reload.is_none() && app.repository().unwrap().commits.len() == 3
    });

    assert_eq!(app.selected_graph_commit().unwrap().oid, selected_oid);
    assert_eq!(
        app.changes
            .branch_comparison()
            .map(|comparison| (comparison.current.as_str(), comparison.target.as_str())),
        Some(("main", "previous"))
    );
    assert_eq!(
        app.changes.worktree_rows_generation_for_test(),
        rows_generation
    );
    assert_eq!(
        app.changes.preview_request_generation_for_test(),
        preview_generation
    );
}

#[test]
fn workspace_fetches_expire_after_five_minutes() {
    let now = Instant::now();
    assert!(!fetch_is_fresh(None, now));
    assert!(fetch_is_fresh(
        Some(&(now - Duration::from_secs(5 * 60 - 1))),
        now
    ));
    assert!(!fetch_is_fresh(
        Some(&(now - Duration::from_secs(5 * 60))),
        now
    ));
}

#[test]
fn recording_workspace_fetch_removes_expired_entries() {
    let now = Instant::now();
    let expired = PathBuf::from("expired");
    let fresh = PathBuf::from("fresh");
    let inserted = PathBuf::from("inserted");
    let mut recent_fetches = HashMap::from([
        (expired.clone(), now - WORKSPACE_FETCH_FRESHNESS),
        (
            fresh.clone(),
            now - WORKSPACE_FETCH_FRESHNESS + Duration::from_secs(1),
        ),
    ]);

    insert_recent_fetch(&mut recent_fetches, inserted.clone(), now);

    assert!(!recent_fetches.contains_key(&expired));
    assert!(recent_fetches.contains_key(&fresh));
    assert_eq!(recent_fetches.get(&inserted), Some(&now));
}

#[test]
fn control_j_commits_in_terminals_that_encode_control_enter_as_line_feed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("next.txt"), "next\n").unwrap();
    run_git(root, &["add", "next.txt"]);

    let mut app = App::new(root.to_path_buf());
    app.mode = Mode::Commit;
    app.commit_input.set("commit from control enter");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert!(app.commit_running());
    assert_eq!(app.commit_input.text(), "commit from control enter");

    for _ in 0..100 {
        thread::sleep(Duration::from_millis(10));
        let _ = app.poll_worker();
        if app.repository().unwrap().commits.len() == 2 {
            break;
        }
    }
    assert!(!app.commit_running());
    assert!(app.commit_input.is_empty());
    assert_eq!(app.repository().unwrap().commits.len(), 2);
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
    assert!(!draft_path.exists());
    assert!(app.commit_draft_due.is_some());
    app.flush_commit_draft();
    assert_eq!(
        fs::read_to_string(&draft_path).unwrap(),
        "persisted subject\npersisted body"
    );
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
    assert!(restored.commit_input.is_empty());
    assert!(!draft_path.exists());
}

#[test]
fn keeps_edits_pending_until_async_draft_discovery_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let mut app = App::new(root.to_path_buf());
    let draft_path = root.join("draft");
    let (sender, receiver) = mpsc::channel();
    app.commit_draft_rx = Some(receiver);
    app.commit_draft_path = None;
    app.commit_input.set("new message");
    app.commit_draft_due = Some(Instant::now());

    assert!(!app.flush_commit_draft());
    assert!(app.commit_draft_due.is_some());
    sender
        .send(CommitDraftResult {
            root: root.to_path_buf(),
            result: Ok((draft_path.clone(), Some("old message".to_owned()))),
        })
        .unwrap();
    app.poll_worker();
    app.flush_commit_draft();

    assert_eq!(app.commit_input.text(), "new message");
    assert_eq!(fs::read_to_string(draft_path).unwrap(), "new message");
}

#[test]
fn failed_draft_save_blocks_workspace_switch_and_keeps_retry_pending() {
    let current = tempfile::tempdir().unwrap();
    initialize_repository(current.path());
    let next = tempfile::tempdir().unwrap();
    initialize_repository(next.path());
    let mut app = App::new(current.path().to_path_buf());
    app.commit_input.set("unsaved draft");
    app.commit_draft_path = Some(current.path().join("missing/draft"));
    app.commit_draft_due = Some(Instant::now());
    app.pending_workspace_restore = Some(next.path().to_path_buf());

    app.try_start_workspace_restore();

    assert_eq!(app.commit_input.text(), "unsaved draft");
    assert!(app.commit_draft_due.is_some());
    assert_eq!(
        app.pending_workspace_restore,
        Some(next.path().to_path_buf())
    );
    assert!(!app.session.open_running());
}

#[test]
fn generated_commit_message_returns_to_its_repository_during_a_workspace_switch() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("requested");
    let other = directory.path().join("other");
    fs::create_dir(&requested).unwrap();
    fs::create_dir(&other).unwrap();
    initialize_repository(&requested);
    initialize_repository(&other);
    let mut app = App::new(requested.clone());
    wait_for_state(&mut app, |app| {
        app.commit_draft_path.is_some()
            && app
                .repository()
                .is_some_and(|repository| repository.details_ready)
    });
    app.commit_input.set("message before generation");
    app.schedule_commit_draft();
    app.flush_commit_draft();
    let requested_draft = git::commit_draft_path(&requested).unwrap();
    assert_eq!(
        fs::read_to_string(&requested_draft).unwrap(),
        "message before generation"
    );

    assert!(app.start_repository_open(other.clone(), false));
    app.receive_generated_commit_message(CommitMessageCompletion {
        root: requested.clone(),
        baseline: "message before generation".to_owned(),
        result: Ok("generated subject\n\ngenerated body".to_owned()),
    });
    assert_eq!(
        fs::read_to_string(&requested_draft).unwrap(),
        "generated subject\n\ngenerated body"
    );

    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repository| repository.root == other && repository.details_ready)
    });
    assert!(app.start_repository_open(requested.clone(), false));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repository| repository.root == requested)
            && app.commit_input.text() == "generated subject\n\ngenerated body"
    });
}

#[test]
fn generated_commit_message_does_not_overwrite_an_edited_inactive_draft() {
    let directory = tempfile::tempdir().unwrap();
    let active = directory.path().join("active");
    let requested = directory.path().join("requested");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&requested).unwrap();
    initialize_repository(&active);
    initialize_repository(&requested);
    let mut app = App::new(active);
    let requested_draft = git::commit_draft_path(&requested).unwrap();
    fs::write(&requested_draft, "edited while generation ran").unwrap();

    app.receive_generated_commit_message(CommitMessageCompletion {
        root: requested.clone(),
        baseline: "original draft".to_owned(),
        result: Ok("generated message".to_owned()),
    });

    assert_eq!(
        fs::read_to_string(requested_draft).unwrap(),
        "edited while generation ran"
    );
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|notice| notice.contains("draft was edited"))
    );
}

#[test]
fn commit_action_submits_an_existing_message() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("next.txt"), "next\n").unwrap();
    run_git(root, &["add", "next.txt"]);

    let mut app = App::new(root.to_path_buf());
    app.commit_input.set("commit from actions");
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.commit_running());
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(10));
        let _ = app.poll_worker();
        if app.repository().unwrap().commits.len() == 2 {
            break;
        }
    }
    assert!(!app.commit_running());
    assert!(app.commit_input.is_empty());
    assert_eq!(app.repository().unwrap().commits.len(), 2);
}

#[test]
fn keeps_hunk_mode_on_the_next_hunk_after_staging() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let baseline = (1..=20)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
    fs::write(
        root.join("tracked.txt"),
        format!("{}\n", baseline.join("\n")),
    )
    .unwrap();
    run_git(root, &["add", "tracked.txt"]);
    run_git(root, &["commit", "-m", "expand fixture"]);

    let mut edited = baseline;
    edited[1] = "changed first".to_owned();
    edited[18] = "changed second".to_owned();
    fs::write(root.join("tracked.txt"), format!("{}\n", edited.join("\n"))).unwrap();
    let mut app = App::new(root.to_path_buf());
    for _ in 0..100 {
        let _ = app.poll_worker();
        if app
            .changes
            .preview
            .text()
            .unwrap_or_default()
            .matches("@@")
            .count()
            == 2
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    app.settings
        .shortcuts
        .set(
            ShortcutAction::StageSelection,
            KeyChord::new(KeyCode::Char('v'), KeyModifiers::ALT),
        )
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    assert!(
        !app.repository()
            .unwrap()
            .changes
            .iter()
            .any(|change| change.staged)
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));

    for _ in 0..200 {
        let _ = app.poll_worker();
        let repo = app.repository().unwrap();
        let split_change = repo
            .changes
            .iter()
            .any(|change| change.path == "tracked.txt" && change.staged)
            && repo
                .changes
                .iter()
                .any(|change| change.path == "tracked.txt" && !change.staged);
        if split_change
            && app.changes.hunk_selection == Some(0)
            && app
                .changes
                .preview
                .text()
                .is_some_and(|text| text.contains("changed second"))
            && !app
                .changes
                .preview
                .text()
                .is_some_and(|text| text.contains("changed first"))
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(app.changes.hunk_selection, Some(0));
    assert!(
        app.changes
            .preview
            .text()
            .unwrap()
            .contains("changed second")
    );
    assert!(
        !app.changes
            .preview
            .text()
            .unwrap()
            .contains("changed first")
    );
}

#[test]
fn configures_and_requests_an_interactive_editor() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("tracked.txt"), "edited\n").unwrap();
    let settings_path = root.join(".git/hunkle-editor-test-config");
    let mut app = App::new(root.to_path_buf());
    app.settings.editor_command = None;
    app.settings_store = SettingsStore::at(settings_path.clone());

    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Editor);
    app.editor_input.clear();
    app.handle_paste("code --wait");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let request = app.take_editor_request().unwrap();
    assert_eq!(request.command, ["code", "--wait"]);
    assert_eq!(
        request.file,
        fs::canonicalize(root.join("tracked.txt")).unwrap()
    );
    assert_eq!(request.repository, fs::canonicalize(root).unwrap());
    assert_eq!(app.settings.editor_command.as_deref(), Some("code --wait"));
    assert!(
        fs::read_to_string(settings_path)
            .unwrap()
            .contains("editor_command=code --wait")
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Editor);
    assert_eq!(app.editor_input, "code --wait");

    app.mode = Mode::Normal;
    let repo = app.repository().unwrap().clone();
    app.changes.set_pane(LeftPane::Files, Some(&repo));
    assert!(
        app.changes
            .select_explorer_path(&repo, &RepoPath::from("tracked.txt"), 20)
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(
        app.take_editor_request().unwrap().file,
        fs::canonicalize(root.join("tracked.txt")).unwrap()
    );

    run_git(root, &["add", "tracked.txt"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.editor_command = Some("code --wait".to_owned());
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(
        app.take_editor_request().unwrap().file,
        fs::canonicalize(root.join("tracked.txt")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn control_s_formats_the_selected_file_with_a_project_formatter() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
    fs::write(root.join("config.jsonc"), "{\"value\":true}\n").unwrap();
    let formatter = root.join("node_modules/.bin/prettier");
    fs::write(
        &formatter,
        "#!/bin/sh\nprintf '{\\n  \"formatted\": true\\n}\\n' > \"$2\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&formatter).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&formatter, permissions).unwrap();

    let mut app = App::new(root.to_path_buf());
    let repo = app.repository().unwrap().clone();
    app.changes.set_pane(LeftPane::Files, Some(&repo));
    assert!(
        app.changes
            .select_explorer_path(&repo, &RepoPath::from("config.jsonc"), 20)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, Mode::Normal);
    assert!(app.format_running());
    wait_for_state(&mut app, |app| !app.format_running());
    assert_eq!(
        fs::read_to_string(root.join("config.jsonc")).unwrap(),
        "{\n  \"formatted\": true\n}\n"
    );
    assert_eq!(
        app.notice.as_deref(),
        Some("Formatted config.jsonc with Prettier")
    );
}

#[test]
fn control_s_reports_files_without_a_known_formatter() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    let repo = app.repository().unwrap().clone();
    app.changes.set_pane(LeftPane::Files, Some(&repo));
    assert!(
        app.changes
            .select_explorer_path(&repo, &RepoPath::from("notes.txt"), 20)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.notice.as_deref(),
        Some("No known formatter for .txt files")
    );
}

#[test]
fn inline_save_skips_formatting_when_disabled() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.settings.format_on_save = false;
    let mut editor = FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap();
    editor.insert("edited ").unwrap();
    app.file_editor = Some(editor);
    app.mode = Mode::FileEdit;

    app.save_file_editor(false);

    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "edited notes\n"
    );
    assert_eq!(app.notice.as_deref(), Some("Saved notes.txt"));
    assert_eq!(app.mode, Mode::FileEdit);
    assert!(!app.format_running());
}

#[test]
fn dirty_inline_editor_blocks_restart() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    let mut editor = FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap();
    assert!(app.can_restart());

    editor.insert("edited ").unwrap();
    app.file_editor = Some(editor);

    assert!(!app.can_restart());
    assert!(app.dirty_file_edit());
}

#[test]
fn opening_a_workspace_queues_its_local_hunkle_build() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let executable = root
        .join("target")
        .join("hunkle-install")
        .join("bin")
        .join(format!("hunkle{}", std::env::consts::EXE_SUFFIX));
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, "local build").unwrap();
    let mut app = App::opening(root.to_path_buf());

    wait_for_state(&mut app, |app| !app.session.open_running());

    assert_eq!(
        app.take_restart_request().as_deref(),
        Some(executable.as_path())
    );
}

#[test]
fn workspace_local_builds_stay_on_their_own_update_channel() {
    let executable = Path::new("worktree")
        .join("target")
        .join("hunkle-install")
        .join("bin")
        .join(format!("hunkle{}", std::env::consts::EXE_SUFFIX));

    assert!(is_workspace_local_build(&executable));
    assert!(!is_workspace_local_build(Path::new("target/debug/hunkle")));
}

#[test]
fn undersized_inline_editor_rejects_text_input() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor = Some(FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    app.regions.screen = Some(Rect::new(0, 0, 39, 48));

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

    assert_eq!(app.file_editor.as_ref().unwrap().text(), "notes\n");
    assert_eq!(
        app.notice.as_deref(),
        Some("Resize the terminal before editing")
    );
}

#[test]
fn input_uses_the_profile_computed_for_the_rendered_frame() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());

    app.begin_render_frame(Rect::new(0, 0, 49, 48));
    app.regions.screen = Some(Rect::new(0, 0, 120, 48));

    assert_eq!(app.layout_profile(), LayoutProfile::Single);
}

#[test]
fn workspace_open_is_blocked_while_inline_editor_exists() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor = Some(FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());

    assert!(!app.start_repository_open(root.join("other"), false));
    assert_eq!(
        app.notice.as_deref(),
        Some("Save or close the editor before opening a workspace")
    );
    assert!(!app.session.open_running());
}

#[test]
fn runs_a_custom_git_command_and_keeps_its_output() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let mut app = App::new(root.to_path_buf());

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Commit);
    assert_eq!(app.view(), View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.actions.status, CommandStatus::Input);

    app.handle_paste("rev-parse --abbrev-ref HEAD");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.actions.status, CommandStatus::Running);
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(10));
        let _ = app.poll_worker();
        if !app.session.command_running() {
            break;
        }
    }

    assert_eq!(
        app.actions.status,
        CommandStatus::Complete {
            success: true,
            exit_code: Some(0),
        }
    );
    assert_eq!(app.actions.stdout.trim(), "main");
    assert_eq!(app.actions.command, "git rev-parse --abbrev-ref HEAD");
    assert!(app.actions.input.is_empty());
    assert_eq!(app.actions.transcript.len(), 1);
    assert_eq!(app.actions.transcript[0].stdout.trim(), "main");

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.handle_paste("tatus --short");
    assert_eq!(app.actions.input, "status --short");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.actions.status, CommandStatus::Running);
    assert_eq!(app.actions.transcript.len(), 1);
    assert_eq!(app.actions.transcript[0].stdout.trim(), "main");
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(10));
        let _ = app.poll_worker();
        if !app.session.command_running() {
            break;
        }
    }
    assert_eq!(
        app.actions.status,
        CommandStatus::Complete {
            success: true,
            exit_code: Some(0),
        }
    );
    assert_eq!(app.actions.command, "git status --short");
    assert!(app.actions.input.is_empty());
    assert_eq!(app.actions.transcript.len(), 2);
    assert_eq!(
        app.actions.transcript[0].command,
        "git rev-parse --abbrev-ref HEAD"
    );
    assert_eq!(app.actions.transcript[0].stdout.trim(), "main");
    assert_eq!(app.actions.transcript[1].command, "git status --short");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn refreshes_an_already_dirty_file_when_its_contents_change_again() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    let tracked = root.join("tracked.txt");
    fs::write(&tracked, "first\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    wait_for_state(&mut app, |app| {
        app.changes
            .preview
            .text()
            .is_some_and(|text| text.contains("first"))
    });

    fs::write(&tracked, "later content\n").unwrap();
    app.session.schedule_status_check_now();
    wait_for_state(&mut app, |app| {
        app.changes
            .preview
            .text()
            .is_some_and(|text| text.contains("later"))
    });
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
