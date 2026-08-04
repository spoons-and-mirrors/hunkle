use super::*;

#[test]
fn inline_editor_renders_and_accepts_input_at_phone_width() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "first\nsecond\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.regions.diff.unwrap().width, 49);
    assert_eq!(app.regions.preview_body.unwrap().width, 46);
    let panel = app.regions.diff.unwrap();
    let body = app.regions.preview_body.unwrap();
    assert_eq!(terminal.backend().buffer()[(panel.x, body.y)].symbol(), "1");
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("EDIT"));
    assert!(screen.contains("first"));

    let body = app.regions.preview_body.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        body.x,
        body.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        body.x + 3,
        body.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        body.x + 3,
        body.y,
    ));
    assert!(app.file_editor.as_ref().unwrap().has_selection());
    app.file_editor.as_mut().unwrap().clear_selection();

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.handle_paste(" pasted ");
    assert!(app.file_editor.as_ref().unwrap().text().contains('x'));
    assert!(
        app.file_editor
            .as_ref()
            .unwrap()
            .text()
            .contains(" pasted ")
    );
}

#[test]
fn selects_visible_text_and_suppresses_clicks_after_dragging() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::write(root.join("selected.txt"), "select me\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.changes.set_diff_for_test("select me".to_owned());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let diff = app.regions.diff.unwrap();
    let buffer = terminal.backend().buffer();
    let start = (diff.y..diff.bottom())
        .find_map(|row| {
            let text: String = (diff.x..diff.right())
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            text.find("select")
                .map(|column| (diff.x + column as u16, row))
        })
        .expect("rendered preview should contain selectable text");
    let end = (start.0 + 5, start.1);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        start.0,
        start.1,
    ));
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), end.0, end.1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let buffer = terminal.backend().buffer();
    let index = usize::from(start.1) * usize::from(buffer.area.width) + usize::from(start.0);
    assert_eq!(buffer.content[index].bg, super::palette().accent);

    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), end.0, end.1));
    assert_eq!(app.take_copy_request().as_deref(), Some("select"));

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let graph = app.regions.graph.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        graph.x + 2,
        graph.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        graph.x + 4,
        graph.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        graph.x + 4,
        graph.y,
    ));
    assert_eq!(app.view, View::Changes);
    assert!(app.take_copy_request().is_some());
}

#[test]
fn inline_editor_keeps_line_numbers_in_a_fixed_gutter() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "first\nsecond\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let body = app.regions.preview_body.unwrap();
    let editor_panel = app.regions.diff.unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(editor_panel.x, editor_panel.y)].symbol(), " ");
    assert_eq!(
        buffer[(editor_panel.x, editor_panel.y)].bg,
        super::palette().canvas
    );
    let first_gutter = (body.x.saturating_sub(7)..body.x)
        .map(|x| buffer[(x, body.y)].symbol())
        .collect::<String>();
    let second_gutter = (body.x.saturating_sub(7)..body.x)
        .map(|x| buffer[(x, body.y + 1)].symbol())
        .collect::<String>();
    assert_eq!(first_gutter, "    1  ");
    assert_eq!(second_gutter, "    2  ");
}

#[test]
fn inline_editor_copies_and_comments_explicit_selections() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("code.rs"), "first();\nsecond();\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("code.rs"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    app.settings.format_on_save = false;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let body = app.regions.preview_body.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        body.x,
        body.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        body.x + 6,
        body.y + 1,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(body.x, body.y)].bg,
        super::palette().accent
    );
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        body.x + 6,
        body.y + 1,
    ));
    assert!(app.take_copy_request().is_none());

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(app.take_copy_request().as_deref(), Some("first();\nsecond"));
    assert_eq!(
        app.file_editor.as_ref().unwrap().selected_line_range(),
        Some((0, 1))
    );
    assert!(!app.should_quit);

    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::CONTROL));
    assert_eq!(
        app.file_editor.as_ref().unwrap().text(),
        "// first();\n// second();\n"
    );
    app.handle_key(KeyEvent::new(
        KeyCode::Char(';'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(
        app.file_editor.as_ref().unwrap().text(),
        "// first();\nsecond();\n"
    );
    app.handle_key(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
    assert_eq!(
        app.file_editor.as_ref().unwrap().text(),
        "// first();\nsecond();\n"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, Mode::FileEdit);
    assert_eq!(
        fs::read_to_string(root.join("code.rs")).unwrap(),
        "// first();\n// second();\n"
    );
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn double_clicking_an_editor_word_selects_it() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("code.rs"), "alpha_beta gamma\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("code.rs"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let body = app.regions.preview_body.unwrap();
    click(&mut app, body.x + 2, body.y);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        body.x + 2,
        body.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        body.x + 2,
        body.y,
    ));
    assert!(app.take_copy_request().is_none());

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(app.take_copy_request().as_deref(), Some("alpha_beta"));
}

#[test]
fn inline_editor_expands_tabs_and_maps_clicks_to_the_same_columns() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "\tvalue\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let body = app.regions.preview_body.unwrap();
    let rendered = (body.x..body.x + 9)
        .map(|x| terminal.backend().buffer()[(x, body.y)].symbol())
        .collect::<String>();
    assert_eq!(rendered, "    value");

    click(&mut app, body.x + 4, body.y);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    assert_eq!(app.file_editor.as_ref().unwrap().text(), "\tXvalue\n");
}

#[test]
fn preview_click_preserves_wrapped_position_and_scroll_through_editing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let content = (0..400)
        .map(|index| format!("word{index:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(root.join("notes.txt"), format!("{content}\n")).unwrap();
    let mut app = App::new(root.to_path_buf());
    wait_for(&mut app, |app| {
        app.changes.preview.text() == Some(format!("{content}\n").as_str())
    });
    app.changes.diff_scroll = 2;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let preview_body = app.regions.preview_body.unwrap();
    assert_eq!(app.regions.preview_scroll, 2);
    let click_column = usize::from(preview_body.width.saturating_sub(1).min(12));
    let gutter = usize::from(preview_body.width >= 72) * 7;
    let expected_column = super::text::word_wrapped_column_at(
        &content,
        usize::from(preview_body.width).saturating_sub(gutter),
        2,
        click_column.saturating_sub(gutter),
    )
    .unwrap();
    let click_position = Position::new(preview_body.x + click_column as u16, preview_body.y);
    click(&mut app, click_position.x, click_position.y);

    assert_eq!(app.mode, Mode::FileEdit);
    assert_eq!(
        app.file_editor.as_ref().unwrap().cursor_position(),
        (0, expected_column)
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let editor_body = app.regions.preview_body.unwrap();
    let editor = app.file_editor.as_ref().unwrap();
    let (cursor_row, rendered_column) = super::wrapped_editor_cursor(
        editor.text(),
        usize::from(editor_body.width),
        0,
        expected_column,
    );
    assert_eq!(cursor_row - editor.wrap_scroll_row, 0);
    assert_eq!(editor_body.y, click_position.y);
    assert_eq!(
        app.regions.editor_rows[0].source_column_at(rendered_column),
        expected_column
    );

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.changes.diff_scroll, 2);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let preview_body = app.regions.preview_body.unwrap();
    click(
        &mut app,
        preview_body.x + click_column as u16,
        preview_body.y,
    );
    app.settings.format_on_save = false;
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    wait_for(&mut app, |app| {
        app.mode == Mode::FileEdit
            && app
                .changes
                .preview
                .text()
                .is_some_and(|text| text.contains('X'))
    });
    assert_eq!(app.changes.diff_scroll, 2);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn inline_editor_renders_and_clicks_wrapped_rows() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let content = (0..80)
        .map(|index| format!("word{index:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(root.join("notes.txt"), format!("{content}\n")).unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    app.changes.diff_wrap = true;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let body = app.regions.preview_body.unwrap();
    assert!(app.regions.editor_rows.len() > 1);
    let continuation_gutter = (body.x.saturating_sub(7)..body.x)
        .map(|x| terminal.backend().buffer()[(x, body.y + 1)].symbol())
        .collect::<String>();
    assert_eq!(continuation_gutter, "       ");
    let expected_column = app.regions.editor_rows[1].source_column_at(3);
    click(&mut app, body.x + 3, body.y + 1);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));

    assert_eq!(
        app.file_editor.as_ref().unwrap().cursor_position(),
        (0, expected_column + 1)
    );
    assert_eq!(
        app.file_editor.as_ref().unwrap().text().as_bytes()[expected_column],
        b'X'
    );
}

#[test]
fn inline_editor_scrolls_past_u16_columns_without_clipping() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let content = format!("{}X\n", "a".repeat(70_000));
    fs::write(root.join("long.txt"), &content).unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("long.txt"), 1, 70_001).unwrap());
    app.mode = Mode::FileEdit;
    app.changes.diff_wrap = false;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let body = app.regions.preview_body.unwrap();
    assert_eq!(
        terminal.backend().buffer()[(body.x + body.width - 2, body.y)].symbol(),
        "X"
    );
}

#[test]
fn untracked_preview_source_lines_open_in_the_inline_editor() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::write(root.join("notes.txt"), "first\nsecond\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    wait_for(&mut app, |app| {
        app.changes
            .preview
            .text()
            .is_some_and(|text| text.contains("Untracked file:"))
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let body = app.regions.preview_body.unwrap();
    assert!(app.regions.preview_untracked);

    click(&mut app, body.x, body.y + 3);

    assert_eq!(app.mode, Mode::FileEdit);
    let editor = app.file_editor.as_ref().unwrap();
    assert_eq!(editor.path(), &RepoPath::from("notes.txt"));
    assert_eq!(editor.cursor_position().0, 1);
}

#[test]
fn preview_click_uses_the_scroll_state_from_the_rendered_frame() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::write(root.join("notes.txt"), "first\nsecond\n").unwrap();
    run_git(root, &["add", "notes.txt"]);
    run_git(
        root,
        &[
            "-c",
            "user.name=Render Test",
            "-c",
            "user.email=render@example.com",
            "commit",
            "-m",
            "initial",
        ],
    );
    let mut app = App::new(root.to_path_buf());
    app.changes.pane = LeftPane::Files;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let row = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path == "notes.txt")
        })
        .unwrap();
    click(&mut app, list.x + 2, list.y + row as u16);
    wait_for(&mut app, |app| {
        app.changes.preview.text() == Some("first\nsecond\n")
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let body = app.regions.preview_body.unwrap();

    app.changes
        .set_source_for_test("first\nsecond\n".to_owned());
    click(&mut app, body.x, body.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.notice.as_deref(),
        Some("Preview changed; click again to edit")
    );

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let body = app.regions.preview_body.unwrap();
    app.changes.diff_scroll = 1;
    click(&mut app, body.x, body.y);

    assert_eq!(app.mode, Mode::FileEdit);
    assert_eq!(app.file_editor.as_ref().unwrap().cursor_position().0, 0);
}

#[test]
fn inline_editor_wheel_scroll_keeps_the_cursor_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(
        root.join("notes.txt"),
        (0..40)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    app.changes.diff_wrap = false;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let body = app.regions.preview_body.unwrap();

    app.handle_mouse(mouse(MouseEventKind::ScrollDown, body.x, body.y));
    assert_eq!(app.file_editor.as_ref().unwrap().cursor_position(), (0, 0));
    assert_eq!(app.file_editor.as_ref().unwrap().scroll_line, 3);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.file_editor.as_ref().unwrap().scroll_line, 3);
}

#[test]
fn inline_editor_tabs_indent_and_outdent_selected_lines() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("code.rs"), "first\nsecond\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("code.rs"), 1, 0).unwrap());
    app.mode = Mode::FileEdit;
    {
        let editor = app.file_editor.as_mut().unwrap();
        editor.begin_selection();
        editor.extend_cursor(1, 6);
    }

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        app.file_editor.as_ref().unwrap().text(),
        "\tfirst\n\tsecond\n"
    );

    {
        let editor = app.file_editor.as_mut().unwrap();
        editor.set_cursor(0, 0);
        editor.begin_selection();
        editor.extend_cursor(1, 7);
    }
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.file_editor.as_ref().unwrap().text(), "first\nsecond\n");
}

#[test]
fn inline_editor_gutter_shows_live_changed_lines() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "context\nnew\nmore\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    app.file_editor =
        Some(crate::app::FileEditor::open(root, RepoPath::from("notes.txt"), 1, 0).unwrap());
    app.changes.set_diff_for_test(
        concat!(
            "@@ -1,3 +1,4 @@\n",
            " context\n",
            "-old\n",
            "+new\n",
            "+more\n",
        )
        .to_owned(),
    );
    app.mode = Mode::FileEdit;
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let body = app.regions.preview_body.unwrap();

    assert_eq!(
        terminal.backend().buffer()[(body.x - 2, body.y + 1)].symbol(),
        "~"
    );
    assert_eq!(
        terminal.backend().buffer()[(body.x - 2, body.y + 2)].symbol(),
        "+"
    );

    app.file_editor.as_mut().unwrap().set_cursor(0, 0);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(body.x - 2, body.y)].symbol(),
        "~"
    );
}

#[test]
fn inline_editor_selection_width_accounts_for_tabs() {
    assert_eq!(
        super::selected_display_range("\tvalue\n", 0, (0, 1)),
        Some((0, 4))
    );
    assert_eq!(
        super::selected_display_range("\tvalue\n", 0, (0, 2)),
        Some((0, 5))
    );
}
