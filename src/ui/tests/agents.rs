use super::*;
use std::path::PathBuf;

fn agent_snapshot() -> serde_json::Value {
    serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE", "focused": true }],
            "agents": [{
                "agent": "opencode",
                "agent_session": {
                    "source": "env",
                    "agent": "opencode",
                    "kind": "session_id",
                    "value": "ses_test"
                },
                "agent_status": "working",
                "focused": true,
                "pane_id": "w1:p1",
                "tab_id": "w1:t1",
                "terminal_title_stripped": "OC | Refine agent timers across every workspace",
                "workspace_id": "w1"
            }]
        } }
    })
}

fn open_agents_pane(app: &mut App) {
    for _ in 0..3 {
        if app.agents_pane_visible() {
            return;
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    assert!(app.agents_pane_visible());
}

#[test]
fn stash_toggle_replaces_live_cards_with_stashed_agent_cards() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agents_height = 9;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    let stash = StashedAgent {
        harness: "opencode".to_owned(),
        agent_name: "opencode".to_owned(),
        session_source: "env".to_owned(),
        session_kind: "session_id".to_owned(),
        session_id: "ses_stashed".to_owned(),
        session_name: Some("Pick this up next week".to_owned()),
        repository: root.to_path_buf(),
        repository_label: "hunkle".to_owned(),
        worktree: root.to_path_buf(),
        branch: "feature/stash".to_owned(),
        workspace_id: "w1".to_owned(),
        tab_id: "w1:t1".to_owned(),
        pane_id: "w1:p2".to_owned(),
        cwd: Some(root.to_path_buf()),
        destination_cwd: Some(root.to_path_buf()),
        focused: false,
        status: crate::app::AgentStatus::Idle,
        stashed_at_ms: 42,
    };
    let mut second_stash = stash.clone();
    second_stash.session_id = "ses_stashed_2".to_owned();
    second_stash.session_name = Some("Another saved agent".to_owned());
    app.herdr
        .set_stashed_agents_for_test(vec![stash, second_stash]);
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let live_height = app.settings.agents_height;
    assert_eq!(live_height, 5);
    let live_card = app.regions.hit_target_rect(HitTarget::Agent(0)).unwrap();
    let header = app.regions.agents_splitter.unwrap();
    let toggle = app
        .regions
        .hit_target_rect(HitTarget::AgentStashToggle)
        .unwrap();
    assert_eq!(toggle.right().saturating_add(1), header.right());
    assert_eq!(toggle.y, header.y);
    let toggle_text = (toggle.x..toggle.right())
        .map(|x| terminal.backend().buffer()[(x, toggle.y)].symbol())
        .collect::<String>();
    assert_eq!(toggle_text, " STASH ");
    let list = app.regions.agents_list.unwrap();
    assert!(
        (list.x..list.right())
            .any(|x| terminal.backend().buffer()[(x, list.bottom() - 1)].symbol() == "▀")
    );
    click(&mut app, toggle.x, toggle.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert!(app.herdr.showing_stash);
    assert_eq!(app.settings.agents_height, 8);
    assert!(app.settings.agents_height > live_height);
    assert!(app.regions.hit_target_rect(HitTarget::Agent(0)).is_none());
    let stashed_card = app
        .regions
        .hit_target_rect(HitTarget::StashedAgent(0))
        .unwrap();
    assert_eq!(stashed_card.width, live_card.width);
    assert_eq!(stashed_card.height, live_card.height);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::StashedAgent(1))
            .is_some()
    );
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("STASHED 2"));
    assert!(screen.contains("Pick this up next week"));
    assert!(screen.contains("feature/s"));

    click(&mut app, stashed_card.x, stashed_card.y);
    assert!(!app.herdr.showing_stash);
}

#[test]
fn renders_and_targets_agents_in_the_normal_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agents_height = 9;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.herdr.workspaces[0].branch = Some("feature/agents".to_owned());
    let stats_path = PathBuf::from("/agent/stats");
    app.herdr.agents[0].destination_cwd = Some(stats_path.clone());
    app.herdr
        .set_agent_change_stats_for_test(stats_path, (128, 34));
    app.herdr.set_agent_user_messages_for_test(
        0,
        &[
            ("First request", Some("First response"), 1, 2),
            ("Second request", Some("Second response"), 2, 4),
            ("Third request", Some("Third response"), 3, 6),
            ("Fourth request", Some("Fourth response"), 4, 8),
            (
                "Please refine the agent timers across every workspace",
                Some("Timer updates are in progress"),
                5,
                10,
            ),
        ],
    );
    app.herdr.set_agent_message_activity_for_test(
        0,
        4,
        &[
            AgentActivityPreview::Tool {
                name: "apply_patch".to_owned(),
                title: Some("Updated agent history".to_owned()),
                running: false,
            },
            AgentActivityPreview::Reasoning,
        ],
        true,
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.mode, Mode::Normal);
    let area = app.regions.hit_target_rect(HitTarget::Agent(0)).unwrap();
    let row: String = (area.x..area.right())
        .map(|x| terminal.backend().buffer()[(x, area.y)].symbol())
        .collect();
    let detail: String = (area.x..area.right())
        .map(|x| terminal.backend().buffer()[(x, area.y + 1)].symbol())
        .collect();
    assert!(row.contains("HUNK"));
    assert!(row.contains("fea"));
    assert!(row.contains('⠋'));
    assert!(detail.contains("Refine agent timers"));
    assert!(detail.contains("+128"));
    assert!(detail.contains("-34"));
    let idle_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!idle_screen.contains("Please refine"));

    app.handle_mouse(mouse(MouseEventKind::Moved, area.x + 2, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(0)));
    assert!(!app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(area.x + 2, area.y + 1)].bg,
        super::palette().selected
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: 0,
                message: 4,
            })
            .is_none()
    );
    let viewer_before_sidebar_cycle = app.changes.diff.clone();
    let view_before_sidebar_cycle = app.view;
    open_agents_pane(&mut app);
    assert!(app.agents_pane_pinned);
    assert!(app.agents_pane_visible());
    assert_eq!(app.changes.diff, viewer_before_sidebar_cycle);
    assert_eq!(app.view, view_before_sidebar_cycle);
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(0)));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(0)));
    assert_eq!(
        app.herdr
            .agent_user_messages(0)
            .map(|messages| messages.len()),
        Some(5)
    );
    assert_eq!(
        terminal.backend().buffer()[(area.x + 2, area.y + 1)].bg,
        super::palette().selected
    );
    let hovered_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(hovered_screen.contains("CONVERSATION LOG"));
    assert!(hovered_screen.contains("TURN 5 OF 5"));
    assert!(!hovered_screen.contains("TEXT SNAPSHOT"));
    assert!(!hovered_screen.contains("LIVE · REFRESHING"));
    assert!(!hovered_screen.contains("FINAL SNAPSHOT"));
    assert!(hovered_screen.contains("5 REQUESTS"));
    assert!(hovered_screen.contains("10 TOOLS"));
    assert!(hovered_screen.contains("Please refine"));
    assert!(hovered_screen.contains("Timer updates are"));
    assert!(hovered_screen.contains("progress"));
    assert!(hovered_screen.contains("REASONING"));
    assert!(hovered_screen.contains("TOOL"));
    assert!(hovered_screen.contains("apply_patch"));
    let history = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: 0,
            message: 4,
        })
        .unwrap();
    let row_containing = |needle: &str| {
        (history.y..history.bottom()).find(|y| {
            (history.x..history.right())
                .map(|x| terminal.backend().buffer()[(x, *y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
    };
    let text_row = row_containing("Timer updates").unwrap();
    let tool_row = row_containing("apply_patch").unwrap();
    let reasoning_row = row_containing("REASONING").unwrap();
    let metrics_row = row_containing("5 REQUESTS").unwrap();
    assert!(text_row < tool_row);
    assert!(tool_row < reasoning_row);
    assert!(reasoning_row < metrics_row);
    for row in [tool_row, reasoning_row] {
        let text = (history.x..history.right())
            .map(|x| terminal.backend().buffer()[(x, row)].symbol())
            .collect::<String>();
        assert!(!text.contains('·'));
    }

    app.handle_mouse(mouse(MouseEventKind::ScrollUp, area.x + 2, area.y));
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 3
        })
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let scrolled_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(scrolled_screen.contains("4 OF 5"));
    assert!(scrolled_screen.contains("4 REQUESTS"));
    assert!(scrolled_screen.contains("8 TOOLS"));
    assert!(scrolled_screen.contains("Fourth request"));
    app.handle_mouse(mouse(MouseEventKind::ScrollDown, area.x + 3, area.y));
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 4
        })
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let tooltip = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: 0,
            message: 4,
        })
        .unwrap();
    let sidebar = app.regions.worktree.unwrap();
    let viewer = app.regions.diff.unwrap();
    assert_eq!(tooltip.x, sidebar.x + 1);
    assert!(tooltip.right() <= sidebar.right());
    assert!(tooltip.height >= 10);
    assert!(
        !terminal.backend().buffer()[(viewer.x, viewer.y)]
            .modifier
            .contains(Modifier::DIM)
    );
    assert!(hovered_screen.contains('▄'));
    assert!(hovered_screen.contains('▀'));
    assert!(hovered_screen.contains('▐'));
    assert!(hovered_screen.contains('▌'));
    app.handle_mouse(mouse(MouseEventKind::Moved, tooltip.x + 8, tooltip.y + 3));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 4
        })
    );
    let second_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: 0,
            message: 1,
        })
        .unwrap();
    let first_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: 0,
            message: 0,
        })
        .unwrap();
    assert_eq!(first_message.x, second_message.x);
    assert_eq!(second_message.y, first_message.y + 1);
    assert_eq!(
        terminal.backend().buffer()[(first_message.x, first_message.y)].symbol(),
        "○"
    );
    let selected_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: 0,
            message: 4,
        })
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(selected_message.x, selected_message.y)].symbol(),
        "◉"
    );
    assert_eq!(
        terminal.backend().buffer()[(first_message.x + 2, first_message.y)].symbol(),
        "▄"
    );
    assert_eq!(
        terminal.backend().buffer()[(first_message.x + 2, first_message.y + 6)].symbol(),
        "▀"
    );
    assert_eq!(
        terminal.backend().buffer()[(first_message.x + 2, first_message.y + 7)].symbol(),
        "▄"
    );
    assert_eq!(
        terminal.backend().buffer()[(first_message.x + 2, first_message.y + 13)].symbol(),
        "▀"
    );
    app.handle_mouse(mouse(
        MouseEventKind::Moved,
        second_message.x,
        second_message.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let historical_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(historical_screen.contains("2 OF 5"));
    assert!(historical_screen.contains("Second request"));
    assert!(historical_screen.contains("Second response"));
    let newest_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: 0,
            message: 4,
        })
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(newest_message.x, newest_message.y)].fg,
        super::palette().muted
    );
    assert_eq!(
        terminal.backend().buffer()[(newest_message.x, newest_message.y)].symbol(),
        "○"
    );

    app.handle_mouse(mouse(
        MouseEventKind::ScrollUp,
        tooltip.x + 8,
        tooltip.y + 3,
    ));
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 0
        })
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::ScrollUp,
        tooltip.x + 8,
        tooltip.y + 3,
    ));
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 4
        })
    );

    click(&mut app, area.x + 2, area.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 4,
        })
    );
    assert!(app.agents_pane_visible());

    app.handle_mouse(mouse(MouseEventKind::Moved, area.x + 3, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(0)));
    assert!(app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: 0,
                message: 4,
            })
            .is_some()
    );

    app.handle_mouse(mouse(MouseEventKind::Moved, viewer.x + 1, viewer.y + 1));
    app.handle_mouse(mouse(MouseEventKind::Moved, area.x + 3, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(0)));
    assert!(app.agents_pane_visible());
    open_agents_pane(&mut app);
    assert!(app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let preview = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: 0,
            message: 4,
        })
        .unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, preview.x + 4, preview.y + 4));
    let pane_before_tab = app.changes.pane;
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_ne!(app.changes.pane, pane_before_tab);
    assert_eq!(app.hovered_hit_target, None);
    assert!(!app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: 0,
                message: 4,
            })
            .is_none()
    );
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.agents_pane_pinned);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: 0,
                message: 4,
            })
            .is_some()
    );
    let preview = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: 0,
            message: 4,
        })
        .unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, preview.x + 4, preview.y + 4));
    let changes_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    click(&mut app, changes_tab.x, changes_tab.y);
    assert!(!app.agents_pane_pinned);
    assert_eq!(app.hovered_hit_target, None);
    assert!(!app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agents_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::AgentsTab))
        .unwrap();
    click(&mut app, agents_tab.x, agents_tab.y);
    assert!(app.agents_pane_pinned);
    assert!(app.agents_pane_visible());
}

#[test]
fn agent_preview_arrows_cycle_without_activating_agent_layouts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut snapshot = agent_snapshot();
    snapshot["result"]["snapshot"]["agents"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "agent": "opencode",
            "agent_session": {
                "source": "env",
                "agent": "opencode",
                "kind": "session_id",
                "value": "ses_second"
            },
            "agent_status": "idle",
            "focused": false,
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "terminal_title_stripped": "OC | Second agent",
            "workspace_id": "w1"
        }));
    snapshot["result"]["snapshot"]["panes"] = serde_json::json!([
        {
            "pane_id": "w1:p1",
            "cwd": "/repos/first-repo"
        },
        {
            "pane_id": "w1:p2",
            "cwd": "/repos/second-repo"
        }
    ]);
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr.set_host_for_test("w1", "w1:t1", "w1:p0");
    app.herdr
        .set_agent_user_messages_for_test(0, &[("First request", Some("First reply"), 1, 0)]);
    app.herdr
        .set_agent_user_messages_for_test(1, &[("Second request", Some("Second reply"), 2, 1)]);
    let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app.regions.hit_target_rect(HitTarget::Agent(0)).unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, card.x + 2, card.y));
    assert!(!app.agents_pane_visible());
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let header_text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(header_text.contains("first-repo"), "{header_text}");
    assert!(header_text.contains("BACKGROUND"));
    assert!(!header_text.contains("1/2"));
    assert!(app.agents_pane_pinned);
    let placement = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPlacement(0))
        .unwrap();
    assert_eq!(placement.width, 12);
    assert_eq!(
        terminal.backend().buffer()[(placement.x, placement.y)].bg,
        super::palette().raised
    );

    let next = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewNext(0))
        .unwrap();
    let previous = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPrevious(0))
        .unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(0))
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(picker.x, picker.y)].symbol(),
        "▐"
    );
    assert_eq!(
        terminal.backend().buffer()[(picker.right() - 1, picker.y)].symbol(),
        "▌"
    );
    assert_eq!(picker.right(), previous.x);
    assert_eq!(previous.right(), next.x);
    assert_eq!(
        terminal.backend().buffer()[(next.x + 1, next.y)].symbol(),
        "→"
    );
    assert_eq!(next.width, 3);
    click(&mut app, next.x, next.y);
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 1,
            message: 0,
        })
    );
    assert!(app.agents_pane_pinned);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(next.x + 1, next.y)].bg,
        super::palette().selected
    );
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains("Second request")
    );
    let second_header: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(second_header.contains("second-re"));
    assert!(!second_header.contains("first-repo"));

    app.herdr.agents[1].tab_id = "w1:t2".to_owned();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let background_header: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(background_header.contains("FOREGROUND"));

    let viewer = app.regions.diff.unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, viewer.x + 1, viewer.y + 1));
    assert_eq!(app.agents_pane_index(), Some(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let stable_header: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(stable_header.contains("second-re"));
    assert!(!stable_header.contains("first-repo"));

    let previous = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPrevious(1))
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(previous.x + 1, previous.y)].symbol(),
        "←"
    );
    assert_eq!(previous.width, 3);
    click(&mut app, previous.x, previous.y);
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 0,
        })
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(0))
        .unwrap();
    click(&mut app, picker.x + 1, picker.y);
    assert!(app.agent_preview_picker_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let second_agent = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPickerItem(1))
        .unwrap();
    click(&mut app, second_agent.x + 1, second_agent.y);
    assert!(!app.agent_preview_picker_open());
    assert_eq!(app.agents_pane_index(), Some(1));
}

#[test]
fn hovering_agent_cards_does_not_open_history() {
    let directory = tempfile::tempdir().unwrap();
    let mut snapshot = agent_snapshot();
    snapshot["result"]["snapshot"]["agents"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "agent": "opencode",
            "agent_status": "idle",
            "focused": false,
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "workspace_id": "w1"
        }));
    let mut app = App::new(directory.path().to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr
        .set_agent_user_messages_for_test(0, &[("First", Some("Reply"), 1, 0)]);
    app.regions.worktree = Some(ratatui::layout::Rect::new(0, 0, 40, 30));
    app.regions.register_hit_target(
        HitTarget::Agent(1),
        ratatui::layout::Rect::new(1, 25, 38, 3),
    );
    app.hovered_hit_target = Some(HitTarget::AgentTooltip {
        agent: 0,
        message: 0,
    });

    app.handle_mouse(mouse(MouseEventKind::Moved, 3, 26));

    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(1)));
    assert!(!app.agents_pane_visible());
    app.handle_mouse(mouse(MouseEventKind::Moved, 50, 5));
    app.handle_mouse(mouse(MouseEventKind::Moved, 3, 26));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(1)));
    assert!(!app.agents_pane_visible());
}

#[test]
fn conversation_timeline_uses_its_full_height_and_tracks_selection() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agents_height = 5;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    let messages = (1..=50)
        .map(|turn| {
            (
                format!("Request {turn}"),
                Some(format!("Response {turn}")),
                turn,
                turn * 2,
            )
        })
        .collect::<Vec<_>>();
    let messages = messages
        .iter()
        .map(|(request, response, requests, tools)| {
            (request.as_str(), response.as_deref(), *requests, *tools)
        })
        .collect::<Vec<_>>();
    app.herdr
        .set_agent_user_messages_for_test(0, messages.as_slice());
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agent = app.regions.hit_target_rect(HitTarget::Agent(0)).unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, agent.x + 2, agent.y));
    assert!(!app.agents_pane_visible());
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.herdr.agent_user_messages(0).unwrap().len(), 50);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: 0,
                message: 49,
            })
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: 0,
                message: 0,
            })
            .is_none()
    );

    for _ in 0..49 {
        app.handle_mouse(mouse(MouseEventKind::ScrollUp, agent.x + 2, agent.y));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: 0,
            message: 0,
        })
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: 0,
                message: 0,
            })
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: 0,
                message: 49,
            })
            .is_none()
    );
}

#[test]
fn agents_pane_fits_to_agent_count_and_keeps_manual_resizes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let snapshot = |count: usize| {
        serde_json::json!({
            "result": { "snapshot": {
                "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE", "focused": false }],
                "agents": (0..count).map(|index| serde_json::json!({
                    "agent": "opencode",
                    "agent_status": "idle",
                    "focused": false,
                    "pane_id": format!("w1:p{index}"),
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                })).collect::<Vec<_>>()
            } }
        })
    };
    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

    app.herdr = HerdrSession::ready_for_test(&snapshot(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 5);
    app.herdr = HerdrSession::ready_for_test(&snapshot(2));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 8);

    let splitter = app.regions.agents_splitter.unwrap();
    let bounds = app.regions.agents_bounds.unwrap();
    let column = splitter.x;
    let target = bounds.bottom().saturating_sub(10);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        column,
        splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        column,
        target,
    ));
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), column, target));
    assert_eq!(app.settings.agents_height, 10);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let splitter = app.regions.agents_splitter.unwrap();
    let bounds = app.regions.agents_bounds.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        column,
        splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        column,
        bounds.bottom(),
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        column,
        bounds.bottom(),
    ));
    assert_eq!(app.settings.agents_height, 5);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.agents_list.unwrap();
    assert!(
        (list.x..list.right())
            .any(|x| terminal.backend().buffer()[(x, list.bottom() - 1)].symbol() == "▀")
    );
}
