use super::*;

#[test]
fn tabs_widget_uses_five_rows_with_a_group_gap_and_no_divider() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::tabs_only(directory.path().to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(49, 5)).unwrap();
    terminal
        .draw(|frame| draw_tabs_only(frame, &mut app))
        .unwrap();
    assert!(screen_text(&terminal).contains("Waiting for OpenCode tabs"));
    assert_eq!(terminal.backend().buffer()[(0, 1)].symbol(), " ");
    assert_eq!(terminal.backend().buffer()[(4, 2)].symbol(), "W");
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderRepository)
            .is_none()
    );
    assert!(app.repository().is_none());

    app.opencode_presence.tabs.items.push(
        serde_json::from_value(serde_json::json!({
            "sessionID": "tab", "title": "Important task", "project": "Hunkle",
            "branch": "feature/widget", "directory": directory.path(), "active": true,
            "busy": false, "attention": false
        }))
        .unwrap(),
    );
    for (width, height) in [(20, 5), (49, 5), (49, 6), (120, 20)] {
        terminal.backend_mut().resize(width, height);
        terminal
            .draw(|frame| draw_tabs_only(frame, &mut app))
            .unwrap();
        let text = screen_text(&terminal);
        assert!(
            text.contains("Important"),
            "session title in {width} columns"
        );
        assert_eq!(terminal.backend().buffer()[(0, 1)].symbol(), " ");
        assert_eq!(terminal.backend().buffer()[(4, 2)].symbol(), "▌");
        assert_eq!(terminal.backend().buffer()[(5, 4)].symbol(), "I");
        assert_eq!(
            app.regions.scroll_target_at(Position::new(8, 2)),
            Some(ScrollTarget::OpenCodeTabs)
        );
        assert!(
            app.regions
                .hit_target_rect(HitTarget::HeaderRepository)
                .is_none()
        );
        if height >= 6 {
            for x in 0..width {
                assert_eq!(terminal.backend().buffer()[(x, 5)].symbol(), " ");
            }
        }
    }
    terminal.backend_mut().resize(49, 4);
    terminal
        .draw(|frame| draw_tabs_only(frame, &mut app))
        .unwrap();
    assert!(screen_text(&terminal).contains("Tabs need at least"));
    assert!(
        app.regions
            .scroll_state(&ScrollTarget::OpenCodeTabs)
            .is_none()
    );
}

#[test]
fn workspace_defaults_to_no_strip_and_tabs_mono_restores_it() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.mode = Mode::Normal;
    app.opencode_presence.tabs.items.push(
        serde_json::from_value(serde_json::json!({
            "sessionID": "tab", "title": "Important task", "project": "Hunkle",
            "branch": "feature/widget", "directory": directory.path(), "active": true,
            "busy": false, "attention": false
        }))
        .unwrap(),
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    assert!(!app.show_opencode_tabs);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let without_strip = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap()
        .y;
    assert!(
        app.regions
            .scroll_state(&ScrollTarget::OpenCodeTabs)
            .is_none()
    );
    assert!(!screen_text(&terminal).contains("Important task"));
    app.show_opencode_tabs = true;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let with_strip = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap()
        .y;
    assert_eq!(with_strip, without_strip + 5);
    assert!(screen_text(&terminal).contains("Important task"));
    app.show_opencode_tabs = false;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .scroll_state(&ScrollTarget::OpenCodeTabs)
            .is_none()
    );
    assert!(!screen_text(&terminal).contains("Important task"));
    assert!(!app.opencode_presence.tabs.items.is_empty());
}

#[test]
fn opencode_strip_tracks_active_cards_and_scrolls_in_both_compositions() {
    fn assert_group_gap(terminal: &Terminal<TestBackend>) {
        let buffer = terminal.backend().buffer();
        for x in 0..buffer.area.width {
            assert_eq!(buffer[(x, 1)].symbol(), " ", "gap below groups stays blank");
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.show_opencode_tabs = true;
    app.mode = Mode::Normal;
    app.opencode_presence.tabs.items = (0..8)
        .map(|index| {
            serde_json::from_value(serde_json::json!({
                "sessionID": format!("session-{index}"), "title": format!("Task {index}"),
                "directory": format!("/projects/repo-{index}"), "project": "Repository",
                "branch": format!("feature/{index}"), "active": index == 7,
                "busy": index == 6, "attention": index == 5, "unread": null
            }))
            .unwrap()
        })
        .collect();
    app.opencode_presence.tabs.reveal_active = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_group_gap(&terminal);
    assert!(screen_text(&terminal).contains("Task 7"));
    assert!(screen_text(&terminal).contains("Repository"));
    assert!(screen_text(&terminal).contains("feature/7"));
    let buffer = terminal.backend().buffer();
    let title_x = (0..buffer.area.width)
        .find(|&x| buffer[(x, 4)].symbol() == "T" && buffer[(x, 4)].fg == palette().orange)
        .expect("active session name appears below its card");
    assert_ne!(buffer[(title_x, 4)].bg, palette().raised);
    assert_eq!(buffer[(title_x, 4)].fg, palette().orange);
    assert_eq!(buffer[(title_x, 3)].bg, palette().raised);
    assert!(screen_text(&terminal).contains("◆"));
    assert_eq!(
        app.regions.scroll_target_at(Position::new(8, 2)),
        Some(ScrollTarget::OpenCodeTabs)
    );
    let offset = app.opencode_presence.tabs.scroll;
    app.handle_mouse(mouse(MouseEventKind::ScrollLeft, 8, 2));
    assert!(app.opencode_presence.tabs.scroll < offset);

    terminal.backend_mut().resize(49, 24);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_group_gap(&terminal);
    assert!(screen_text(&terminal).contains("Task 7"));
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 8, 2));
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 35, 2));
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 35, 2));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_group_gap(&terminal);
    assert!(!app.header_picker.is_open());
    assert!(screen_text(&terminal).contains("Task 5"));
    let offset = app.opencode_presence.tabs.scroll;
    app.opencode_presence.tabs.items[7].busy = true;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.opencode_presence.tabs.scroll, offset);

    app.opencode_presence.tabs.items[7].active = false;
    app.opencode_presence.tabs.items[0].active = true;
    app.opencode_presence.tabs.reveal_active = true;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.opencode_presence.tabs.scroll, 0);
    assert!(screen_text(&terminal).contains("Task 0"));
    assert_group_gap(&terminal);
    // Scroll into the active card while preserving the group gap.
    app.opencode_presence.tabs.scroll = 10;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_group_gap(&terminal);
    app.opencode_groups.catalog.hidden.insert(1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_group_gap(&terminal);
    let with_strip = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap()
        .y;
    app.opencode_presence.tabs.items.clear();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        app.regions
            .hit_target_rect(HitTarget::HeaderRepository)
            .unwrap()
            .y
            + 5,
        with_strip
    );
    assert!(
        app.regions
            .scroll_state(&ScrollTarget::OpenCodeTabs)
            .is_none()
    );
}
