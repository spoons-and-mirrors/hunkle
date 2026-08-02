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
    assert!(hovered_screen.contains("LIVE · REFRESHING"));
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
    assert!(text_row < tool_row);
    assert!(tool_row < reasoning_row);

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
    assert!(first_message.y < second_message.y);
    assert_eq!(
        terminal.backend().buffer()[(first_message.x, first_message.y)].symbol(),
        "■"
    );
    assert_eq!(
        terminal.backend().buffer()[(first_message.x + 2, first_message.y)].symbol(),
        "▄"
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
    assert_eq!(app.hovered_hit_target, None);
    assert!(!app.agents_pane_visible());

    app.handle_mouse(mouse(MouseEventKind::Moved, area.x + 3, area.y));
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

    app.handle_mouse(mouse(MouseEventKind::Moved, viewer.x + 1, viewer.y + 1));
    app.handle_mouse(mouse(MouseEventKind::Moved, area.x + 3, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(0)));
    app.handle_mouse(mouse(MouseEventKind::Moved, viewer.x + 1, viewer.y + 1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: 0,
                message: 4,
            })
            .is_none()
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT));
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
    let changes_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    click(&mut app, changes_tab.x, changes_tab.y);
    assert!(!app.agents_pane_pinned);
}

#[test]
fn hovering_another_agent_replaces_the_open_history() {
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
    let column = splitter.right().saturating_sub(2);
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
}
