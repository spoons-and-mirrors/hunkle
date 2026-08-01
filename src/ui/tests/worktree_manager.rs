use super::*;

#[test]
fn worktree_manager_renders_and_uses_semantic_rows() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main");
    let linked = directory.path().join("feature");
    fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "-b", "main"]);
    run_git(&root, &["config", "user.name", "Render Test"]);
    run_git(&root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("tracked.txt"), "first\n").unwrap();
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "initial commit"]);
    let linked_argument = linked.to_string_lossy().into_owned();
    run_git(
        &root,
        &["worktree", "add", "-b", "feature/modal", &linked_argument],
    );

    let mut app = App::new(root.clone());
    app.handle_key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT));
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Worktrees);
    wait_for(&mut app, |app| !app.worktree_manager.loading);

    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("WORKTREES"));
    assert!(screen.contains("feature/modal"));
    assert!(!screen.contains("ACTIVE REPOSITORY"));
    assert!(!screen.contains("MAIN ·"));
    assert!(!screen.contains("PRIMARY"));
    assert!(!screen.contains("2 CHECKOUTS"));
    assert!(screen.contains("WORKTREE DETAILS"));
    assert!(screen.contains("AVAILABLE ACTIONS"));
    assert!(screen.contains("Native Git worktree"));
    assert!(!screen.contains("HERDR"));

    let mut compact_terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    compact_terminal
        .draw(|frame| draw(frame, &mut app))
        .unwrap();
    let compact_screen = compact_terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(compact_screen.contains("AVAILABLE ACTIONS"));

    let rows = app.worktree_manager.rows();
    let linked_row = rows
        .iter()
        .position(|row| match row {
            WorktreeManagerRow::Worktree {
                repository,
                worktree,
            } => app.worktree_manager.repositories[*repository].worktrees[*worktree].path == linked,
            _ => false,
        })
        .unwrap();
    let target = HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
        generation: app.worktree_manager.content_generation(),
        row: linked_row,
    });
    let row_area = app.regions.hit_target_rect(target).unwrap();
    assert_eq!(row_area.height, 1);

    app.handle_mouse(mouse(MouseEventKind::Moved, row_area.x + 1, row_area.y));
    assert_eq!(app.worktree_manager.state.selected(), Some(linked_row));
    compact_terminal
        .draw(|frame| draw(frame, &mut app))
        .unwrap();
    let row_area = app.regions.hit_target_rect(target).unwrap();
    let buffer = compact_terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let selected_cell = &buffer.content
        [usize::from(row_area.y) * width + usize::from(row_area.x.saturating_add(1))];
    let selected_branch_cell = &buffer.content
        [usize::from(row_area.y) * width + usize::from(row_area.x.saturating_add(7))];
    let selected_path_cell = &buffer.content
        [usize::from(row_area.y) * width + usize::from(row_area.x.saturating_add(22))];
    assert_eq!(selected_cell.bg, super::palette().raised);
    assert_eq!(selected_branch_cell.fg, super::palette().accent);
    assert_eq!(selected_path_cell.fg, super::palette().soft);
    let selected_screen = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(selected_screen.contains("REMOVE Ready"));

    let current_row = rows
        .iter()
        .position(|row| match row {
            WorktreeManagerRow::Worktree {
                repository,
                worktree,
            } => app.worktree_manager.repositories[*repository].worktrees[*worktree].path == root,
            _ => false,
        })
        .unwrap();
    let current_area = app
        .regions
        .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
            generation: app.worktree_manager.content_generation(),
            row: current_row,
        }))
        .unwrap();
    let current_cell = &buffer.content
        [usize::from(current_area.y) * width + usize::from(current_area.x.saturating_add(1))];
    let repository_cell = &buffer.content
        [usize::from(current_area.y) * width + usize::from(current_area.x.saturating_add(2))];
    assert_eq!(current_cell.bg, super::palette().add_bg);
    assert!(repository_cell.modifier.contains(Modifier::BOLD));
    click(&mut app, row_area.x + 1, row_area.y);
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Worktrees);

    app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
    assert!(app.worktree_manager.create_dialog.is_some());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let create_dialog = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(create_dialog.contains("CREATE WORKTREE"));
    assert!(create_dialog.contains("Managed by Herdr"));
    assert!(create_dialog.contains("feature/modal"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(app.worktree_manager.remove_dialog_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let dialog = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(dialog.contains("REMOVE WORKTREE"));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    click(&mut app, 0, 0);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn worktree_manager_separates_groups_with_a_top_margin() {
    let directory = tempfile::tempdir().unwrap();
    let alpha = directory.path().join("alpha");
    let zulu = directory.path().join("zulu");
    for repository in [&alpha, &zulu] {
        fs::create_dir(repository).unwrap();
        run_git(repository, &["init", "-b", "main"]);
        run_git(repository, &["config", "user.name", "Render Test"]);
        run_git(repository, &["config", "user.email", "render@example.com"]);
        fs::write(repository.join("tracked.txt"), "first\n").unwrap();
        run_git(repository, &["add", "."]);
        run_git(repository, &["commit", "-m", "initial commit"]);
    }

    let mut app = App::new(alpha.clone());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    { "workspace_id": "w1", "label": "alpha", "number": 1 },
                    { "workspace_id": "w2", "label": "zulu", "number": 2 }
                ],
                "agents": []
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(alpha.clone());
    app.workspace_panel.workspaces[1].path = Some(zulu.clone());
    app.workspace_panel.begin_group();
    app.workspace_panel.paste("First");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.workspace_panel.begin_workspace_drag(0));
    app.workspace_panel
        .update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
    let _ = app.workspace_panel.finish_workspace_drag();
    app.workspace_panel.begin_group();
    app.workspace_panel.paste("Second");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.workspace_panel.begin_workspace_drag(1));
    app.workspace_panel
        .update_workspace_drag(Some(WorkspaceDropTarget::Group(1)));
    let _ = app.workspace_panel.finish_workspace_drag();
    app.workspace_panel.loading = true;

    let candidates = app.workspace_panel.worktree_candidates();
    let _ = app.worktree_manager.open(
        candidates,
        app.workspace_panel.linked_herdr_worktrees(),
        Some(alpha.clone()),
        app.workspace_panel.is_enabled(),
        app.workspace_panel.worktree_inventory_verified(),
    );
    app.mode = Mode::Explorer;
    app.explorer_tab = ExplorerTab::Worktrees;
    wait_for(&mut app, |app| !app.worktree_manager.loading);
    let rows = app.worktree_manager.rows();
    assert_eq!(
        rows,
        [
            WorktreeManagerRow::Group(0),
            WorktreeManagerRow::Worktree {
                repository: 0,
                worktree: 0,
            },
            WorktreeManagerRow::Group(1),
            WorktreeManagerRow::Worktree {
                repository: 1,
                worktree: 0,
            },
        ]
    );

    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app
        .regions
        .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::List))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let line_at = |y: u16| {
        (list.x..list.right())
            .map(|x| buffer.content[usize::from(y) * width + usize::from(x)].symbol())
            .collect::<String>()
    };
    let row_y = |row: usize| {
        app.regions
            .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                generation: app.worktree_manager.content_generation(),
                row,
            }))
            .unwrap()
            .y
    };
    let alpha_y = row_y(1);
    let zulu_y = row_y(3);
    assert_eq!(zulu_y - alpha_y, 3);
    let group_line = line_at(zulu_y - 1);
    let gap_line = line_at(zulu_y - 2);
    assert!(group_line.contains("SECOND"));
    assert!(
        !gap_line.contains("alpha")
            && !gap_line.contains("zulu")
            && !gap_line.contains("FIRST")
            && !gap_line.contains("SECOND")
    );
    assert!(line_at(alpha_y - 1).contains("FIRST"));
}

#[test]
fn worktree_manager_groups_repositories_into_aligned_columns() {
    let directory = tempfile::tempdir().unwrap();
    let alpha = directory.path().join("alpha");
    let zulu = directory.path().join("zulu");
    for repository in [&alpha, &zulu] {
        fs::create_dir(repository).unwrap();
        run_git(repository, &["init", "-b", "main"]);
        run_git(repository, &["config", "user.name", "Render Test"]);
        run_git(repository, &["config", "user.email", "render@example.com"]);
        fs::write(repository.join("tracked.txt"), "first\n").unwrap();
        run_git(repository, &["add", "."]);
        run_git(repository, &["commit", "-m", "initial commit"]);
    }

    let mut app = App::new(alpha.clone());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    { "workspace_id": "w1", "label": "alpha", "number": 1 },
                    { "workspace_id": "w2", "label": "zulu", "number": 2 }
                ],
                "agents": []
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(alpha.clone());
    app.workspace_panel.workspaces[1].path = Some(zulu.clone());
    app.workspace_panel.begin_group();
    app.workspace_panel.paste("Projects");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for workspace in 0..2 {
        assert!(app.workspace_panel.begin_workspace_drag(workspace));
        app.workspace_panel
            .update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
        app.workspace_panel.finish_workspace_drag();
    }
    app.workspace_panel.loading = true;

    let candidates = app.workspace_panel.worktree_candidates();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.group.as_deref() == Some("Projects"))
    );
    let _ = app.worktree_manager.open(
        candidates,
        app.workspace_panel.linked_herdr_worktrees(),
        Some(alpha.clone()),
        app.workspace_panel.is_enabled(),
        app.workspace_panel.worktree_inventory_verified(),
    );
    app.mode = Mode::Explorer;
    app.explorer_tab = ExplorerTab::Worktrees;
    wait_for(&mut app, |app| !app.worktree_manager.loading);
    let rows = app.worktree_manager.rows();
    assert_eq!(
        rows,
        [
            WorktreeManagerRow::Group(0),
            WorktreeManagerRow::Worktree {
                repository: 0,
                worktree: 0,
            },
            WorktreeManagerRow::Worktree {
                repository: 1,
                worktree: 0,
            },
        ]
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app
        .regions
        .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::List))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let line_at = |y: u16| {
        (list.x..list.right())
            .map(|x| buffer.content[usize::from(y) * width + usize::from(x)].symbol())
            .collect::<String>()
    };
    let row_y = |row: usize| {
        app.regions
            .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                generation: app.worktree_manager.content_generation(),
                row,
            }))
            .unwrap()
            .y
    };
    let group_line = line_at(row_y(1) - 1);
    let alpha_line = line_at(row_y(1));
    let zulu_line = line_at(row_y(2));
    assert!(group_line.contains("PROJECTS"));
    assert!(alpha_line.contains("alpha"));
    assert!(zulu_line.contains("zulu"));
    let column_of =
        |line: &str, needle: &str| line.find(needle).map(|byte| line[..byte].chars().count());
    let branch_column = column_of(&alpha_line, "main").unwrap();
    assert_eq!(column_of(&zulu_line, "main"), Some(branch_column));
    let path_column = column_of(&alpha_line, "/tmp").unwrap();
    assert_eq!(column_of(&zulu_line, "/tmp"), Some(path_column));
}
