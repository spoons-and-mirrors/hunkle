use super::*;
use std::path::PathBuf;

fn agent_snapshot() -> serde_json::Value {
    serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE", "focused": true }],
            "agents": [{
                "agent": "opencode",
                "agent_status": "working",
                "focused": true,
                "pane_id": "w1:p1",
                "tab_id": "w1:t1",
                "terminal_title_stripped": "OC | Refine agent timers",
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
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

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

    app.handle_mouse(mouse(MouseEventKind::Moved, area.x + 2, area.y));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(area.x + 2, area.y + 1)].bg,
        super::palette().selected
    );
    click(&mut app, area.x + 2, area.y);
    assert_eq!(app.mode, Mode::Normal);
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
