use std::{process::Command, thread};

use crate::media::MediaPreviewProtocol;

use super::*;

#[test]
fn clearing_hit_targets_removes_overlaps_but_keeps_adjacent_targets() {
    let mut regions = Regions::default();
    regions.register_hit_target(HitTarget::CommitMessageGenerate, Rect::new(0, 0, 4, 1));
    regions.register_hit_target(HitTarget::MarkdownPreviewToggle, Rect::new(3, 0, 4, 1));
    regions.register_hit_target(
        HitTarget::Graph(GraphHitTarget::AuthorHeader),
        Rect::new(7, 0, 2, 1),
    );

    regions.clear_hit_targets_in(Rect::new(4, 0, 3, 1));

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
    assert_eq!(app.view, View::Changes);
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
    assert!(!app.agents_visible);
    assert_eq!(app.notice.as_deref(), Some("Agents hidden"));

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
fn creates_renames_drags_and_deletes_files_from_the_files_pane() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let renamed = " renamed.txt";
    fs::write(root.join("old.txt"), "content\n").unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    let mut app = App::new(root.to_path_buf());
    app.view = View::Changes;
    app.changes.pane = LeftPane::Files;

    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
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
    assert_eq!(app.view, View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Files);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view, View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    wait_for_state(&mut app, |app| app.notice.as_deref() != Some("Refreshing…"));
    assert_eq!(app.view, View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view, View::Changes);

    fs::write(root.join("tracked.txt"), "edited\n").unwrap();
    let mut dirty_app = App::new(root.to_path_buf());
    assert_eq!(dirty_app.view, View::Changes);
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);

    fs::write(root.join("tracked.txt"), "base\n").unwrap();
    dirty_app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    wait_for_state(&mut dirty_app, |app| {
        app.repository().is_some_and(|repo| repo.changes.is_empty())
    });
    assert_eq!(dirty_app.view, View::Changes);
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);
}

#[test]
fn background_startup_selects_the_pane_after_repository_details_load() {
    let clean_directory = tempfile::tempdir().unwrap();
    initialize_repository(clean_directory.path());
    let mut clean_app = App::opening(clean_directory.path().to_path_buf());
    assert_eq!(clean_app.mode, Mode::Normal);
    assert!(clean_app.workspace_loading_initial_state());
    wait_for_state(&mut clean_app, |app| {
        app.repository().is_some_and(|repo| repo.details_ready)
    });
    assert!(!clean_app.workspace_loading_initial_state());
    assert_eq!(clean_app.changes.pane, LeftPane::Files);

    let dirty_directory = tempfile::tempdir().unwrap();
    initialize_repository(dirty_directory.path());
    fs::write(dirty_directory.path().join("tracked.txt"), "edited\n").unwrap();
    let mut dirty_app = App::opening(dirty_directory.path().to_path_buf());
    assert_eq!(dirty_app.mode, Mode::Normal);
    assert!(dirty_app.workspace_loading_initial_state());
    wait_for_state(&mut dirty_app, |app| {
        app.repository().is_some_and(|repo| repo.details_ready)
    });
    assert!(!dirty_app.workspace_loading_initial_state());
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);
}

#[test]
fn worktree_actions_preserve_the_visible_graph() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    initialize_repository(root);
    fs::write(root.join("tracked.txt"), "edited\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.view = View::Graph;
    app.stage_all();
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.changes.iter().all(|change| change.staged))
    });
    assert_eq!(app.view, View::Graph);

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    wait_for_state(&mut app, |app| {
        app.repository()
            .is_some_and(|repo| repo.changes.iter().all(|change| !change.staged))
    });
    assert_eq!(app.view, View::Graph);

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
    assert_eq!(app.view, View::Graph);
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
fn primary_navigation_has_stable_precedence_and_edits_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    initialize_repository(directory.path());
    fs::write(directory.path().join("tracked.txt"), "edited\n").unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.mode = Mode::Normal;
    app.settings = Settings::default();
    app.settings_store = SettingsStore::at(path.clone());

    assert!(app.agents_visible);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(!app.agents_visible);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.agents_visible);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.view, View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Files);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view, View::Graph);
    app.graph_commit_open = true;
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.agents_pane_visible());
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(app.view, View::Graph);
    assert!(app.graph_commit_open);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    assert_eq!(app.view, View::Graph);
    assert!(app.graph_commit_open);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view, View::Changes);
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view, View::Graph);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    app.mode = Mode::Commit;
    app.commit_input.clear();
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Commit);
    assert_eq!(app.commit_input.text(), "g");
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(app.view, View::Graph);

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
            agent_time_display: settings::AgentTimeDisplay::LatestLoop,
            agents_height: 7,
            graph_lane_width: 0,
            graph_description_width: 0,
            graph_changes_width: 11,
            graph_date_width: 11,
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
    app.view = View::Changes;
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
fn shortcut_settings_rebind_reset_and_persist_commands() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    let mut app = App::new(directory.path().to_path_buf());
    app.settings = Settings::default();
    app.settings_store = SettingsStore::at(path);
    app.mode = Mode::Settings;
    app.settings_page = SettingsPage::Shortcuts;
    app.shortcut_selection = Shortcuts::definitions()
        .iter()
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
    app.settings_page = SettingsPage::Shortcuts;
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
    app.settings_page = SettingsPage::OpenCode;

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
    assert!(app.opencode_error.is_some());
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
        if app.changes.diff.matches("@@").count() == 2 {
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
            && app.changes.diff.contains("changed second")
            && !app.changes.diff.contains("changed first")
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(app.changes.hunk_selection, Some(0));
    assert!(app.changes.diff.contains("changed second"));
    assert!(!app.changes.diff.contains("changed first"));
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
fn undersized_inline_editor_rejects_text_input() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "notes\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor = Some(FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    app.regions.screen = Some(Rect::new(0, 0, 50, 12));

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

    assert_eq!(app.file_editor.as_ref().unwrap().text(), "notes\n");
    assert_eq!(
        app.notice.as_deref(),
        Some("Resize the terminal before editing")
    );
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
    assert_eq!(app.view, View::Changes);
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
    wait_for_state(&mut app, |app| app.changes.diff.contains("first"));

    fs::write(&tracked, "later content\n").unwrap();
    app.session.schedule_status_check_now();
    wait_for_state(&mut app, |app| app.changes.diff.contains("later"));
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
