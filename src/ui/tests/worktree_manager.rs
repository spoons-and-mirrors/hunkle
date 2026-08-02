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
    wait_for(&mut app, |app| !app.worktree_manager.loading());
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
    assert!(screen.contains("WORKTREE DETAILS"));
    assert!(screen.contains("AVAILABLE ACTIONS"));
    assert!(screen.contains("Native Git worktree"));

    let rows = app.worktree_manager.rows();
    let linked_row = rows
        .iter()
        .position(|row| match row {
            WorktreeManagerRow::Worktree {
                repository,
                worktree,
            } => {
                app.worktree_manager.repositories()[*repository].worktrees[*worktree].path == linked
            }
            WorktreeManagerRow::Status(_) => false,
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
}
