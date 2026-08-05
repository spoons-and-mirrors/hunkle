use super::*;

#[test]
fn narrow_layout_drills_from_changes_into_a_full_width_detail_panel() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Narrow Test"]);
    run_git(root, &["config", "user.email", "narrow@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.worktree_list.unwrap();
    assert_eq!(app.regions.worktree.unwrap().width, 49);
    assert!(app.regions.diff.is_none());
    assert!(app.regions.splitter.is_none());
    assert!(app.regions.agents_list.is_none());
    let tabs = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    let tab_padding = &terminal.backend().buffer()[(tabs.x, tabs.y - 1)];
    assert_eq!(tab_padding.symbol(), "▄");
    assert_eq!(tab_padding.fg, super::palette().panel);
    assert_eq!(tab_padding.bg, super::palette().canvas);

    let row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.label == "tracked.txt")
        .unwrap();
    let y = list.y + (row - app.changes.worktree_scroll) as u16;
    click(&mut app, list.x + 2, y);
    wait_for_preview(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert!(app.regions.worktree.is_none());
    assert_eq!(app.regions.diff.unwrap().width, 49);
    assert!(app.regions.worktree_list.is_none());
    assert!(app.regions.commit.is_none());
    assert!(app.regions.changes.is_none());
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!screen.contains("STAGE ALL"));

    let selected = app.changes.worktree_state.selected();
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.changes.worktree_state.selected(), selected);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.worktree_list.is_some());
    assert!(app.regions.diff.is_none());

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.worktree.is_none());
    assert_eq!(app.regions.diff.unwrap().width, 49);

    let preview_origin = app.changes.preview.origin().clone();
    terminal.backend_mut().resize(100, 48);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.worktree.is_some());
    assert!(app.regions.diff.is_some());
    assert!(app.regions.splitter.is_some());
    assert_eq!(app.changes.worktree_state.selected(), selected);
    assert_eq!(app.changes.preview.origin(), &preview_origin);

    terminal.backend_mut().resize(49, 48);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.worktree.is_none());
    assert_eq!(app.regions.diff.unwrap().width, 49);
    assert_eq!(app.changes.worktree_state.selected(), selected);
    assert_eq!(app.changes.preview.origin(), &preview_origin);

    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.worktree_list.is_some());
    assert!(app.regions.diff.is_none());
}

#[test]
fn narrow_files_drag_to_scroll_and_double_click_to_open() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    for index in 0..50 {
        let content = (0..100)
            .map(|line| format!("file {index} line {line}\n"))
            .collect::<String>();
        fs::write(root.join(format!("file-{index:02}.txt")), content).unwrap();
    }

    let mut app = App::new(root.to_path_buf());
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 2,
        list.y + 10,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        list.x + 2,
        list.y + 3,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        list.x + 2,
        list.y + 3,
    ));
    assert_eq!(app.changes.explorer_scroll, 7);
    assert!(app.take_copy_request().is_none());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.diff.is_none());

    let list = app.regions.explorer_list.unwrap();
    click(&mut app, list.x + 2, list.y + 2);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.explorer_list.is_some());
    assert!(app.regions.diff.is_none());

    click(&mut app, list.x + 2, list.y + 2);
    wait_for_preview(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.explorer_list.is_none());
    let detail = app.regions.diff.unwrap();
    assert!(app.regions.diff_scroll_max > 0);

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        detail.x + 2,
        detail.bottom() - 2,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        detail.x + 2,
        detail.bottom() - 7,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        detail.x + 2,
        detail.bottom() - 7,
    ));
    assert_eq!(app.changes.diff_scroll, 5);
    assert!(app.take_copy_request().is_none());
}

#[test]
fn right_clicking_worktree_rows_toggles_staging() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Right Click Test"]);
    run_git(root, &["config", "user.email", "right-click@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    for staged in [true, false] {
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let repo = app.repository().unwrap();
        let row = app
            .changes
            .worktree_rows(repo)
            .iter()
            .position(|row| {
                row.change_index
                    .and_then(|index| repo.changes.get(index))
                    .is_some_and(|change| change.path == "tracked.txt")
            })
            .unwrap();
        let target = app.changes.worktree_row_target(row);
        let rect = app
            .regions
            .hit_target_rect(HitTarget::Changes(target))
            .unwrap();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Right),
            rect.x,
            rect.y,
        ));
        wait_for(&mut app, |app| {
            app.repository().is_some_and(|repo| {
                repo.changes
                    .iter()
                    .find(|change| change.path == "tracked.txt")
                    .is_some_and(|change| change.staged == staged)
            })
        });
    }
}

#[test]
fn graph_replacement_clears_hidden_diff_targets() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Graph Target Test"]);
    run_git(root, &["config", "user.email", "graph-target@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    wait_for_preview(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let target = app.changes.hunk_action_target(0);
    let action = app
        .regions
        .hit_target_rect(HitTarget::Changes(target))
        .unwrap();

    app.set_view_for_test(View::Graph);
    app.set_graph_commit_open_for_test(false);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Changes(target))
            .is_none()
    );
    click(&mut app, action.x, action.y);
    for _ in 0..20 {
        let _ = app.poll_worker();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.staged)
    );
}

#[test]
fn decoupled_sidebar_passes_clear_inactive_content_and_targets() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Sidebar Test"]);
    run_git(root, &["config", "user.email", "sidebar@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 35)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    assert_eq!(app.changes.preview.pane(), LeftPane::Worktree);
    assert!(app.regions.diff.is_some());
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Changes(ChangesHitTarget::StageAll))
            .is_none()
    );
    assert!(app.regions.files_add.is_some());
    let sidebar = app.regions.worktree.unwrap();
    let mut sidebar_text = String::new();
    for y in sidebar.y..sidebar.bottom() {
        for x in sidebar.x..sidebar.right() {
            sidebar_text.push_str(terminal.backend().buffer()[(x, y)].symbol());
        }
    }
    assert!(!sidebar_text.contains("STAGE ALL"));

    let explorer = app.regions.explorer_list.unwrap();
    let file_row = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| row.file_path.is_some())
        .unwrap();
    click(&mut app, explorer.x + 2, explorer.y + file_row as u16);
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.sidebar_pane(), LeftPane::Worktree);
    assert_eq!(app.changes.preview.pane(), LeftPane::Files);
    assert!(app.regions.diff.is_some());
    assert!(app.regions.files_add.is_none());
    assert!(app.regions.files_root.is_none());
    let mut sidebar_text = String::new();
    for y in sidebar.y..sidebar.bottom() {
        for x in sidebar.x..sidebar.right() {
            sidebar_text.push_str(terminal.backend().buffer()[(x, y)].symbol());
        }
    }
    assert!(!sidebar_text.contains("NEW  +"));
}

#[test]
fn renders_colored_file_type_icons_in_the_files_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for path in [
        "main.rs",
        "app.ts",
        "config.json",
        "README.md",
        "styles.css",
        "run.sh",
        "asset.png",
        "notes.xyz",
        "readme_parser.rs",
    ] {
        fs::write(root.join(path), "fixture\n").unwrap();
    }

    let mut app = App::new(root.to_path_buf());
    app.agents_visible = false;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    for (path, symbol, color) in [
        ("main.rs", "R", super::palette().orange),
        ("app.ts", "T", super::palette().cyan),
        ("config.json", "{", super::palette().yellow),
        ("README.md", "#", super::palette().cyan),
        ("styles.css", "#", super::palette().purple),
        ("run.sh", ">", super::palette().green),
        ("asset.png", "@", super::palette().purple),
        ("notes.xyz", "?", super::palette().faint),
        ("readme_parser.rs", "R", super::palette().orange),
    ] {
        let row_index = rows
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|file| file == path))
            .unwrap();
        let row = &rows[row_index];
        let x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        let icon = &terminal.backend().buffer()[(x, y)];
        assert_eq!(icon.symbol(), symbol, "{path}");
        assert_eq!(icon.fg, color, "{path}");
    }
}

#[test]
fn keeps_file_tree_connectors_faint_and_folder_names_bright() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("src/nested/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("root.txt"), "root\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.agents_visible = false;
    let src = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src")
        })
        .unwrap();
    app.changes.explorer_state.select(Some(src));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.explorer_rows().iter().any(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src/nested")
        })
    });
    let nested = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src/nested")
        })
        .unwrap();
    app.changes.explorer_state.select(Some(nested));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.explorer_rows().iter().any(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path == "src/nested/main.rs")
        })
    });

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();

    let src = rows
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src")
        })
        .unwrap();
    let src_y = list.y + src.saturating_sub(app.changes.explorer_scroll) as u16;
    assert_eq!(
        terminal.backend().buffer()[(list.x + 2, src_y)].fg,
        super::palette().ink
    );

    for path in ["root.txt", "src/nested/main.rs"] {
        let row_index = rows
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|file| file == path))
            .unwrap();
        let row = &rows[row_index];
        let x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16 + 2;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        assert_eq!(
            terminal.backend().buffer()[(x, y)].fg,
            super::palette().soft,
            "{path}"
        );
    }

    let mut saw_connector = false;
    for (row_index, row) in rows.iter().enumerate() {
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        for (offset, character) in row.prefix.chars().enumerate() {
            if !character.is_whitespace() {
                saw_connector = true;
                assert_eq!(
                    terminal.backend().buffer()[(list.x + offset as u16, y)].fg,
                    super::palette().faint
                );
            }
        }
    }
    assert!(saw_connector);
}

#[test]
fn colors_changed_files_in_the_files_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("modified.txt"), "original\n").unwrap();
    fs::write(root.join("deleted.txt"), "deleted\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    fs::write(root.join("modified.txt"), "changed\n").unwrap();
    fs::write(root.join("added.txt"), "added\n").unwrap();
    run_git(root, &["add", "added.txt"]);
    fs::write(root.join("new.txt"), "new\n").unwrap();
    run_git(root, &["rm", "deleted.txt"]);
    fs::write(root.join("deleted.txt"), "replacement\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.set_sidebar_pane_for_test(LeftPane::Files);
    app.agents_visible = false;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    for (path, expected) in [
        ("added.txt", super::palette().accent),
        ("deleted.txt", super::palette().red),
        ("modified.txt", super::palette().yellow),
        ("new.txt", super::palette().green),
    ] {
        let row_index = rows
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|file| file == path))
            .unwrap();
        let row = &rows[row_index];
        let icon_x = list.x + row.prefix.chars().count() as u16;
        let x = icon_x + 2;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        assert_eq!(
            terminal.backend().buffer()[(icon_x, y)].fg,
            expected,
            "{path} icon"
        );
        assert_eq!(terminal.backend().buffer()[(x, y)].fg, expected, "{path}");
    }
}

#[test]
fn refreshes_file_and_folder_colors_after_the_worktree_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Refresh Test"]);
    run_git(root, &["config", "user.email", "refresh@example.com"]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    let mut app = App::new(root.to_path_buf());
    app.set_sidebar_pane_for_test(LeftPane::Files);
    app.agents_visible = false;
    fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    app.session.schedule_status_check_now();
    wait_for(&mut app, |app| {
        app.repository().is_some_and(|repo| {
            repo.changes
                .iter()
                .any(|change| change.path == "src/main.rs" && change.code == 'M')
        })
    });

    let src = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src")
        })
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let row = &app.changes.explorer_rows()[src];
    let folder_x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16;
    let folder_y = list.y + src.saturating_sub(app.changes.explorer_scroll) as u16;
    assert_eq!(
        terminal.backend().buffer()[(folder_x, folder_y)].fg,
        super::palette().yellow
    );

    app.changes.explorer_state.select(Some(src));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.explorer_rows().iter().any(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path == "src/main.rs")
        })
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    let main = rows
        .iter()
        .position(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path == "src/main.rs")
        })
        .unwrap();
    let row = &rows[main];
    let icon_x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16;
    let label_x = icon_x + 2;
    let y = list.y + main.saturating_sub(app.changes.explorer_scroll) as u16;
    assert_eq!(
        terminal.backend().buffer()[(icon_x, y)].fg,
        super::palette().yellow
    );
    assert_eq!(
        terminal.backend().buffer()[(label_x, y)].fg,
        super::palette().yellow
    );
}

#[test]
fn shows_worktree_file_status_letters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("modified.txt"), "original\n").unwrap();
    fs::write(root.join("deleted.txt"), "deleted\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    fs::write(root.join("modified.txt"), "changed\n").unwrap();
    fs::remove_file(root.join("deleted.txt")).unwrap();
    fs::write(root.join("new.txt"), "new\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.settings.agents_height = 9;
    let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.worktree_list.unwrap();
    let rows = app.changes.worktree_rows(app.repository().unwrap());
    for (path, status, color) in [
        ("deleted.txt", 'D', super::palette().red),
        ("modified.txt", 'M', super::palette().yellow),
        ("new.txt", 'U', super::palette().green),
    ] {
        let row_index = rows.iter().position(|row| row.label == path).unwrap();
        let row = &rows[row_index];
        let y = list.y + row_index.saturating_sub(app.changes.worktree_scroll) as u16;
        let status_x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16;
        assert_eq!(
            terminal.backend().buffer()[(status_x, y)].symbol(),
            status.to_string()
        );
        assert_eq!(terminal.backend().buffer()[(status_x, y)].fg, color);
    }
}

#[test]
fn colors_collapsed_folders_for_the_changes_they_contain() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    for path in ["modified/file.txt", "deleted/file.txt", "deleted/keep.txt"] {
        fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        fs::write(root.join(path), "original\n").unwrap();
    }
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    fs::write(root.join("modified/file.txt"), "changed\n").unwrap();
    fs::create_dir_all(root.join("added")).unwrap();
    fs::write(root.join("added/file.txt"), "added\n").unwrap();
    run_git(root, &["add", "added/file.txt"]);
    fs::create_dir_all(root.join("untracked")).unwrap();
    fs::write(root.join("untracked/file.txt"), "new\n").unwrap();
    run_git(root, &["rm", "deleted/file.txt"]);

    let mut app = App::new(root.to_path_buf());
    app.set_sidebar_pane_for_test(LeftPane::Files);
    app.agents_visible = false;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    for (path, expected) in [
        ("added", super::palette().accent),
        ("deleted", super::palette().red),
        ("modified", super::palette().yellow),
        ("untracked", super::palette().green),
    ] {
        let row_index = rows
            .iter()
            .position(|row| {
                row.directory_path
                    .as_ref()
                    .is_some_and(|directory| directory == path)
            })
            .unwrap();
        assert_eq!(rows[row_index].directory_expanded, Some(false));
        let x = list.x + rows[row_index].prefix.chars().count() as u16;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        assert_eq!(terminal.backend().buffer()[(x, y)].fg, expected, "{path}");
    }
}

#[test]
fn files_click_waits_for_release_without_styling_every_file_as_a_drop_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("first.txt"), "first\n").unwrap();
    fs::write(root.join("second.txt"), "second\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let selected_before = app.changes.explorer_state.selected();
    let target = app
        .changes
        .explorer_rows()
        .iter()
        .enumerate()
        .find_map(|(index, row)| {
            (row.file_path.is_some() && Some(index) != selected_before).then_some(index)
        })
        .unwrap();
    let y = list.y + target.saturating_sub(app.changes.explorer_scroll) as u16;

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 1,
        y,
    ));
    assert_eq!(app.changes.explorer_state.selected(), selected_before);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    for (index, row) in app.changes.explorer_rows().iter().enumerate() {
        if row.file_path.is_some() && Some(index) != selected_before {
            let y = list.y + index.saturating_sub(app.changes.explorer_scroll) as u16;
            assert_ne!(
                terminal.backend().buffer()[(list.x, y)].bg,
                super::palette().inactive_selected
            );
        }
    }

    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), list.x + 1, y));
    assert_eq!(app.changes.explorer_state.selected(), Some(target));
}

#[test]
fn opens_plain_directories_as_file_workspaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("config/nested")).unwrap();
    fs::write(root.join("README.md"), "local workspace\n").unwrap();
    fs::write(root.join("config/nested/settings.toml"), "theme = 'test'\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.repository().unwrap().is_local());
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    wait_for_preview(&mut app);
    assert_eq!(app.changes.preview.text(), Some("local workspace\n"));

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("CHANGES"));
    assert!(screen.contains("FILES"));
    assert!(screen.contains("README.md"));
    assert!(screen.contains("local workspace"));

    let add = app.regions.files_add.unwrap();
    click(&mut app, add.x, add.y);
    assert_eq!(app.mode, Mode::Files);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let popover = app.regions.file_dialog_overlay.unwrap();
    assert_eq!(popover.y, add.bottom());
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!screen.contains("ADD TO FILES"));
    assert!(screen.contains("New file"));
    assert!(screen.contains("New folder"));
    let new_file = app.regions.file_dialog_primary.unwrap();
    click(&mut app, new_file.x, new_file.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("NEW FILE"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    let worktree_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    click(&mut app, worktree_tab.x, worktree_tab.y);
    assert_eq!(app.sidebar_pane(), LeftPane::Worktree);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!screen.contains("Working tree clean"));
    assert!(screen.contains("LOCAL WORKSPACE"));
    assert!(screen.contains("Local file workspace"));
}

#[test]
fn fuzzy_searches_and_opens_repository_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::create_dir_all(root.join("src/components")).unwrap();
    fs::write(
        root.join("src/components/profile_card.rs"),
        "pub struct ProfileCard;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/components/button.rs"),
        "pub struct Button;\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "search fixture\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::RepositorySearch);
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    assert_eq!(app.file_search.query.text(), "/");
    for character in "ab".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    assert_eq!(app.file_search.query.text(), "/aXb");
    assert_eq!(app.view(), View::RepositorySearch);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view(), View::Changes);

    app.set_view_for_test(View::Graph);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view(), View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let graph = app.regions.graph.unwrap();
    click(&mut app, graph.x, graph.y);
    assert_eq!(app.view(), View::Changes);
    app.set_view_for_test(View::Graph);
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for character in "profile card".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    for _ in 0..100 {
        let _ = app.poll_worker();
        if !app.file_search.searching {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.view(), View::RepositorySearch);
    assert!(!app.file_search.searching);
    assert_eq!(app.file_search.match_count, 1);
    let mut narrow = Terminal::new(TestBackend::new(60, 16)).unwrap();
    narrow.draw(|frame| draw(frame, &mut app)).unwrap();
    let narrow_screen: String = narrow
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(narrow_screen.contains("REPOSITORY"));
    assert!(narrow_screen.contains("Esc back"));
    assert!(app.file_search.preview_path.is_none());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("REPOSITORY"));
    assert!(screen.contains("FILES"));
    assert!(screen.contains("TEXT"));
    assert!(screen.contains("profile_card.rs"));
    assert!(screen.contains("src/components"));
    assert!(app.regions.file_search.is_some());
    assert!(app.regions.file_search_list.is_some());

    for _ in 0..100 {
        let _ = app.poll_worker();
        if !app.file_search.preview_loading {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("pub struct ProfileCard"));

    let selected_row = app.file_search.state.selected().unwrap();
    let result = app
        .regions
        .hit_target_rect(HitTarget::FileSearch(
            crate::app::FileSearchHitTarget::Result {
                generation: app.file_search.target_generation(),
                row: selected_row,
            },
        ))
        .unwrap();
    click(&mut app, result.x, result.y);
    assert_eq!(app.view(), View::RepositorySearch);
    click(&mut app, result.x, result.y);
    wait_for_preview(&mut app);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view(), View::Changes);
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    assert_eq!(
        app.selected_explorer_file_path().map(RepoPath::display),
        Some("src/components/profile_card.rs".to_owned())
    );
    assert_eq!(
        app.changes.preview.text(),
        Some("pub struct ProfileCard;\n")
    );
    assert!(
        app.changes
            .explorer_rows()
            .iter()
            .any(|row| row.label == "profile_card.rs")
    );
}

#[test]
fn repository_text_search_opens_the_matching_source_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let content = (1..=40)
        .map(|line| {
            if line == 30 {
                "pub struct Needle;".to_owned()
            } else {
                format!("// source line {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join("symbols.rs"), format!("{content}\n")).unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    for character in "Needle".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    for _ in 0..100 {
        let _ = app.poll_worker();
        if !app.file_search.searching {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!app.file_search.searching);
    assert_eq!(app.file_search.text_match_count, 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("symbols.rs:30"));
    assert!(screen.contains("Needle"));

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for_preview(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.selected_explorer_file_path().map(RepoPath::display),
        Some("symbols.rs".to_owned())
    );
    assert!(app.changes.diff_scroll > 0);
}

#[test]
fn left_pane_files_take_over_the_preview_from_graph() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Graph Test"]);
    run_git(root, &["config", "user.email", "graph@example.com"]);
    fs::write(root.join("tracked.txt"), "first\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    app.set_view_for_test(View::Graph);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let worktree = app.regions.worktree_list.unwrap();
    let file_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.change_index.is_some())
        .unwrap();
    let stage_target = app.changes.worktree_stage_target(file_row);
    let status = app
        .regions
        .hit_target_rect(HitTarget::Changes(stage_target))
        .unwrap();
    let file_y = worktree.y + (file_row - app.changes.worktree_scroll) as u16;
    click(&mut app, status.x, file_y);
    assert_eq!(app.view(), View::Graph, "staging should not close Graph");

    click(&mut app, worktree.x + 3, file_y);
    assert_eq!(app.view(), View::Changes);
    assert!(!app.graph_commit_open());

    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    app.set_view_for_test(View::Graph);
    app.set_graph_commit_open_for_test(true);
    app.mode = Mode::Normal;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let explorer = app.regions.explorer_list.unwrap();
    let file_row = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| row.file_path.is_some())
        .unwrap();
    click(&mut app, explorer.x + 3, explorer.y + file_row as u16);
    wait_for_preview(&mut app);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view(), View::Changes);
    assert!(!app.graph_commit_open());
    assert_eq!(app.changes.preview.text(), Some("changed\n"));
}

#[test]
fn double_clicking_worktree_files_opens_them_in_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Double Click Test"]);
    run_git(root, &["config", "user.email", "double-click@example.com"]);
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/unstaged.txt"), "initial\n").unwrap();
    fs::write(root.join("staged.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("nested/unstaged.txt"), "unstaged content\n").unwrap();
    fs::write(root.join("staged.txt"), "staged content\n").unwrap();
    run_git(root, &["add", "staged.txt"]);

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 50)).unwrap();

    for (path, content) in [
        ("nested/unstaged.txt", "unstaged content\n"),
        ("staged.txt", "staged content\n"),
    ] {
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let worktree = app.regions.worktree_list.unwrap();
        let repo = app.repository().unwrap();
        let row = app
            .changes
            .worktree_rows(repo)
            .iter()
            .position(|row| {
                row.change_index
                    .and_then(|index| repo.changes.get(index))
                    .is_some_and(|change| change.path == path)
            })
            .unwrap();
        let y = worktree.y + (row - app.changes.worktree_scroll) as u16;
        assert!(worktree.contains(Position::new(worktree.x + 4, y)));

        click(&mut app, worktree.x + 4, y);
        assert_eq!(app.sidebar_pane(), LeftPane::Worktree);
        click(&mut app, worktree.x + 4, y);
        wait_for_preview(&mut app);

        assert_eq!(app.sidebar_pane(), LeftPane::Files);
        assert_eq!(app.view(), View::Changes);
        assert_eq!(
            app.selected_explorer_file_path().map(RepoPath::display),
            Some(path.to_owned())
        );
        assert_eq!(app.changes.preview.text(), Some(content));

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(app.sidebar_pane(), LeftPane::Worktree);
    }
}

#[test]
fn renders_markdown_files_and_toggles_back_to_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut markdown = "# Markdown Title\n\nA **strong** [link](https://example.com).\n".to_owned();
    for section in 1..=40 {
        markdown.push_str(&format!(
            "\n## Section {section}\n\nParagraph {section} has enough content to remain visible.\n"
        ));
    }
    fs::write(root.join("README.md"), markdown).unwrap();

    let mut app = App::new(root.to_path_buf());
    app.settings.worktree_width = 48;
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let explorer = app.regions.explorer_list.unwrap();
    click(&mut app, explorer.x + 1, explorer.y);
    wait_for_preview(&mut app);
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    assert_eq!(
        app.selected_explorer_file_path().map(RepoPath::display),
        Some("README.md".to_owned())
    );

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let preview_button = (0..30)
        .find_map(|row| {
            let text: String = (0..100)
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            text.find("Preview").map(|column| (column as u16 + 1, row))
        })
        .expect("Markdown files should show a Preview button");
    let source: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(source.contains("# Markdown Title"));

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.markdown_preview_rendered());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Source"));
    assert!(rendered.contains("Markdown Title"));
    assert!(!rendered.contains("# Markdown Title"));
    assert!(rendered.contains("https://"));
    assert!(rendered.contains("    1  Markdown Title"));

    app.changes.diff_scroll = 7;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.changes.diff_scroll, 7);
    click(&mut app, preview_button.0, preview_button.1);
    assert!(!app.markdown_preview_rendered());
    assert_eq!(app.changes.diff_scroll, 0);
    app.changes.diff_scroll = 40;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.changes.diff_scroll, 40);

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.markdown_preview_rendered());
    assert_eq!(app.changes.diff_scroll, 7);
    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(!app.markdown_preview_rendered());
    assert_eq!(app.changes.diff_scroll, 40);

    terminal.backend_mut().resize(50, 10);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_at(Position::new(preview_button.0, preview_button.1))
            .is_none()
    );

    terminal.backend_mut().resize(100, 30);
    app.set_view_for_test(View::Graph);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_at(Position::new(preview_button.0, preview_button.1))
            .is_none()
    );
}
