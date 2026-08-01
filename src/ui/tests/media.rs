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
    wait_for(&mut app, |app| app.changes.preview_image.is_some());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    wait_for_halfblock_render(&mut terminal, &mut app);
    let preview_body = app.regions.diff.unwrap();
    let image_cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .filter(|(index, cell)| cell.symbol() == "▀" && index / 100 != 1)
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
            .any(|(index, cell)| cell.symbol() == "▀" && index / 100 != 1)
    );

    app.mode = Mode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.preview_image.is_none() && app.changes.diff == "plain text preview\n"
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
            .any(|(index, cell)| cell.symbol() == "▀" && index / 100 != 1)
    );
}

#[test]
fn corrupt_image_shows_an_error_as_text() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("broken.png"), b"not a png\0\xff").unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| {
        app.changes
            .diff
            .starts_with("Could not read image dimensions:")
    });
    assert!(app.changes.preview_image.is_none());
    assert!(
        app.changes
            .diff
            .starts_with("Could not read image dimensions:")
    );
}
