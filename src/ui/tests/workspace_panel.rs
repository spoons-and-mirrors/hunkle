use super::*;

#[test]
fn renders_the_workspace_manager_as_a_bottom_drawer() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [{
                    "workspace_id": "w1",
                    "label": "HUNKLE",
                    "number": 4,
                    "pane_count": 2,
                    "focused": true,
                    "agent_status": "working"
                }],
                "agents": [{
                    "agent": "opencode",
                    "agent_status": "working",
                    "focused": true,
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "terminal_title_stripped": "OC | Refine workspace timers",
                    "workspace_id": "w1"
                }]
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(directory.path().to_path_buf());
    app.workspace_panel.workspaces[0].branch = Some("topic".to_owned());
    app.mode = Mode::WorkspacePanel;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let drawer = app.regions.workspace_panel.unwrap();
    assert_eq!(drawer.width, 120);
    assert_eq!(drawer.height, 17);
    assert_eq!(drawer.x, 0);
    assert_eq!(drawer.y, 13);
    assert_eq!(drawer.bottom(), 30);
    assert_eq!(app.regions.worktree.unwrap().x, 0);
    assert!(app.regions.workspace_panel_workspaces.unwrap().x > drawer.x);
    assert!(app.regions.workspace_panel_agents.unwrap().x > drawer.x);
    assert!(
        app.regions.workspace_panel_agents.unwrap().x
            > app.regions.workspace_panel_workspaces.unwrap().right()
    );
    for target in [
        WorkspacePanelHitTarget::Collapse,
        WorkspacePanelHitTarget::CreateMenu,
        WorkspacePanelHitTarget::SnapshotMenu,
        WorkspacePanelHitTarget::Workspace(0),
        WorkspacePanelHitTarget::Agent(0),
    ] {
        assert!(
            app.regions
                .hit_target_rect(HitTarget::WorkspacePanel(target))
                .is_some()
        );
    }
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("WORKSPACE MANAGER"));
    assert!(rendered.contains("WORKSPACES"));
    assert!(rendered.contains("AGENT ACTIVITY"));
    assert!(rendered.contains("HUNKLE"));
    assert!(rendered.contains("ACTIVE"));
    assert!(rendered.contains('⠋'));
    assert!(!rendered.contains("WORKING"));
    let agent_section = app.regions.workspace_panel_agents.unwrap();
    let agent_card_y = agent_section.y.saturating_add(3);
    let agent_target = Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(0)));
    assert_eq!(
        app.regions
            .hit_target_at(Position::new(agent_section.x, agent_card_y)),
        agent_target
    );
    assert_eq!(
        app.regions.hit_target_at(Position::new(
            agent_section.right().saturating_sub(1),
            agent_card_y,
        )),
        agent_target
    );
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(agent_section.x + 2, agent_card_y)].symbol(), "H");
    assert_eq!(
        buffer[(agent_section.right().saturating_sub(3), agent_card_y)].symbol(),
        "⠋"
    );
    let mut agent_rendered = String::new();
    for y in agent_section.y..agent_section.bottom() {
        for x in agent_section.x..agent_section.right() {
            agent_rendered.push_str(buffer[(x, y)].symbol());
        }
    }
    assert!(agent_rendered.contains("HUNKLE"));
    assert!(agent_rendered.contains("topic"));
    assert!(agent_rendered.contains('⠋'));
    assert!(agent_rendered.contains("▐topic▌⠋"));
    assert!(
        agent_rendered.find("topic") < agent_rendered.find('⠋'),
        "status should follow the branch: {agent_rendered:?}"
    );
    assert!(agent_rendered.contains("Refine workspace..."));
    assert!(!agent_rendered.contains("Refine workspace timers"));
    assert!(!agent_rendered.contains("opencode"));
    assert_black_underlay(&terminal);

    let mut tall = Terminal::new(TestBackend::new(120, 50)).unwrap();
    tall.draw(|frame| draw(frame, &mut app)).unwrap();
    let tall_drawer = app.regions.workspace_panel.unwrap();
    assert_eq!(tall_drawer.height, 20);
    assert_eq!(tall_drawer.y, 30);

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::WorkspacePanel);

    let mut narrow = Terminal::new(TestBackend::new(60, 16)).unwrap();
    narrow.draw(|frame| draw(frame, &mut app)).unwrap();
    let narrow_drawer = app.regions.workspace_panel.unwrap();
    assert_eq!(narrow_drawer.width, 60);
    assert_eq!(narrow_drawer.x, 0);
    assert_eq!(narrow_drawer.height, 14);
    assert_eq!(narrow_drawer.bottom(), 16);
    let workspace_section = app.regions.workspace_panel_workspaces.unwrap();
    let agent_section = app.regions.workspace_panel_agents.unwrap();
    assert_eq!(workspace_section.x, agent_section.x);
    assert!(agent_section.y >= workspace_section.bottom());
    let narrow_rendered: String = narrow
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(narrow_rendered.contains("WORKSPACE MANAGER"));
    assert!(narrow_rendered.contains("AGENT ACTIVITY"));
}

#[test]
fn distinguishes_the_active_herdr_workspace_from_the_loaded_workspace() {
    let loaded = tempfile::tempdir().unwrap();
    let active = tempfile::tempdir().unwrap();
    let mut app = App::new(loaded.path().to_path_buf());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    {
                        "workspace_id": "active",
                        "label": "ACTIVE",
                        "focused": true
                    },
                    {
                        "workspace_id": "loaded",
                        "label": "LOADED",
                        "focused": false
                    }
                ],
                "agents": []
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(active.path().to_path_buf());
    app.workspace_panel.workspaces[1].path = Some(loaded.path().to_path_buf());
    app.mode = Mode::WorkspacePanel;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let active_row = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::Workspace(0),
        ))
        .unwrap();
    let loaded_row = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::Workspace(1),
        ))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let active_label = &buffer[(active_row.x + 2, active_row.y)];
    let loaded_label = &buffer[(loaded_row.x + 2, loaded_row.y)];

    assert_eq!(active_label.fg, super::palette().yellow);
    assert!(active_label.modifier.contains(Modifier::BOLD));
    assert_eq!(loaded_label.fg, super::palette().ink);
    assert!(!loaded_label.modifier.contains(Modifier::BOLD));
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains("ACTIVE"));
    assert!(rendered.contains("OPEN"));
}

#[test]
fn toggles_worktree_directories_with_the_mouse() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/app.rs"), "fn main() {}\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        app.changes.worktree_rows(app.repository().unwrap()).len(),
        4
    );

    let worktree = app.regions.worktree_list.unwrap();
    let directory_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.directory_path.is_some())
        .unwrap();
    let directory_y = worktree.y + (directory_row - app.changes.worktree_scroll) as u16;
    click(&mut app, worktree.x + 1, directory_y);
    assert_eq!(app.changes.worktree_state.selected(), Some(directory_row));
    let rows = app.changes.worktree_rows(app.repository().unwrap());
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].directory_expanded, Some(false));

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktree = app.regions.worktree_list.unwrap();
    let directory_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.directory_path.is_some())
        .unwrap();
    let directory_y = worktree.y + (directory_row - app.changes.worktree_scroll) as u16;
    click(&mut app, worktree.x + 1, directory_y);
    assert_eq!(
        app.changes.worktree_rows(app.repository().unwrap()).len(),
        4
    );
}

#[test]
fn clicking_an_agent_displays_it_without_opening_the_workspace_manager() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Agents Pane Test"]);
    run_git(root, &["config", "user.email", "agents-pane@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    let mut app = App::new(root.to_path_buf());
    app.settings.agents_height = 9;
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [{
                    "workspace_id": "w1",
                    "label": "HUNKLE",
                    "focused": true
                }],
                "agents": [{
                    "agent": "opencode",
                    "agent_status": "working",
                    "focused": true,
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "terminal_title_stripped": "OC | Refine workspace timers",
                    "workspace_id": "w1"
                }]
            }
        }
    }));
    app.workspace_panel.workspaces[0].branch = Some("feature/agents".to_owned());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.mode, Mode::Normal);

    let agents = app.regions.agents_list.unwrap();
    let splitter = app.regions.agents_splitter.unwrap();
    let agent = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(0)))
        .unwrap();
    assert_eq!(agent.x.saturating_add(1), splitter.x);
    assert_eq!(agent.right(), splitter.right().saturating_add(1));
    let top_padding_row = agent.y - 1;
    assert_eq!(top_padding_row, agents.y);
    assert!((agent.x..agent.right()).all(|column| {
        terminal.backend().buffer()[(column, top_padding_row)].symbol() == "▀"
    }));
    assert_eq!(
        app.regions
            .hit_target_at(Position::new(agent.x + 2, top_padding_row)),
        None
    );
    let agent_row: String = (agent.x..agent.right())
        .map(|column| terminal.backend().buffer()[(column, agent.y)].symbol())
        .collect();
    assert!(agent_row.contains("HUNKLE"), "agent row was: {agent_row:?}");
    assert!(
        agent_row.contains("feature/agents"),
        "agent row was: {agent_row:?}"
    );
    assert!(agent_row.contains('⠋'));
    assert!(agent_row.contains("▐feature/agents▌⠋"));
    assert!(
        agent_row.find("feature/agents") < agent_row.find('⠋'),
        "status should follow the branch: {agent_row:?}"
    );
    assert!(!agent_row.contains("WORKING"));
    assert_eq!(
        terminal.backend().buffer()[(splitter.x, agent.y)].symbol(),
        "●",
        "card content should retain its original left inset"
    );
    assert_eq!(
        terminal.backend().buffer()[(splitter.right().saturating_sub(1), agent.y)].symbol(),
        "⠋",
        "card content should retain its original right inset"
    );
    let session_row: String = (agent.x..agent.right())
        .map(|column| terminal.backend().buffer()[(column, agent.y + 1)].symbol())
        .collect();
    assert!(
        session_row.contains("Refine workspace..."),
        "session row was: {session_row:?}"
    );
    assert!(!session_row.contains('⠋'));
    assert!(!session_row.contains("WORKING"));
    assert!(!session_row.contains("timers"));
    let padding_row = agent.bottom();
    assert!(padding_row < agents.bottom());
    assert!(
        (agent.x..agent.right())
            .all(|column| { terminal.backend().buffer()[(column, padding_row)].symbol() == "▀" })
    );
    assert_eq!(
        app.regions
            .hit_target_at(Position::new(agent.x + 2, padding_row)),
        None
    );

    app.handle_mouse(mouse(MouseEventKind::Moved, agent.x + 2, agent.y));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(agent.x + 2, agent.y)].bg,
        super::palette().selected
    );

    click(&mut app, agent.x + 2, agent.y);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn colors_only_agents_in_hunkles_herdr_tab_yellow() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    { "workspace_id": "host", "label": "HOST", "focused": false },
                    { "workspace_id": "other", "label": "OTHER", "focused": true }
                ],
                "agents": [
                    {
                        "agent": "opencode",
                        "agent_status": "working",
                        "focused": false,
                        "pane_id": "host:p1",
                        "tab_id": "host:t1",
                        "workspace_id": "host"
                    },
                    {
                        "agent": "opencode",
                        "agent_status": "idle",
                        "focused": false,
                        "pane_id": "host:p2",
                        "tab_id": "host:t1",
                        "workspace_id": "host"
                    },
                    {
                        "agent": "opencode",
                        "agent_status": "idle",
                        "focused": false,
                        "pane_id": "host:p3",
                        "tab_id": "host:t2",
                        "workspace_id": "host"
                    },
                    {
                        "agent": "opencode",
                        "agent_status": "idle",
                        "focused": true,
                        "pane_id": "other:p1",
                        "tab_id": "other:t1",
                        "workspace_id": "other"
                    }
                ]
            }
        }
    }));
    app.workspace_panel
        .set_host_location_for_test("host", "host:t1");
    app.settings.agents_height = 15;
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let buffer = terminal.backend().buffer();
    for index in [0, 1] {
        let row = app
            .regions
            .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(
                index,
            )))
            .unwrap();
        assert_eq!(buffer[(row.x + 3, row.y)].fg, super::palette().yellow);
        assert_eq!(buffer[(row.x + 3, row.y)].bg, super::palette().surface_alt);
    }
    for index in [2, 3] {
        let row = app
            .regions
            .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(
                index,
            )))
            .unwrap();
        assert_eq!(buffer[(row.x + 3, row.y)].fg, super::palette().ink);
        assert_eq!(
            buffer[(row.x + 3, row.y)].bg,
            if index == 3 {
                super::palette().selected
            } else {
                super::palette().surface_alt
            }
        );
    }
}

#[test]
fn workspace_focus_passes_through_application_shortcuts() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());

    app.mode = Mode::WorkspacePanel;
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    assert!(!app.changes.diff_wrap);

    app.mode = Mode::WorkspacePanel;
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Settings);

    app.mode = Mode::WorkspacePanel;
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::FileSearch);
}

#[test]
fn agents_pane_fits_to_agent_count_and_keeps_manual_resizes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Agents Fit Test"]);
    run_git(root, &["config", "user.email", "agents-fit@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    let snapshot = |agent_count: usize| {
        serde_json::json!({
            "result": { "snapshot": {
                "workspaces": [{ "workspace_id": "w1", "label": "HUNKLE", "focused": false }],
                "agents": (0..agent_count).map(|index| {
                    serde_json::json!({
                        "agent": "opencode",
                        "agent_status": "idle",
                        "focused": false,
                        "pane_id": format!("w:p{index}"),
                        "tab_id": "w:t1",
                        "workspace_id": "w1"
                    })
                }).collect::<Vec<_>>()
            } }
        })
    };

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 5);

    app.workspace_panel = WorkspacePanel::ready_for_test(&snapshot(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 5);

    app.workspace_panel = WorkspacePanel::ready_for_test(&snapshot(2));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 8);

    app.workspace_panel = WorkspacePanel::ready_for_test(&snapshot(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 5);

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
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 10);

    let splitter = app.regions.agents_splitter.unwrap();
    let bounds = app.regions.agents_bounds.unwrap();
    let floor = bounds.bottom().saturating_sub(1);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        column,
        splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        column,
        floor,
    ));
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), column, floor));
    assert_eq!(app.settings.agents_height, 5);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.settings.agents_height, 5);
}
