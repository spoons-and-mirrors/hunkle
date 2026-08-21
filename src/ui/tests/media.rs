use super::*;

#[test]
fn renders_static_media_and_clears_it_for_text_and_overlays() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let image = image::RgbaImage::from_fn(40, 40, |_x, y| {
        if (y / 10) % 2 == 0 {
            image::Rgba([220, 40, 30, 255])
        } else {
            image::Rgba([20, 80, 210, 255])
        }
    });
    image.save(root.join("a-preview.png")).unwrap();
    fs::write(root.join("b-notes.txt"), "plain text preview\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.settings.media_preview_protocol = crate::media::MediaPreviewProtocol::Halfblocks;
    assert_eq!(
        app.selected_explorer_file_path().map(|path| path.display()),
        Some("a-preview.png".to_string())
    );
    wait_for(&mut app, |app| app.changes.preview.image(false).is_some());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    wait_for_halfblock_render(&mut terminal, &mut app);
    let preview_body = app.regions.diff.unwrap();
    let transition_y = usize::from(preview_body.bottom());
    let image_cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .filter(|(index, cell)| {
            cell.symbol() == "▀" && index / 100 != 2 && index / 100 != transition_y
        })
        .collect();
    assert!(!image_cells.is_empty());
    assert!(image_cells.iter().all(|(index, _)| {
        let x = (*index % 100) as u16;
        let y = (*index / 100) as u16;
        x >= preview_body.x
            && x < preview_body.right()
            && y >= preview_body.y
            && y < preview_body.bottom()
    }));

    app.mode = Mode::Settings;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        !terminal
            .backend()
            .buffer()
            .content
            .iter()
            .enumerate()
            .any(|(index, cell)| {
                cell.symbol() == "▀" && index / 100 != 2 && index / 100 != transition_y
            })
    );

    app.mode = Mode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.preview.image(false).is_none()
            && app.changes.preview.text() == Some("plain text preview\n")
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("plain text preview"));
    assert!(
        !terminal
            .backend()
            .buffer()
            .content
            .iter()
            .enumerate()
            .any(|(index, cell)| {
                cell.symbol() == "▀" && index / 100 != 2 && index / 100 != transition_y
            })
    );
}

#[test]
fn corrupt_image_shows_an_error_as_text() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("broken.png"), b"not a png\0\xff").unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| {
        app.changes
            .preview
            .text()
            .unwrap()
            .starts_with("Could not read image dimensions:")
    });
    assert!(app.changes.preview.image(false).is_none());
    assert!(
        app.changes
            .preview
            .text()
            .unwrap()
            .starts_with("Could not read image dimensions:")
    );
}

#[test]
fn svg_toggles_between_source_and_rendered_image() {
    let directory = tempfile::tempdir().unwrap();
    let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">
<rect width="80" height="10" y="0" fill="#d42a22"/>
<rect width="80" height="10" y="10" fill="#1450d2"/>
<rect width="80" height="10" y="20" fill="#d42a22"/>
<rect width="80" height="10" y="30" fill="#1450d2"/>
</svg>
"##;
    let second_source =
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"></svg>\n";
    fs::write(directory.path().join("preview.svg"), source).unwrap();
    fs::write(directory.path().join("second.svg"), second_source).unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.settings.media_preview_protocol = crate::media::MediaPreviewProtocol::Halfblocks;
    wait_for(&mut app, |app| app.changes.preview.text() == Some(source));

    assert!(app.rendered_preview_available());
    assert!(!app.rendered_preview_visible());
    assert!(app.changes.preview.image(false).is_none());
    assert!(app.changes.preview.image(true).is_some());

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.rendered_preview_visible());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    wait_for_halfblock_render(&mut terminal, &mut app);
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "▀")
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(!app.rendered_preview_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("<svg"));

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.rendered_preview_visible());
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.selected_explorer_file_path()
            .is_some_and(|path| path.display() == "second.svg")
            && app.changes.preview.text() == Some(second_source)
    });
    assert!(!app.rendered_preview_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("xmlns="));
}

#[test]
fn malformed_svg_keeps_source_and_reports_preview_error() {
    let directory = tempfile::tempdir().unwrap();
    let source = "<svg><broken></svg>\n";
    fs::write(directory.path().join("broken.svg"), source).unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| app.changes.preview.text() == Some(source));

    assert!(app.changes.preview.rendered_preview_error().is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Could not parse SVG"));

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(app.changes.preview.text(), Some(source));
}
