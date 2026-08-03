use super::*;

#[test]
fn renders_sqlite_databases_from_the_files_view() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
             INSERT INTO people (name) VALUES ('Ada'), ('Grace');",
        )
        .unwrap();
    drop(connection);

    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| app.changes.sqlite_browser.is_some());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(screen.contains("DATABASE  app.sqlite  read-only"));
    assert!(screen.contains("OBJECTS  1"));
    assert!(screen.contains("people  TABLE"));
    assert!(screen.contains("name · TEXT"));
    assert!(screen.contains("Ada"));
    assert!(screen.contains("Enter explore"));
}

#[test]
fn explores_and_pages_sqlite_databases_with_keys_and_mouse() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT); \
             WITH RECURSIVE sequence(value) AS ( \
                VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 105 \
             ) INSERT INTO events SELECT value, printf('event-%03d', value) FROM sequence; \
             CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT); \
             INSERT INTO people VALUES (1, 'Ada');",
        )
        .unwrap();
    drop(connection);

    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| app.changes.sqlite_browser.is_some());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.changes.sqlite_browser.as_ref().unwrap().focus,
        SqliteFocus::Rows
    );
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes
            .sqlite_browser
            .as_ref()
            .and_then(|browser| browser.page.as_ref())
            .is_some_and(|page| page.key.offset == 100 && page.rows.len() == 5)
    });

    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let objects = app.regions.sqlite_objects.unwrap();
    click(&mut app, objects.x + 2, objects.y + 1);
    assert!(app.changes.sqlite_browser.as_ref().unwrap().active);
    assert_eq!(
        app.changes
            .sqlite_browser
            .as_ref()
            .unwrap()
            .selected_object()
            .unwrap()
            .name,
        "people"
    );
    wait_for(&mut app, |app| {
        app.changes
            .sqlite_browser
            .as_ref()
            .and_then(|browser| browser.page.as_ref())
            .is_some_and(|page| page.key.object == "people" && page.rows[0][1] == "Ada")
    });
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.changes.sqlite_browser.as_ref().unwrap().active);
}

#[test]
fn narrow_sqlite_detail_returns_to_the_files_panel() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL);")
        .unwrap();
    drop(connection);

    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| app.changes.sqlite_browser.is_some());
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.explorer_list.is_some());
    assert!(app.regions.diff.is_none());

    let list = app.regions.explorer_list.unwrap();
    click(&mut app, list.x + 2, list.y);
    click(&mut app, list.x + 2, list.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.changes.sqlite_browser.as_ref().unwrap().active);
    assert_eq!(app.regions.diff.unwrap().width, 49);
    assert!(app.regions.explorer_list.is_none());

    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(!app.changes.sqlite_browser.as_ref().unwrap().active);
    assert!(app.regions.explorer_list.is_some());
    assert!(app.regions.diff.is_none());
}
