use super::*;

#[cfg(unix)]
#[test]
fn opencode_tab_switch_follows_groups_without_status_updates_resetting_filters() {
    use super::super::{Mode, opencode_presence::OpenCodePresence};
    use ratatui::{Terminal, backend::TestBackend};
    use std::os::unix::fs::PermissionsExt;

    for (tabs_only, show_tabs, width) in [(true, true, 49), (false, true, 120), (false, false, 120)]
    {
        for isolated in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let publication = directory.path().join("opencode-active.json");
            let groups_path = directory.path().join("groups.json");
            let mut snapshot = serde_json::json!({
                "version": 1, "instanceID": "cli-a", "pid": std::process::id(),
                "activeSessionID": "first", "directory": directory.path(),
                "focusSocket": directory.path().join("focus.sock"),
                "tabs": (["first", "second"].map(|id| serde_json::json!({
                    "sessionID": id, "title": id, "directory": directory.path(),
                    "active": id == "first", "busy": false, "attention": false,
                }))),
            });
            atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
            let mut app = if tabs_only {
                App::tabs_only(directory.path().to_path_buf())
            } else {
                App::new(directory.path().to_path_buf())
            };
            app.mode = Mode::Normal;
            app.show_opencode_tabs = show_tabs;
            app.opencode_presence = OpenCodePresence::at_for_test(publication.clone());
            app.opencode_groups = OpenCodeGroups::load(Some(groups_path.clone()));
            app.opencode_groups.catalog.groups.extend([
                Group {
                    id: 2,
                    name: "Build".into(),
                },
                Group {
                    id: 3,
                    name: "Review".into(),
                },
            ]);
            app.opencode_groups
                .catalog
                .assignments
                .insert("second".into(), 2);
            app.opencode_groups.catalog.hidden = BTreeSet::from([2, 3]);
            app.opencode_groups.catalog.solo = isolated.then_some(1);
            app.opencode_groups.save().unwrap();
            let poll = |app: &mut App| {
                if tabs_only {
                    app.poll_tabs_only()
                } else {
                    app.poll_worker()
                }
            };
            let draw = |terminal: &mut Terminal<TestBackend>, app: &mut App| {
                terminal
                    .draw(|frame| {
                        if tabs_only {
                            crate::ui::draw_tabs_only(frame, app)
                        } else {
                            crate::ui::draw(frame, app)
                        }
                    })
                    .unwrap();
            };
            let target = HitTarget::OpenCodeTab {
                instance_id: "cli-a".into(),
                session_id: "second".into(),
            };
            let mut terminal =
                Terminal::new(TestBackend::new(width, if tabs_only { 5 } else { 24 })).unwrap();
            draw(&mut terminal, &mut app);
            assert!(app.regions.hit_target_rect(target.clone()).is_none());

            // A tab switch between sessions in the same directory must follow the group too.
            snapshot["activeSessionID"] = "second".into();
            snapshot["tabs"][0]["active"] = false.into();
            snapshot["tabs"][1]["active"] = true.into();
            atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
            assert!(poll(&mut app));
            draw(&mut terminal, &mut app);
            assert_eq!(app.regions.hit_target_rect(target).is_some(), show_tabs);
            assert_eq!(app.opencode_groups.visible(2), show_tabs);
            assert!(!app.opencode_groups.visible(3));
            assert_eq!(
                app.opencode_groups.catalog.solo,
                isolated.then_some(if show_tabs { 2 } else { 1 })
            );
            let saved = OpenCodeGroups::load(Some(groups_path.clone()));
            assert_eq!(saved.visible(2), show_tabs);
            assert_eq!(saved.catalog.hidden.contains(&2), isolated || !show_tabs);
            if tabs_only {
                assert!(app.repository().is_none());
                assert!(app.pending_workspace_open.is_none());
            }

            // Browsing another group must survive redraws, status/title changes and tab reorders.
            app.opencode_groups.catalog.solo = isolated.then_some(1);
            app.opencode_groups.catalog.hidden.insert(2);
            snapshot["tabs"][1]["title"] = "Renamed".into();
            snapshot["tabs"][1]["busy"] = true.into();
            snapshot["tabs"].as_array_mut().unwrap().reverse();
            atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
            poll(&mut app);
            draw(&mut terminal, &mut app);
            assert!(!app.opencode_groups.visible(2));
            assert_eq!(app.opencode_groups.catalog.solo, isolated.then_some(1));

            // Selection follows even before OpenCode has hydrated the destination directory.
            snapshot["directory"] = serde_json::Value::Null;
            snapshot["activeSessionID"] = "first".into();
            snapshot["tabs"][0]["active"] = false.into();
            snapshot["tabs"][1]["active"] = true.into();
            app.opencode_groups.catalog.solo = isolated.then_some(2);
            app.opencode_groups.catalog.hidden.insert(1);
            atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
            poll(&mut app);
            assert_eq!(app.opencode_groups.visible(1), show_tabs);
            app.shutdown();
        }
    }
}

#[test]
fn opencode_groups_reorder_by_drag_without_changing_identity_or_filters() {
    use super::super::{Mode, opencode_tabs::OpenCodeTab};
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn point(app: &App, target: HitTarget) -> Position {
        let rect = app.regions.hit_target_rect(target).unwrap();
        Position::new(rect.x, rect.y)
    }
    fn send(app: &mut App, kind: MouseEventKind, point: Position) {
        app.handle_mouse(MouseEvent {
            kind,
            column: point.x,
            row: point.y,
            modifiers: KeyModifiers::NONE,
        });
    }
    fn order(app: &App) -> Vec<u64> {
        app.opencode_groups
            .catalog
            .groups
            .iter()
            .map(|group| group.id)
            .collect()
    }

    for width in [120, 49] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("groups.json");
        let mut app = App::new(directory.path().to_path_buf());
        app.show_opencode_tabs = true;
        app.mode = Mode::Normal;
        app.opencode_groups = OpenCodeGroups::load(Some(path.clone()));
        app.opencode_groups.catalog.groups.extend([
            Group {
                id: 2,
                name: "Build".into(),
            },
            Group {
                id: 3,
                name: "Review".into(),
            },
        ]);
        app.opencode_groups
            .catalog
            .assignments
            .insert("assigned".into(), 2);
        app.opencode_groups.catalog.hidden.insert(3);
        app.opencode_groups.catalog.solo = Some(2);
        app.opencode_presence.tabs.items = vec![OpenCodeTab {
            session_id: "new-session".into(),
            ..Default::default()
        }];
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let first = point(&app, HitTarget::OpenCodeGroup(1));
        let last = point(&app, HitTarget::OpenCodeGroup(3));
        let color = terminal.backend().buffer()[(first.x, first.y)].fg;
        send(&mut app, MouseEventKind::Down(MouseButton::Left), first);
        assert_eq!(
            app.opencode_groups.catalog.solo,
            Some(2),
            "press alone must not filter"
        );
        send(&mut app, MouseEventKind::Drag(MouseButton::Left), last);
        assert_eq!(app.opencode_groups.drag.as_ref().unwrap().target, Some(3));
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        send(&mut app, MouseEventKind::Up(MouseButton::Left), last);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert_eq!(order(&app), [2, 3, 1]);
        assert_eq!(app.opencode_groups.catalog.solo, Some(2));
        assert_eq!(app.opencode_groups.catalog.hidden, BTreeSet::from([3]));
        let moved = point(&app, HitTarget::OpenCodeGroup(1));
        assert_eq!(terminal.backend().buffer()[(moved.x, moved.y)].fg, color);
        let saved = OpenCodeGroups::load(Some(path));
        assert_eq!(
            saved
                .catalog
                .groups
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            [2, 3, 1]
        );
        assert_eq!(saved.group_for("new-session"), 1);
        assert_eq!(saved.group_for("assigned"), 2);

        // Cancelled and out-of-strip drops do not reorder or toggle visibility.
        let destination = point(&app, HitTarget::OpenCodeGroup(2));
        send(&mut app, MouseEventKind::Down(MouseButton::Left), moved);
        send(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            destination,
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        send(&mut app, MouseEventKind::Up(MouseButton::Left), destination);
        assert_eq!(order(&app), [2, 3, 1]);
        send(&mut app, MouseEventKind::Down(MouseButton::Left), moved);
        send(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            Position::new(0, 5),
        );
        assert_eq!(order(&app), [2, 3, 1]);
        assert_eq!(app.opencode_groups.catalog.solo, Some(2));

        // Reordering also works toward the beginning.
        send(&mut app, MouseEventKind::Down(MouseButton::Left), moved);
        send(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            destination,
        );
        send(&mut app, MouseEventKind::Up(MouseButton::Left), destination);
        assert_eq!(order(&app), [1, 2, 3]);

        // Keep holding while scrolling to an initially off-screen destination.
        app.opencode_groups
            .catalog
            .groups
            .extend((4..=10).map(|id| Group {
                id,
                name: format!("Project {id}"),
            }));
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let start = point(&app, HitTarget::OpenCodeGroup(1));
        let arrow = point(&app, HitTarget::OpenCodeGroupScroll(16));
        send(&mut app, MouseEventKind::Down(MouseButton::Left), start);
        send(&mut app, MouseEventKind::Drag(MouseButton::Left), arrow);
        assert_eq!(app.opencode_groups.scroll, 16);
        for _ in 0..100 {
            send(&mut app, MouseEventKind::ScrollRight, start);
        }
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let destination = point(&app, HitTarget::OpenCodeGroup(10));
        send(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            destination,
        );
        send(&mut app, MouseEventKind::Up(MouseButton::Left), destination);
        assert_eq!(order(&app), [2, 3, 4, 5, 6, 7, 8, 9, 10, 1]);
        assert_eq!(app.opencode_groups.catalog.solo, Some(2));
        assert_eq!(app.opencode_groups.catalog.hidden, BTreeSet::from([3]));
    }
}

#[test]
fn opencode_group_isolation_restores_visibility_and_saved_assignments() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("groups.json");
    let mut groups = OpenCodeGroups::load(Some(path.clone()));
    groups.catalog.groups.push(Group {
        id: 2,
        name: "Build 🦀".into(),
    });
    groups.catalog.groups.push(Group {
        id: 3,
        name: "Review".into(),
    });
    groups.catalog.assignments.insert("session-a".into(), 2);
    groups.click(1);
    assert!(!groups.visible(1));
    groups.click(2);
    groups.click(2);
    assert_eq!(groups.catalog.solo, Some(2));
    assert!(groups.visible(2));
    assert!(!groups.visible(3));
    assert_eq!(groups.catalog.hidden, BTreeSet::from([1]));

    groups.click(3);
    assert_eq!(groups.catalog.solo, Some(3));
    groups.click(3);
    assert_eq!(groups.catalog.solo, None);
    assert!(!groups.visible(1));
    assert!(groups.visible(2));
    assert!(groups.visible(3));
    groups.save().unwrap();

    let mut restored = OpenCodeGroups::load(Some(path));
    assert_eq!(restored.group_for("session-a"), 2);
    assert_eq!(restored.group_for("new-session"), 1);
    assert_eq!(restored.catalog.groups[1].name, "Build 🦀");
    assert!(!restored.visible(1));
    // Isolating a previously hidden group must restore its hidden state on exit too.
    restored.click(1);
    restored.click(1);
    assert!(restored.visible(1));
    restored.click(2);
    restored.click(2);
    assert!(!restored.visible(1));
}

#[test]
fn opencode_card_order_survives_external_reorder_and_closed_tabs() {
    use super::super::opencode_tabs::OpenCodeTab;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("groups.json");
    let mut groups = OpenCodeGroups::load(Some(path.clone()));
    let tabs = ["a", "b", "c"].map(|id| OpenCodeTab {
        session_id: id.into(),
        ..Default::default()
    });
    let ordered = |groups: &OpenCodeGroups, tabs: &[OpenCodeTab]| {
        groups
            .ordered_tabs(tabs)
            .into_iter()
            .map(|index| tabs[index].session_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ordered(&groups, &tabs), ["a", "b", "c"]);
    assert!(groups.reorder_card(&tabs, "a", "c"));
    assert_eq!(ordered(&groups, &tabs), ["b", "c", "a"]);
    groups.save().unwrap();
    let restored = OpenCodeGroups::load(Some(path));
    assert_eq!(ordered(&restored, &tabs), ["b", "c", "a"]);
    let reversed = [tabs[2].clone(), tabs[1].clone(), tabs[0].clone()];
    assert_eq!(ordered(&restored, &reversed), ["b", "c", "a"]);
    let without_b = [tabs[2].clone(), tabs[0].clone()];
    assert_eq!(ordered(&restored, &without_b), ["c", "a"]);
    let mut with_new = tabs.to_vec();
    with_new.push(OpenCodeTab {
        session_id: "d".into(),
        ..Default::default()
    });
    assert_eq!(ordered(&restored, &with_new), ["b", "c", "a", "d"]);
    assert!(!groups.reorder_card(&tabs, "a", "missing"));
}

#[cfg(unix)]
#[test]
fn opencode_groups_support_creation_rename_drop_and_filters_at_both_widths() {
    use super::super::opencode_presence::OpenCodePresence;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};

    fn point(app: &App, target: HitTarget) -> Position {
        let rect = app
            .regions
            .hit_target_rect(target)
            .expect("visible semantic target");
        Position::new(rect.x, rect.y)
    }
    fn send(app: &mut App, kind: MouseEventKind, point: Position) {
        app.handle_mouse(MouseEvent {
            kind,
            column: point.x,
            row: point.y,
            modifiers: KeyModifiers::NONE,
        });
    }
    fn click(app: &mut App, target: HitTarget, button: MouseButton) {
        let point = point(app, target);
        send(app, MouseEventKind::Down(button), point);
        send(app, MouseEventKind::Up(button), point);
    }
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    let rename = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
    let tab_target = |id: usize| HitTarget::OpenCodeTab {
        instance_id: "cli-a".into(),
        session_id: format!("tab-{id}"),
    };

    for width in [120, 49] {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let publication = directory.path().join("opencode-active.json");
        let socket = directory.path().join("focus.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut snapshot = serde_json::json!({
            "version": 1, "instanceID": "cli-a", "pid": std::process::id(),
            "activeSessionID": "tab-0", "directory": directory.path(), "focusSocket": socket,
            "tabs": (0..3).map(|index| serde_json::json!({
                "sessionID": format!("tab-{index}"), "title": format!("Task {index}"),
                "directory": directory.path(), "active": index == 0, "busy": false, "attention": false,
            })).collect::<Vec<_>>()
        });
        atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
        let groups_path = directory.path().join("groups.json");
        let mut app = App::new(directory.path().to_path_buf());
        app.show_opencode_tabs = true;
        app.mode = super::super::Mode::Normal;
        app.opencode_presence = OpenCodePresence::at_for_test(publication.clone());
        app.opencode_groups = OpenCodeGroups::load(Some(groups_path.clone()));
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();

        click(&mut app, HitTarget::OpenCodeGroupAdd, MouseButton::Left);
        app.handle_paste("Work");
        app.handle_key(enter);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert_eq!(app.opencode_groups.catalog.groups[1].name, "Work");
        let group = point(&app, HitTarget::OpenCodeGroup(2));
        send(&mut app, MouseEventKind::Moved, group);
        assert!(!app.handle_opencode_group_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::SHIFT)));
        app.handle_key(rename);
        assert_eq!(
            app.opencode_groups.edit.as_ref().unwrap().target,
            NameTarget::Group(Some(2))
        );
        assert!(
            app.opencode_groups.catalog.hidden.is_empty(),
            "hovering to rename must not hide a group"
        );
        app.handle_paste("Build\n 🦀");
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        click(&mut app, HitTarget::OpenCodeGroupSave, MouseButton::Left);
        assert_eq!(app.opencode_groups.catalog.groups[1].name, "Build 🦀");
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let card = point(&app, tab_target(0));
        send(&mut app, MouseEventKind::Moved, card);
        app.opencode_groups
            .catalog
            .session_names
            .insert("tab-0".into(), "Old local name".into());
        app.opencode_groups.save().unwrap();
        app.handle_key(rename);
        assert_eq!(
            app.opencode_groups.edit.as_ref().unwrap().target,
            NameTarget::Session {
                instance_id: "cli-a".into(),
                session_id: "tab-0".into()
            }
        );
        assert_eq!(
            app.opencode_groups.edit.as_ref().unwrap().input.text(),
            "Task 0"
        );
        app.handle_paste("My 🦀 task");
        app.handle_key(enter);
        let (mut request, _) = listener.accept().unwrap();
        let mut bytes = String::new();
        std::io::Read::read_to_string(&mut request, &mut bytes).unwrap();
        let request: serde_json::Value = serde_json::from_str(&bytes).unwrap();
        assert_eq!(request["action"], "rename");
        assert_eq!(request["sessionID"], "tab-0");
        assert_eq!(request["title"], "My 🦀 task");
        assert_eq!(request["instanceID"], "cli-a");
        snapshot["tabs"][0]["title"] = "My 🦀 task".into();
        atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
        let update = app.opencode_presence.poll();
        assert!(update.changed);
        assert!(update.workspace.is_none());
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let row = |terminal: &Terminal<TestBackend>, y: u16| {
            (0..width)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        };
        // Wide glyphs occupy an additional continuation cell in the test buffer.
        let label = row(&terminal, card.y + 2);
        assert!(label.contains("My 🦀") && label.contains("task"), "{label}");
        assert_eq!(app.opencode_presence.tabs.items[0].title, "My 🦀 task");
        assert!(listener.accept().is_err(), "naming must not focus a tab");
        assert_eq!(
            OpenCodeGroups::load(Some(groups_path.clone()))
                .catalog
                .session_names["tab-0"],
            "Old local name"
        );

        // The session title remains a hover target; empty names are not sent to OpenCode.
        send(
            &mut app,
            MouseEventKind::Moved,
            Position::new(card.x, card.y + 2),
        );
        app.handle_key(rename);
        app.handle_paste("cancelled");
        app.handle_key(escape);
        assert_eq!(app.opencode_presence.tabs.items[0].title, "My 🦀 task");
        app.handle_key(rename);
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(enter);
        assert!(app.opencode_groups.edit.is_some());
        assert!(listener.accept().is_err());
        app.handle_key(escape);
        app.handle_key(rename);
        snapshot["instanceID"] = "other-cli".into();
        atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
        app.handle_key(enter);
        assert!(
            app.opencode_groups.edit.is_some(),
            "a stale target remains editable"
        );
        assert!(app.notice.as_deref().unwrap().contains("Could not rename"));
        assert!(
            listener.accept().is_err(),
            "a stale target must not rename a session"
        );
        app.handle_key(escape);
        snapshot["instanceID"] = "cli-a".into();
        atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();

        send(&mut app, MouseEventKind::Moved, Position::new(0, 1));
        assert!(
            !app.handle_opencode_group_key(rename),
            "F2 away from groups and cards keeps its usual action"
        );
        click(&mut app, HitTarget::OpenCodeGroup(2), MouseButton::Right);
        app.handle_paste("cancelled");
        app.handle_key(escape);
        assert_eq!(app.opencode_groups.catalog.groups[1].name, "Build 🦀");
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();

        // Drag between cards reorders locally without focusing or changing groups.
        let first = point(&app, tab_target(0));
        let second = point(&app, tab_target(1));
        assert!(first.x < second.x);
        send(&mut app, MouseEventKind::Down(MouseButton::Left), first);
        send(&mut app, MouseEventKind::Drag(MouseButton::Left), second);
        assert_eq!(
            app.opencode_groups
                .drag
                .as_ref()
                .unwrap()
                .card_target
                .as_deref(),
            Some("tab-1")
        );
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        send(&mut app, MouseEventKind::Up(MouseButton::Left), second);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert!(point(&app, tab_target(1)).x < point(&app, tab_target(0)).x);
        assert_eq!(app.opencode_groups.group_for("tab-0"), 1);
        assert!(
            listener.accept().is_err(),
            "reordering must not focus a tab"
        );
        assert!(app.pending_workspace_open.is_none());
        assert_eq!(
            OpenCodeGroups::load(Some(groups_path.clone()))
                .catalog
                .card_order[&1],
            ["tab-1", "tab-0", "tab-2"]
        );

        // Escape and dropping outside a card leave the saved order intact.
        let first = point(&app, tab_target(1));
        let second = point(&app, tab_target(0));
        send(&mut app, MouseEventKind::Down(MouseButton::Left), first);
        send(&mut app, MouseEventKind::Drag(MouseButton::Left), second);
        app.handle_key(escape);
        send(&mut app, MouseEventKind::Up(MouseButton::Left), second);
        assert!(point(&app, tab_target(1)).x < point(&app, tab_target(0)).x);
        send(&mut app, MouseEventKind::Down(MouseButton::Left), first);
        send(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            Position::new(0, 5),
        );
        assert!(point(&app, tab_target(1)).x < point(&app, tab_target(0)).x);
        assert!(listener.accept().is_err());

        let card = point(&app, tab_target(0));
        let group = point(&app, HitTarget::OpenCodeGroup(2));
        send(&mut app, MouseEventKind::Down(MouseButton::Left), card);
        send(&mut app, MouseEventKind::Drag(MouseButton::Left), group);
        assert_eq!(app.opencode_groups.drag.as_ref().unwrap().target, Some(2));
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        send(&mut app, MouseEventKind::Up(MouseButton::Left), group);
        assert_eq!(app.opencode_groups.group_for("tab-0"), 2);
        assert!(
            listener.accept().is_err(),
            "a drop must not focus the dragged tab"
        );
        assert!(app.pending_workspace_open.is_none());
        assert!(app.opencode_presence.tabs.items[0].active);
        let saved = OpenCodeGroups::load(Some(groups_path));
        assert_eq!(saved.group_for("tab-0"), 2);
        assert_eq!(saved.catalog.groups[1].name, "Build 🦀");
        assert_eq!(saved.catalog.session_names["tab-0"], "Old local name");
        assert!(!saved.catalog.card_order[&1].contains(&"tab-0".to_owned()));

        // A fresh publisher/order must not reset Hunkle's stable session assignments.
        snapshot["tabs"].as_array_mut().unwrap().reverse();
        atomic_write(&publication, snapshot.to_string().as_bytes()).unwrap();
        app.opencode_presence.poll();
        assert_eq!(app.opencode_groups.group_for("tab-0"), 2);
        assert_eq!(app.opencode_presence.tabs.items[2].title, "My 🦀 task");
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        click(&mut app, HitTarget::OpenCodeGroup(1), MouseButton::Left);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert!(app.regions.hit_target_rect(tab_target(0)).is_some());
        assert!(app.regions.hit_target_rect(tab_target(1)).is_none());
        click(&mut app, HitTarget::OpenCodeGroup(2), MouseButton::Left);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert!(app.regions.hit_target_rect(tab_target(0)).is_none());
        assert!(
            app.regions
                .hit_target_rect(HitTarget::OpenCodeGroup(2))
                .is_some(),
            "hidden groups remain accessible"
        );
        click(&mut app, HitTarget::OpenCodeGroup(2), MouseButton::Left);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert_eq!(app.opencode_groups.catalog.solo, Some(2));
        assert!(app.regions.hit_target_rect(tab_target(0)).is_some());

        // Cancelling a card drag cannot accidentally focus it or change its group.
        let card = point(&app, tab_target(0));
        send(&mut app, MouseEventKind::Down(MouseButton::Left), card);
        app.handle_key(escape);
        send(&mut app, MouseEventKind::Up(MouseButton::Left), card);
        assert!(listener.accept().is_err());
        assert_eq!(app.opencode_groups.group_for("tab-0"), 2);

        for index in 0..8 {
            click(&mut app, HitTarget::OpenCodeGroupAdd, MouseButton::Left);
            app.handle_paste(&format!("Project {index}"));
            app.handle_key(enter);
            terminal
                .draw(|frame| crate::ui::draw(frame, &mut app))
                .unwrap();
        }
        assert!(
            app.regions
                .hit_target_rect(HitTarget::OpenCodeGroup(10))
                .is_some()
        );
        assert!(
            app.regions
                .hit_target_rect(HitTarget::OpenCodeGroup(1))
                .is_none()
        );
        let offset = app.opencode_groups.scroll;
        click(
            &mut app,
            HitTarget::OpenCodeGroupScroll(-16),
            MouseButton::Left,
        );
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert_eq!(app.opencode_groups.scroll, offset.saturating_sub(16));
        assert!(listener.accept().is_err());
    }
}
