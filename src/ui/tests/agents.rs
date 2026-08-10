use super::*;
use crate::app::{
    AgentCardClickAction, AgentKey, AgentPromptDelivery, AgentRequestPartPreview,
    AgentRequestPreview, AgentUserMessage, ScheduledRun, ScheduledRunStatus, ScheduledTask,
};
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
            }],
            "panes": [{
                "pane_id": "w1:p1",
                "tab_id": "w1:t1",
                "workspace_id": "w1",
                "focused": true
            }]
        } }
    })
}

fn open_agents_pane(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert!(app.agents_pane_visible());
}

fn agent_key(app: &App, index: usize) -> AgentKey {
    app.herdr.agent_key(index).unwrap()
}

fn agent_preview_scroll(app: &App) -> (usize, usize) {
    let target = if let Some(run_id) = app.agent_preview.scheduled_run {
        ScrollTarget::AgentScheduledTranscript(run_id)
    } else {
        let index = app.agent_preview_index().unwrap();
        ScrollTarget::AgentTranscript(agent_key(app, index))
    };
    let state = app
        .regions
        .scroll_state(&target)
        .expect("agent preview should register semantic scroll state");
    (state.offset, state.maximum)
}

#[test]
fn control_click_opens_the_live_agent_preview_modal() {
    let directory = tempfile::tempdir().unwrap();
    run_git(directory.path(), &["init", "-b", "main"]);
    let mut app = App::new(directory.path().to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.herdr.set_agent_user_messages_for_test(
        0,
        &[("Inspect the scheduler", Some("Conversation loaded"), 1, 0)],
    );
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 42)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app
        .regions
        .hit_target_rect(HitTarget::Agent(key.clone()))
        .unwrap();
    let point = (card.y..card.bottom())
        .flat_map(|y| (card.x..card.right()).map(move |x| (x, y)))
        .find(|(x, y)| {
            app.regions.hit_target_at(Position::new(*x, *y)) == Some(HitTarget::Agent(key.clone()))
        })
        .unwrap();

    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::CONTROL,
    });
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::CONTROL,
    });
    assert_eq!(app.mode, Mode::AgentPreview);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let screen = screen_text(&terminal);
    assert!(screen.contains("AGENT PREVIEW"));
    assert!(!screen.contains("MESSAGE"));
    assert!(!screen.contains("Enter send"));
    assert!(!screen.contains("Esc cancel"));
    assert!(screen.contains("Inspect the scheduler"));
    assert!(screen.contains("Conversation loaded"));
    let overlay = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewModalOverlay)
        .unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewModalClose)
            .is_some()
    );
    let prompt = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPrompt(key))
        .unwrap();
    let delivery = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPromptDelivery(agent_key(&app, 0)))
        .unwrap();
    assert!(screen.contains("send now"));
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: delivery.x,
        row: delivery.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(
        app.agent_preview.prompt_delivery,
        AgentPromptDelivery::OnIdle
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("send on idle"));
    assert_eq!(prompt.height, 3);
    assert_eq!(delivery.bottom(), prompt.y);
    for y in prompt.y..prompt.bottom() {
        for x in prompt.x..prompt.right() {
            assert_eq!(
                terminal.backend().buffer()[(x, y)].bg,
                palette().surface_alt
            );
        }
    }
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: prompt.x,
        row: prompt.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.agent_preview.prompt_focused);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert_eq!(app.agent_preview.prompt.text(), "\n\n\n");
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.handle_paste("Please check\nthe tests");
    assert_eq!(app.agent_preview.prompt.text(), "Please check\nthe tests");
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let grown_prompt = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPrompt(agent_key(&app, 0)))
        .unwrap();
    assert_eq!(grown_prompt.height, 4);
    assert_eq!(
        terminal.backend().buffer()[(grown_prompt.x, grown_prompt.y + 1)].symbol(),
        " "
    );
    assert_eq!(
        terminal.backend().buffer()[(grown_prompt.x + 1, grown_prompt.y + 1)].symbol(),
        "›"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.agent_preview.prompt.text().is_empty());
    assert!(!app.should_quit);
    app.handle_paste("Wait for idle");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.agent_preview.prompt_focused);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("waiting for idle"));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewPromptDelivery(agent_key(&app, 0)))
            .is_none()
    );

    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: overlay.x,
        row: overlay.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.mode, Mode::AgentPreview);

    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn agent_card_click_setting_swaps_plain_and_control_actions() {
    let directory = tempfile::tempdir().unwrap();
    run_git(directory.path(), &["init", "-b", "main"]);
    let mut app = App::new(directory.path().to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.settings.agent_card_click_action = AgentCardClickAction::OpenPreview;
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 42)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app
        .regions
        .hit_target_rect(HitTarget::Agent(key.clone()))
        .unwrap();
    let point = (card.y..card.bottom())
        .flat_map(|y| (card.x..card.right()).map(move |x| (x, y)))
        .find(|(x, y)| {
            app.regions.hit_target_at(Position::new(*x, *y)) == Some(HitTarget::Agent(key.clone()))
        })
        .unwrap();

    click(&mut app, point.0, point.1);
    assert_eq!(app.mode, Mode::AgentPreview);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let key = agent_key(&app, 0);
    let card = app
        .regions
        .hit_target_rect(HitTarget::Agent(key.clone()))
        .unwrap();
    let point = (card.y..card.bottom())
        .flat_map(|y| (card.x..card.right()).map(move |x| (x, y)))
        .find(|(x, y)| {
            app.regions.hit_target_at(Position::new(*x, *y)) == Some(HitTarget::Agent(key.clone()))
        })
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::CONTROL,
    });
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn agent_preview_modal_routes_message_and_agent_scroll_gestures() {
    let directory = tempfile::tempdir().unwrap();
    run_git(directory.path(), &["init", "-b", "main"]);
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
    snapshot["result"]["snapshot"]["panes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "workspace_id": "w1",
            "focused": false
        }));
    let mut app = App::new(directory.path().to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr.set_agent_user_messages_for_test(
        0,
        &[
            ("First request", Some("First response"), 1, 0),
            ("Second request", Some("Second response"), 1, 0),
            ("Third request", Some("Third response"), 1, 0),
        ],
    );
    app.herdr
        .set_agent_user_messages_for_test(1, &[("Other agent", Some("Other response"), 1, 0)]);
    let first_key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 42)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app
        .regions
        .hit_target_rect(HitTarget::Agent(first_key.clone()))
        .unwrap();
    let point = (card.y..card.bottom())
        .flat_map(|y| (card.x..card.right()).map(move |x| (x, y)))
        .find(|(x, y)| {
            app.regions.hit_target_at(Position::new(*x, *y))
                == Some(HitTarget::Agent(first_key.clone()))
        })
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::CONTROL,
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let timeline = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewMessageTimeline(first_key.clone()))
        .unwrap();
    app.handle_mouse(mouse(MouseEventKind::ScrollUp, timeline.x + 1, timeline.y));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let user_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: first_key.clone(),
            message: 1,
        })
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::ScrollUp,
        user_message.x + 2,
        user_message.y + 1,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let first_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: first_key.clone(),
            message: 0,
        })
        .unwrap();

    app.handle_mouse(mouse(
        MouseEventKind::ScrollLeft,
        first_message.x + 2,
        first_message.y + 1,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewPicker(first_key))
            .is_some()
    );
    assert!(screen_text(&terminal).contains("First response"));
}

#[test]
fn fullscreen_agent_first_click_replaces_footer_path_with_activation_hint() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.herdr.agents[0].destination_cwd = Some(root.to_path_buf());
    app.herdr.set_fullscreen_for_test(true);
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app.regions.hit_target_rect(HitTarget::Agent(key)).unwrap();
    click(&mut app, card.right() - 2, card.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let footer_y = terminal.backend().buffer().area.height - 1;
    let footer = (0..terminal.backend().buffer().area.width)
        .map(|x| terminal.backend().buffer()[(x, footer_y)].symbol())
        .collect::<String>();
    let hint = "double click or press tab to show agent";
    let hint_x = u16::try_from(footer.find(hint).unwrap()).unwrap();
    assert!(!footer.contains(root.to_string_lossy().as_ref()));
    assert!(footer.contains("Git Graph"));
    assert!(
        (hint_x..hint_x + hint.len() as u16)
            .all(|x| terminal.backend().buffer()[(x, footer_y)].fg == super::palette().orange)
    );
}

#[test]
fn panel_mode_toggle_reaches_stashed_agent_cards() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.settings.agents_height = 9;
    app.settings.worktree_width = 48;
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
    let live_key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let live_height = app.settings.agents_height;
    assert_eq!(live_height, 4);
    let live_card = app
        .regions
        .hit_target_rect(HitTarget::Agent(live_key.clone()))
        .unwrap();
    let header = app.regions.agents_splitter.unwrap();
    let toggle = app
        .regions
        .hit_target_rect(HitTarget::AgentListModeToggle)
        .unwrap();
    assert_eq!(toggle.right(), header.right());
    assert_eq!(toggle.y, header.y);
    let toggle_text = (toggle.x..toggle.right())
        .map(|x| terminal.backend().buffer()[(x, toggle.y)].symbol())
        .collect::<String>();
    assert_eq!(toggle_text, " SCHEDULED ");
    let list = app.regions.agents_list.unwrap();
    assert!(
        (list.x..list.right())
            .any(|x| terminal.backend().buffer()[(x, list.bottom())].symbol() == "▀")
    );
    click(&mut app, toggle.x, toggle.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Scheduled);
    let toggle = app
        .regions
        .hit_target_rect(HitTarget::AgentListModeToggle)
        .unwrap();
    click(&mut app, toggle.x, toggle.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Stash);
    assert_eq!(app.settings.agents_height, 7);
    assert!(app.settings.agents_height > live_height);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Agent(live_key))
            .is_none()
    );
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
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Agents);
}

#[test]
fn scheduled_run_cards_cap_height_and_control_click_promotes_instead_of_previewing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.scheduled_tasks.set_tasks_for_test(vec![ScheduledTask {
        id: 7,
        title: "Review".to_owned(),
        description: String::new(),
        prompt: "Review this repository".to_owned(),
        model: String::new(),
        discord_webhook_id: String::new(),
        destination: root.to_path_buf(),
        repository: "repo".to_owned(),
        branch: "main".to_owned(),
        enabled: true,
        interval_minutes: 60,
        next_run_ms: 0,
        source: None,
        project_status: None,
    }]);
    let completed_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .saturating_sub(120_000) as i64;
    app.scheduled_tasks.set_runs_for_test(
        (1..=12)
            .map(|id| ScheduledRun {
                id,
                task_id: 7,
                created_at_ms: id,
                completed_at_ms: Some(completed_at_ms),
                status: ScheduledRunStatus::Completed,
                pane_id: None,
                terminal_id: None,
                session_id: Some(format!("ses_{id}")),
                error: Some("No session".to_owned()),
            })
            .collect(),
    );
    app.herdr.cycle_agent_list_mode();
    let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Scheduled);
    assert_eq!(app.settings.agents_height, 31);
    let visible_runs = (1..=12)
        .filter_map(|id| {
            app.regions
                .hit_target_rect(HitTarget::AgentScheduledRun(id))
                .map(|area| (id, area))
        })
        .collect::<Vec<_>>();
    assert_eq!(visible_runs.len(), 10);
    let (run_id, card) = visible_runs[0];
    let detail = (card.x..card.right())
        .map(|x| terminal.backend().buffer()[(x, card.y + 1)].symbol())
        .collect::<String>();
    assert!(detail.contains("finished 2m ago"));
    assert!((card.x..card.right()).any(|x| {
        let cell = &terminal.backend().buffer()[(x, card.y)];
        cell.symbol() == "⠋" && cell.fg == palette().green
    }));
    click(&mut app, card.x, card.y);
    assert_eq!(app.mode, Mode::AgentPreview);
    assert_eq!(app.agent_preview.scheduled_run, Some(run_id));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app
        .regions
        .hit_target_rect(HitTarget::AgentScheduledRun(run_id))
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: card.x,
        row: card.y,
        modifiers: KeyModifiers::CONTROL,
    });
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.notice.as_deref(),
        Some("Loading active Herdr tab layout")
    );
}

#[test]
fn scheduled_preview_with_one_user_message_has_prompt_and_mouse_scroll() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.scheduled_tasks.set_tasks_for_test(Vec::new());
    app.scheduled_tasks.set_runs_for_test(vec![ScheduledRun {
        id: 17,
        task_id: 7,
        created_at_ms: 1,
        completed_at_ms: None,
        status: ScheduledRunStatus::Working,
        pane_id: Some("w1:p1".to_owned()),
        terminal_id: None,
        session_id: Some("ses_test".to_owned()),
        error: None,
    }]);
    app.agent_preview.set_scheduled_conversation_for_test(
        "ses_test",
        vec![AgentUserMessage {
            text: (0..20)
                .map(|line| format!("scheduled user line {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            requests: vec![AgentRequestPreview {
                parts: vec![AgentRequestPartPreview::Text(
                    (0..100)
                        .map(|line| format!("response line {line}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )],
                reasoning_active: false,
                duration_ms: None,
                reasoning_duration_ms: None,
                tool_call_count: 0,
            }],
        }],
    );
    // Exercise the scheduled fallback while retaining the authoritative observed agent.
    app.herdr.agents.clear();
    app.agent_preview.open_scheduled_run(17, Mode::Normal);
    app.mode = Mode::AgentPreview;
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewScheduledPrompt(17))
            .is_some()
    );
    assert!(agent_preview_scroll(&app).1 > 0);
    assert_eq!(app.agent_preview.scheduled_scroll(), None);
    let user_message = app
        .regions
        .hit_target_rect(HitTarget::AgentScheduledMessage {
            run_id: 17,
            message: 0,
        })
        .unwrap();
    click(&mut app, user_message.x + 1, user_message.y + 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("scheduled user line 19"));
    let transcript = app
        .regions
        .scroll_target_rect(ScrollTarget::AgentScheduledTranscript(17))
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: transcript.x,
        row: transcript.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.agent_preview.scheduled_scroll().is_some());
}

#[test]
fn renders_and_targets_agents_in_the_normal_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.settings.agents_height = 9;
    app.settings.worktree_width = 48;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.herdr.workspaces[0].branch = Some("feature/agents".to_owned());
    let stats_path = PathBuf::from("/agent/stats");
    app.herdr.agents[0].destination_cwd = Some(stats_path.clone());
    app.linked_worktrees
        .set_change_stats_for_test(stats_path, (128, 34));
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
        Some(12_300),
        Some(4_200),
    );
    app.herdr.set_agent_request_for_test(
        0,
        4,
        3,
        AgentRequestPreview {
            parts: vec![
                AgentRequestPartPreview::Text("Earlier harness output".to_owned()),
                AgentRequestPartPreview::Activity(AgentActivityPreview::Tool {
                    name: "read".to_owned(),
                    title: Some("Earlier request context".to_owned()),
                    running: false,
                }),
            ],
            reasoning_active: false,
            duration_ms: Some(3_000),
            reasoning_duration_ms: None,
            tool_call_count: 1,
        },
    );
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.mode, Mode::Normal);
    let area = app
        .regions
        .hit_target_rect(HitTarget::Agent(key.clone()))
        .unwrap();
    let row: String = (area.x..area.right())
        .map(|x| terminal.backend().buffer()[(x, area.y)].symbol())
        .collect();
    let detail: String = (area.x..area.right())
        .map(|x| terminal.backend().buffer()[(x, area.y + 1)].symbol())
        .collect();
    assert!(row.contains("HUNK"));
    assert!(row.contains("fea"));
    assert!(!row.contains("w1:p1"));
    assert!(row.find("id").unwrap() < row.find("HUNK").unwrap());
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

    let pane_id = app
        .regions
        .hit_target_rect(HitTarget::AgentPaneId("w1:p1".to_owned()))
        .unwrap();
    let card_x = pane_id.right().saturating_add(2);
    click(&mut app, pane_id.x + 1, pane_id.y);
    assert_eq!(
        app.take_copy_request().as_deref(),
        Some("herdr_pane_id w1:p1")
    );
    assert!(!app.agents_pane_visible());

    app.handle_mouse(mouse(MouseEventKind::Moved, card_x, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(key.clone())));
    assert!(!app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(area.x + 2, area.y + 1)].bg,
        super::palette().selected
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: key.clone(),
                message: 4,
            })
            .is_none()
    );
    let viewer_before_sidebar_cycle = app.changes.preview.text().unwrap().to_owned();
    let view_before_sidebar_cycle = app.view();
    open_agents_pane(&mut app);
    assert!(app.agents_pane_selected());
    assert!(app.agents_pane_visible());
    assert_eq!(
        app.changes.preview.text(),
        Some(viewer_before_sidebar_cycle.as_str())
    );
    assert_eq!(app.view(), view_before_sidebar_cycle);
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(key.clone())));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(key.clone())));
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
    assert!(!hovered_screen.contains("CONVERSATION LOG"));
    assert!(!hovered_screen.contains("TURN 5 OF 5"));
    assert!(!hovered_screen.contains("TEXT SNAPSHOT"));
    assert!(!hovered_screen.contains("LIVE · REFRESHING"));
    assert!(!hovered_screen.contains("FINAL SNAPSHOT"));
    assert!(!hovered_screen.contains("< USER"));
    assert!(!hovered_screen.contains("REQUEST"));
    assert!(hovered_screen.contains("Timer updates are"));
    assert!(hovered_screen.contains("progress"));
    assert!(hovered_screen.contains("reasoning"));
    assert!(hovered_screen.contains("tool"));
    assert!(hovered_screen.contains("apply_patch"));
    assert!(hovered_screen.contains("› tool  apply_patch"));
    assert!(!hovered_screen.contains("›  tool"));
    assert!(hovered_screen.contains("12.3s"));
    assert!(hovered_screen.contains("5 requests"));
    assert!(hovered_screen.contains("total 15.3s"));
    assert!(hovered_screen.contains("reasoning  4.2s"));
    let history = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
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
    let reasoning_row = row_containing("reasoning").unwrap();
    let message_timeline = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewMessageTimeline(key.clone()))
        .unwrap();
    let repository = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(key.clone()))
        .unwrap();
    let timeline_symbols = (message_timeline.x..message_timeline.right())
        .map(|x| {
            terminal.backend().buffer()[(x, message_timeline.y)]
                .symbol()
                .to_owned()
        })
        .collect::<String>();
    assert_eq!(timeline_symbols.trim(), "○ ○ ○ ○ ●");
    assert_eq!(message_timeline.width, history.width);
    assert_eq!(message_timeline.height, 1);
    assert_eq!(message_timeline.y, history.y + 1);
    assert_eq!(repository.y, message_timeline.y.saturating_sub(2));
    assert_eq!(
        repository.x,
        history.x + history.width.saturating_sub(repository.width) / 2
    );
    assert!(message_timeline.y < text_row);
    assert!(text_row < tool_row);
    assert_eq!(reasoning_row, tool_row + 1);
    app.handle_mouse(mouse(
        MouseEventKind::ScrollUp,
        message_timeline.x,
        message_timeline.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let previous_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!previous_screen.contains("< USER"));
    assert!(previous_screen.contains("Fourth request"));
    let message_timeline = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewMessageTimeline(key.clone()))
        .unwrap();
    let timeline_symbols = (message_timeline.x..message_timeline.right())
        .map(|x| {
            terminal.backend().buffer()[(x, message_timeline.y)]
                .symbol()
                .to_owned()
        })
        .collect::<String>();
    assert_eq!(timeline_symbols.trim(), "○ ○ ○ ● ○");
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        message_timeline.x,
        message_timeline.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(agent_preview_scroll(&app).0, 0);

    let user_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: key.clone(),
            message: 4,
        })
        .unwrap();
    let transcript_row = user_message.bottom().saturating_add(1);
    let scroll_before = agent_preview_scroll(&app).0;
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        user_message.x + 2,
        transcript_row,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(agent_preview_scroll(&app).0 > scroll_before);

    let tooltip = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
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
    assert!(hovered_screen.contains('┃'));
    for _ in 0..100 {
        app.handle_mouse(mouse(
            MouseEventKind::ScrollUp,
            tooltip.x + 2,
            tooltip.bottom().saturating_sub(2),
        ));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(agent_preview_scroll(&app).0, 0);
    let user_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: key.clone(),
            message: 4,
        })
        .unwrap();
    assert_eq!(agent_preview_scroll(&app).0, 0);
    assert_eq!(user_message.x, tooltip.x + 1);
    assert_eq!(user_message.y, message_timeline.y + 2);
    assert_eq!(
        terminal.backend().buffer()[(user_message.x, user_message.y - 1)].symbol(),
        " "
    );
    assert_eq!(
        terminal.backend().buffer()[(user_message.x, user_message.y - 1)].bg,
        super::palette().panel
    );
    assert_ne!(
        terminal.backend().buffer()[(tooltip.x.saturating_sub(1), user_message.y)].symbol(),
        "●"
    );
    assert_eq!(user_message.right(), tooltip.right());
    assert_eq!(
        terminal.backend().buffer()[(user_message.x, user_message.y)].symbol(),
        "▄"
    );
    assert_eq!(
        terminal.backend().buffer()[(user_message.x, user_message.y + 1)].symbol(),
        "┃"
    );
    assert_eq!(
        terminal.backend().buffer()[(user_message.x, user_message.y + 1)].fg,
        super::palette().yellow
    );
    assert_eq!(
        terminal.backend().buffer()[(user_message.x + 2, user_message.y + 1)].symbol(),
        "P"
    );
    assert_eq!(
        terminal.backend().buffer()[(user_message.x + 2, user_message.y + 1)].bg,
        super::palette().canvas
    );
    assert_eq!(
        terminal.backend().buffer()[(user_message.right() - 2, user_message.y)].bg,
        super::palette().canvas
    );
    app.handle_mouse(mouse(
        MouseEventKind::ScrollUp,
        user_message.x + 2,
        user_message.y + 1,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let previous_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(previous_screen.contains("Fourth request"));
    let previous_user_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: key.clone(),
            message: 3,
        })
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        previous_user_message.x + 2,
        previous_user_message.y + 1,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let latest_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(latest_screen.contains("Please refine the agent timers"));
    let top_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(top_screen.contains("Please refine the agent timers"));

    for _ in 0..100 {
        app.handle_mouse(mouse(
            MouseEventKind::ScrollDown,
            user_message.x + 2,
            user_message.bottom().saturating_add(1),
        ));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let (scroll, maximum) = agent_preview_scroll(&app);
    assert_eq!(scroll, maximum);

    click(&mut app, card_x, area.y);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.agents_pane_visible());

    app.handle_mouse(mouse(MouseEventKind::Moved, card_x, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(key.clone())));
    assert!(app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: key.clone(),
                message: 4,
            })
            .is_some()
    );

    app.handle_mouse(mouse(MouseEventKind::Moved, viewer.x + 1, viewer.y + 1));
    app.handle_mouse(mouse(MouseEventKind::Moved, card_x, area.y));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(key.clone())));
    assert!(app.agents_pane_visible());
    open_agents_pane(&mut app);
    assert!(app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let preview = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
            message: 4,
        })
        .unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, preview.x + 4, preview.y + 4));
    let pane_before_switch = app.sidebar_pane();
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_ne!(app.sidebar_pane(), pane_before_switch);
    assert_eq!(app.hovered_hit_target, None);
    assert!(!app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: key.clone(),
                message: 4,
            })
            .is_none()
    );
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.agents_pane_selected());
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: key.clone(),
                message: 4,
            })
            .is_some()
    );
    let preview = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key,
            message: 4,
        })
        .unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, preview.x + 4, preview.y + 4));
    let changes_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    click(&mut app, changes_tab.x, changes_tab.y);
    assert!(!app.agents_pane_selected());
    assert_eq!(app.hovered_hit_target, None);
    assert!(!app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agents_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::AgentsTab))
        .unwrap();
    click(&mut app, agents_tab.x, agents_tab.y);
    assert!(app.agents_pane_selected());
    assert!(app.agents_pane_visible());
}

#[test]
fn agent_preview_scrolls_a_bounded_user_message_with_requests() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.worktree_width = 48;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    let user_text = (0..40)
        .map(|line| format!("user line {line:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let agent_text = (0..40)
        .map(|line| format!("agent line {line:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.herdr.set_agent_user_messages_for_test(
        0,
        &[(user_text.as_str(), Some(agent_text.as_str()), 1, 2)],
    );
    app.herdr.set_agent_message_activity_for_test(
        0,
        0,
        &[
            AgentActivityPreview::Reasoning,
            AgentActivityPreview::Tool {
                name: "tool_0".to_owned(),
                title: None,
                running: false,
            },
            AgentActivityPreview::Tool {
                name: "tool_1".to_owned(),
                title: None,
                running: false,
            },
            AgentActivityPreview::Tool {
                name: "tool_2".to_owned(),
                title: None,
                running: false,
            },
            AgentActivityPreview::Tool {
                name: "tool_3".to_owned(),
                title: None,
                running: false,
            },
        ],
        false,
        Some(9_000),
        Some(2_000),
    );
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let (scroll, maximum) = agent_preview_scroll(&app);
    assert!(maximum > 0);
    assert_eq!(scroll, maximum);
    let initial_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(initial_screen.contains("agent line 39"));
    assert!(initial_screen.contains("tool_3"));
    assert!(!initial_screen.contains("⌄ more"));
    assert!(initial_screen.contains("user line 00"));
    assert!(!initial_screen.contains("user line 39"));
    let user_message = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: key.clone(),
            message: 0,
        })
        .unwrap();
    click(&mut app, user_message.x + 1, user_message.y + 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let expanded_message = app
        .regions
        .hit_target_rect(HitTarget::AgentExpandedMessage {
            agent: key.clone(),
            message: 0,
        })
        .unwrap();
    assert!(app.agent_preview_user_message_expanded(0));
    assert!(screen_text(&terminal).contains("agent line 00"));
    assert_eq!(
        app.regions.scroll_target_at(Position::new(
            expanded_message.x + 1,
            expanded_message.y + 1,
        )),
        Some(ScrollTarget::AgentTranscript(key.clone()))
    );
    for _ in 0..10 {
        app.handle_mouse(mouse(
            MouseEventKind::ScrollDown,
            expanded_message.x + 1,
            expanded_message.y + 1,
        ));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let (scroll, maximum) = agent_preview_scroll(&app);
    assert_eq!(scroll, maximum);
    assert!(screen_text(&terminal).contains("user line 39"));
    click(&mut app, expanded_message.x + 1, expanded_message.y + 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let preview = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
            message: 0,
        })
        .unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewRequest {
                agent: key.clone(),
                message: 0,
                request: 0,
            })
            .is_none()
    );
    let build_counts = app.agent_preview.presentation.build_counts_for_test();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        app.agent_preview.presentation.build_counts_for_test(),
        build_counts
    );
    for _ in 0..10 {
        app.handle_mouse(mouse(
            MouseEventKind::ScrollDown,
            preview.x + 2,
            preview.bottom() - 2,
        ));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let (scroll, maximum) = agent_preview_scroll(&app);
    assert!(maximum > 0);
    assert_eq!(scroll, maximum);
    let expanded_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(expanded_screen.contains("agent line 39"));
    assert!(expanded_screen.contains("tool_3"));
    assert!(expanded_screen.contains("user line 00"));
    assert!(expanded_screen.contains("1 request"));
    assert!(expanded_screen.contains("total 9s"));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: key.clone(),
                message: 0,
            })
            .is_some()
    );
    let transcript = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
            message: 0,
        })
        .unwrap();
    let drag_start = transcript.y.saturating_add(2);
    let drag_end = drag_start.saturating_add(7).min(transcript.bottom() - 1);

    for _ in 0..10 {
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            transcript.x + 4,
            drag_start,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            transcript.x + 4,
            drag_end,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            transcript.x + 4,
            drag_end,
        ));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let top_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(top_screen.contains("user line 00"));
    let user = app
        .regions
        .hit_target_rect(HitTarget::AgentMessage {
            agent: key,
            message: 0,
        })
        .unwrap();
    assert_eq!(user.right(), preview.right());
    assert!(user.height <= 8);
    assert_eq!(terminal.backend().buffer()[(user.x, user.y)].symbol(), "▄");
    assert_eq!(agent_preview_scroll(&app).0, 0);
}

#[test]
fn mobile_agent_preview_swipes_between_messages() {
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
            "tab_id": "w1:t1",
            "workspace_id": "w1",
            "cwd": "/repos/first-repo"
        },
        {
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "workspace_id": "w1",
            "cwd": "/repos/second-repo"
        }
    ]);
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr.set_agent_user_messages_for_test(
        0,
        &[
            ("First request", Some("First reply"), 1, 0),
            ("Second request", Some("Second reply"), 1, 0),
        ],
    );
    app.herdr
        .set_agent_user_messages_for_test(1, &[("Second request", Some("Second reply"), 1, 0)]);
    let first_key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let first = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: first_key.clone(),
            message: 1,
        })
        .unwrap();
    let first_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!first_screen.contains("LIVE"));
    let drag_start = first.x + 8;
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        drag_start,
        first.y + 8,
    ));
    app.handle_mouse(mouse(MouseEventKind::Moved, first.x + 20, first.y + 24));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let dragging_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!dragging_screen.contains("LIVE"));
    assert!(dragging_screen.contains("message 1"));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        first.x + 20,
        first.y + 24,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewPicker(first_key.clone()))
            .is_some()
    );
    let first_message_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(first_message_screen.contains("First reply"));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewPicker(first_key.clone()))
            .is_some()
    );

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentTooltip {
                agent: first_key.clone(),
                message: 0,
            })
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewPicker(agent_key(&app, 1)))
            .is_none()
    );

    let first = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: first_key.clone(),
            message: 0,
        })
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        first.x + 20,
        first.y + 8,
    ));
    app.handle_mouse(mouse(MouseEventKind::Moved, first.x + 8, first.y + 24));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let next_drag_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(next_drag_screen.contains("message 2"));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        first.x + 8,
        first.y + 24,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewPicker(first_key.clone()))
            .is_some()
    );
    let second_message_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(second_message_screen.contains("Second reply"));
}

#[test]
fn narrow_agent_card_opens_a_full_screen_preview() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.settings.agent_card_click_action = AgentCardClickAction::ChangeLayout;
    app.herdr = HerdrSession::ready_for_test(&agent_snapshot());
    app.herdr.set_agent_user_messages_for_test(
        0,
        &[("Make mobile useful", Some("Working on it"), 1, 2)],
    );
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(!app.regions.agent_cards_presented);

    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agent = app
        .regions
        .hit_target_rect(HitTarget::Agent(key.clone()))
        .unwrap();
    let pane_id = app
        .regions
        .hit_target_rect(HitTarget::AgentPaneId("w1:p1".to_owned()))
        .unwrap();
    assert!(app.regions.worktree_list.is_none());
    assert!(app.regions.diff.is_none());
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderFullscreen)
            .is_none()
    );

    click(&mut app, pane_id.right().saturating_add(2), agent.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.mode, Mode::AgentPreview);
    let overlay = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewModalOverlay)
        .unwrap();
    assert_eq!(overlay, Rect::new(0, 0, 49, 48));
    assert_eq!(
        app.regions
            .hit_target_rect(HitTarget::AgentPreviewModalBackdrop),
        Some(overlay)
    );
    let back = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewModalClose)
        .unwrap();
    assert!(app.regions.changes.is_none());
    let repository = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(key.clone()))
        .unwrap();
    assert_eq!(back.y, repository.y);
    assert!(back.right() < repository.x);
    let history = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
            message: 0,
        })
        .unwrap();
    let timeline = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewMessageTimeline(key.clone()))
        .unwrap();
    let prompt = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPrompt(key.clone()))
        .unwrap();
    let delivery = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPromptDelivery(key.clone()))
        .unwrap();
    assert!(repository.x >= overlay.x);
    assert!(repository.right() <= overlay.right());
    assert!(history.x >= overlay.x);
    assert!(history.right() <= overlay.right());
    assert_eq!(history.x, overlay.x + 2);
    assert_eq!(timeline.y, history.y + 1);
    assert_eq!(delivery.right(), prompt.right());
    assert!(delivery.y >= prompt.y);
    assert!(delivery.bottom() <= prompt.bottom());
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("Make mobile useful"));
    assert!(screen.contains("Working on it"));
    assert!(screen.contains("BACK"));
    assert!(screen.contains("▐ now ▌"));
    assert!(!screen.contains("AGENT PREVIEW"));
    assert!(!screen.contains("Structured OpenCode conversation"));
    assert!(!screen.contains("Enter message"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.hit_target_rect(HitTarget::Agent(key)).is_some());
    assert!(app.regions.changes.is_none());
}

#[test]
fn agent_preview_picker_switches_without_activating_agent_layouts() {
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
            "tab_id": "w1:t1",
            "workspace_id": "w1",
            "cwd": "/repos/first-repo"
        },
        {
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "workspace_id": "w1",
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
    let first_key = agent_key(&app, 0);
    let second_key = agent_key(&app, 1);
    let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let card = app
        .regions
        .hit_target_rect(HitTarget::Agent(first_key.clone()))
        .unwrap();
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
    assert!(!header_text.contains("BACKGROUND"));
    assert!(!header_text.contains("FOREGROUND"));
    let fullscreen = app
        .regions
        .hit_target_rect(HitTarget::HeaderFullscreen)
        .unwrap();
    assert_eq!(fullscreen.right(), 120);
    assert_eq!(fullscreen.y, 1);
    assert_eq!(
        terminal.backend().buffer()[(fullscreen.x + 1, fullscreen.y)].symbol(),
        "⛶"
    );
    assert!(!header_text.contains("1/2"));
    assert!(app.agents_pane_selected());
    let picker = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(first_key.clone()))
        .unwrap();
    let history = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: first_key.clone(),
            message: 0,
        })
        .unwrap();
    let agents_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::AgentsTab))
        .unwrap();
    assert_eq!(
        picker.x,
        history.x + history.width.saturating_sub(picker.width) / 2
    );
    assert_eq!(picker.y, agents_tab.y.saturating_add(2));
    assert_eq!(history.y, picker.y.saturating_add(1));
    assert_eq!(
        terminal.backend().buffer()[(picker.x, picker.y)].symbol(),
        "▐"
    );
    assert_eq!(
        terminal.backend().buffer()[(picker.right() - 1, picker.y)].symbol(),
        "▌"
    );
    click(&mut app, picker.x + 1, picker.y);
    assert!(app.agent_preview_picker_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(picker_screen.contains("AGENTS · 2"), "{picker_screen}");
    let second_agent = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPickerItem(second_key.clone()))
        .unwrap();
    click(&mut app, second_agent.x + 1, second_agent.y);
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: second_key.clone(),
            message: 0,
        })
    );
    assert!(app.agents_pane_selected());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
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
    assert!(!second_header.contains("IDLE"));

    app.herdr.agents[1].tab_id = "w1:t2".to_owned();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let background_header: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!background_header.contains("FOREGROUND"));

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

    let picker = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(second_key.clone()))
        .unwrap();
    click(&mut app, picker.x + 1, picker.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let first_agent = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPickerItem(first_key.clone()))
        .unwrap();
    click(&mut app, first_agent.x + 1, first_agent.y);
    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::AgentTooltip {
            agent: first_key.clone(),
            message: 0,
        })
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(first_key))
        .unwrap();
    click(&mut app, picker.x + 1, picker.y);
    assert!(app.agent_preview_picker_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let second_agent = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPickerItem(second_key))
        .unwrap();
    click(&mut app, second_agent.x + 1, second_agent.y);
    assert!(!app.agent_preview_picker_open());
    assert_eq!(app.agents_pane_index(), Some(1));
}

#[test]
fn cached_agent_target_follows_the_agent_across_reordering_and_session_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut snapshot = agent_snapshot();
    let first = &mut snapshot["result"]["snapshot"]["agents"][0];
    first["terminal_id"] = serde_json::json!("term-first");
    first["state_change_seq"] = serde_json::json!(2);
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
            "terminal_id": "term-second",
            "terminal_title_stripped": "OC | Second agent",
            "workspace_id": "w1",
            "state_change_seq": 1
        }));
    snapshot["result"]["snapshot"]["panes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "workspace_id": "w1"
        }));
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr
        .set_agent_user_messages_for_test(0, &[("First request", None, 1, 0)]);
    app.herdr
        .set_agent_user_messages_for_test(1, &[("Old second request", None, 1, 0)]);
    let first_key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let picker = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPicker(first_key.clone()))
        .unwrap();

    let agents = snapshot["result"]["snapshot"]["agents"]
        .as_array_mut()
        .unwrap();
    agents[1]["agent_session"]["value"] = serde_json::json!("ses_second_new");
    agents[1]["terminal_title_stripped"] = serde_json::json!("OC | Updated second agent");
    agents[1]["state_change_seq"] = serde_json::json!(3);
    app.herdr.apply_snapshot_for_test(&snapshot);

    click(&mut app, picker.x, picker.y);
    assert_eq!(app.agents_pane_index(), Some(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let updated_second_key = agent_key(&app, 0);
    let updated_second = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewPickerItem(updated_second_key))
        .unwrap();
    click(&mut app, updated_second.x + 1, updated_second.y);
    assert_eq!(app.agents_pane_index(), Some(0));
    app.herdr
        .set_agent_user_messages_for_test(0, &[("New second request", None, 1, 0)]);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("New second request"), "{screen}");
    assert!(!screen.contains("First request"), "{screen}");
    assert!(!screen.contains("Old second request"), "{screen}");
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
    snapshot["result"]["snapshot"]["panes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "pane_id": "w1:p2",
            "tab_id": "w1:t1",
            "workspace_id": "w1"
        }));
    let mut app = App::new(directory.path().to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr
        .set_agent_user_messages_for_test(0, &[("First", Some("Reply"), 1, 0)]);
    let first_key = agent_key(&app, 0);
    let second_key = agent_key(&app, 1);
    app.regions.worktree = Some(ratatui::layout::Rect::new(0, 0, 40, 30));
    app.regions.register_hit_target(
        HitTarget::Agent(second_key.clone()),
        ratatui::layout::Rect::new(1, 25, 38, 3),
    );
    app.hovered_hit_target = Some(HitTarget::AgentTooltip {
        agent: first_key,
        message: 0,
    });

    app.handle_mouse(mouse(MouseEventKind::Moved, 3, 26));

    assert_eq!(
        app.hovered_hit_target,
        Some(HitTarget::Agent(second_key.clone()))
    );
    assert!(!app.agents_pane_visible());
    app.handle_mouse(mouse(MouseEventKind::Moved, 50, 5));
    app.handle_mouse(mouse(MouseEventKind::Moved, 3, 26));
    assert_eq!(app.hovered_hit_target, Some(HitTarget::Agent(second_key)));
    assert!(!app.agents_pane_visible());
}

#[test]
fn conversation_preview_scopes_requests_to_the_selected_user_message() {
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
                1,
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
    let key = agent_key(&app, 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agent = app
        .regions
        .hit_target_rect(HitTarget::Agent(key.clone()))
        .unwrap();
    app.handle_mouse(mouse(MouseEventKind::Moved, agent.x + 2, agent.y));
    assert!(!app.agents_pane_visible());
    open_agents_pane(&mut app);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.herdr.agent_user_messages(0).unwrap().len(), 50);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: key.clone(),
                message: 49,
            })
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: key.clone(),
                message: 0,
            })
            .is_none()
    );
    let transcript = app
        .regions
        .hit_target_rect(HitTarget::AgentTooltip {
            agent: key.clone(),
            message: 49,
        })
        .unwrap();

    for _ in 0..500 {
        app.handle_mouse(mouse(
            MouseEventKind::ScrollUp,
            transcript.x + 4,
            transcript.y + 6,
        ));
    }
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(agent_preview_scroll(&app).0, 0);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: key.clone(),
                message: 49,
            })
            .is_some()
    );
    let message_timeline = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewMessageTimeline(key.clone()))
        .unwrap();
    let timeline_symbols = (message_timeline.x..message_timeline.right())
        .map(|x| {
            terminal.backend().buffer()[(x, message_timeline.y)]
                .symbol()
                .to_owned()
        })
        .collect::<String>();
    assert!(timeline_symbols.trim_end().ends_with('●'));
    app.handle_mouse(mouse(
        MouseEventKind::ScrollUp,
        message_timeline.x,
        message_timeline.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: key.clone(),
                message: 48,
            })
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::AgentMessage {
                agent: key.clone(),
                message: 49,
            })
            .is_none()
    );
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!screen.contains("< USER"));
    let message_timeline = app
        .regions
        .hit_target_rect(HitTarget::AgentPreviewMessageTimeline(key))
        .unwrap();
    let timeline_symbols = (message_timeline.x..message_timeline.right())
        .map(|x| {
            terminal.backend().buffer()[(x, message_timeline.y)]
                .symbol()
                .to_owned()
        })
        .collect::<String>();
    assert_eq!(timeline_symbols.matches('●').count(), 1);
    assert!(timeline_symbols.trim_end().ends_with("● ○"));
    assert!(screen.contains("Request 49"));
    assert!(screen.contains("Response 49"));
}

#[test]
fn collapses_agents_sharing_a_tab_into_one_card() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let snapshot = serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE", "focused": false }],
            "agents": (0..2).map(|index| serde_json::json!({
                "agent": "opencode",
                "agent_status": "idle",
                "focused": false,
                "pane_id": format!("w1:p{index}"),
                "tab_id": "w1:t1",
                "workspace_id": "w1"
            })).collect::<Vec<_>>(),
            "panes": (0..2).map(|index| serde_json::json!({
                "pane_id": format!("w1:p{index}"),
                "tab_id": "w1:t1",
                "workspace_id": "w1"
            })).collect::<Vec<_>>()
        } }
    });
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    let first_key = agent_key(&app, 0);
    let second_key = agent_key(&app, 1);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.settings.agents_height, 4);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Agent(first_key))
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Agent(second_key))
            .is_none()
    );
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("2 agents"));
}

#[test]
fn offscreen_working_agent_does_not_register_spinner_animation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let snapshot = serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE", "focused": false }],
            "agents": [
                {
                    "agent": "opencode",
                    "agent_session": {
                        "source": "env",
                        "agent": "opencode",
                        "kind": "session_id",
                        "value": "ses_working"
                    },
                    "agent_status": "working",
                    "focused": false,
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                },
                {
                    "agent": "opencode",
                    "agent_session": {
                        "source": "env",
                        "agent": "opencode",
                        "kind": "session_id",
                        "value": "ses_idle"
                    },
                    "agent_status": "idle",
                    "focused": false,
                    "pane_id": "w1:p2",
                    "tab_id": "w1:t2",
                    "workspace_id": "w1"
                }
            ],
            "panes": [
                { "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1" },
                { "pane_id": "w1:p2", "tab_id": "w1:t2", "workspace_id": "w1" }
            ]
        } }
    });
    let mut app = App::new(root.to_path_buf());
    app.herdr = HerdrSession::ready_for_test(&snapshot);
    app.herdr
        .set_agent_user_messages_for_test(0, &[("Working", None, 1, 0)]);
    app.herdr
        .set_agent_user_messages_for_test(1, &[("Idle", None, 1, 0)]);
    let working = agent_key(&app, 0);
    let idle = agent_key(&app, 1);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.agent_animation_presented);
    app.settings.agents_height = 5;
    app.herdr.scroll_agents(1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert!(
        app.regions
            .hit_target_rect(HitTarget::Agent(working.clone()))
            .is_none()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Agent(idle))
            .is_some()
    );
    assert!(!app.regions.agent_animation_presented);

    app.herdr.scroll_agents(-1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Agent(working))
            .is_some()
    );
    assert!(app.regions.agent_animation_presented);
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
                    "tab_id": format!("w1:t{index}"),
                    "workspace_id": "w1"
                })).collect::<Vec<_>>(),
                "panes": (0..count).map(|index| serde_json::json!({
                    "pane_id": format!("w1:p{index}"),
                    "tab_id": format!("w1:t{index}"),
                    "workspace_id": "w1"
                })).collect::<Vec<_>>()
            } }
        })
    };
    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

    app.herdr = HerdrSession::ready_for_test(&snapshot(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 4);
    app.herdr = HerdrSession::ready_for_test(&snapshot(2));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 7);
    let second = agent_key(&app, 1);
    let fitted_second_card = app
        .regions
        .hit_target_rect(HitTarget::Agent(second.clone()))
        .unwrap();
    let fitted_list = app.regions.agents_list.unwrap();
    assert_eq!(fitted_second_card.bottom(), fitted_list.bottom());
    assert_eq!(
        terminal.backend().buffer()[(fitted_second_card.x, fitted_second_card.bottom())].bg,
        palette().canvas
    );

    app.settings.agents_height = 8;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let second_card = app
        .regions
        .hit_target_rect(HitTarget::Agent(second))
        .unwrap();
    let list = app.regions.agents_list.unwrap();
    assert_eq!(fitted_second_card.y, second_card.y + 1);
    assert!(second_card.bottom() < list.bottom());
    assert_eq!(
        terminal.backend().buffer()[(second_card.x, second_card.bottom())].symbol(),
        "▀"
    );
    assert_eq!(
        terminal.backend().buffer()[(second_card.x, second_card.bottom())].bg,
        palette().panel
    );

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
    assert_eq!(app.settings.agents_height, 4);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.agents_list.unwrap();
    assert!(
        (list.x..list.right())
            .any(|x| terminal.backend().buffer()[(x, list.bottom())].symbol() == "▀")
    );
}
