use super::*;

fn snapshot() -> Value {
    serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    {
                        "workspace_id": "w1",
                        "label": "HUNKLE",
                        "active_tab_id": "w1:t1",
                        "number": 2,
                        "pane_count": 2,
                        "focused": true,
                        "agent_status": "working"
                    },
                    {
                        "workspace_id": "w2",
                        "label": "docs",
                        "number": 3,
                        "pane_count": 1,
                        "focused": false,
                        "agent_status": "idle"
                    }
                ],
                "agents": [{
                    "agent": "opencode",
                    "agent_status": "blocked",
                    "focused": true,
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                }],
                "panes": [{
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1",
                    "cwd": "/home/spoon/code/gitui",
                    "foreground_cwd": "/home/spoon/code/gitui"
                }],
                "layouts": [{
                    "workspace_id": "w1",
                    "tab_id": "w1:t1",
                    "focused_pane_id": "w1:p1"
                }]
            }
        }
    })
}

#[test]
fn snapshots_continue_while_the_panel_is_hidden() {
    let mut panel = WorkspacePanel::new(true, None, None);
    let now = Instant::now();
    panel.next_refresh = now;

    assert!(panel.should_start_snapshot(now));
    panel.set_visible(true);
    assert!(panel.should_start_snapshot(Instant::now()));

    panel.set_visible(false);
    assert!(panel.should_start_snapshot(Instant::now()));
    panel.set_visible(true);
    assert!(panel.should_start_snapshot(Instant::now()));

    panel.loading = true;
    assert!(!panel.should_start_snapshot(Instant::now()));
}

#[test]
fn keeps_worktree_inventory_verified_during_routine_refresh() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    assert!(matches!(
        panel.linked_worktree_observation().ownership,
        HerdrOwnership::Verified(_)
    ));

    panel.loading = true;

    assert!(matches!(
        panel.linked_worktree_observation().ownership,
        HerdrOwnership::Verified(_)
    ));
}

fn agent(name: &str, status: AgentStatus) -> HerdrAgent {
    HerdrAgent {
        name: name.to_owned(),
        session_name: None,
        workspace_id: "workspace".to_owned(),
        tab_id: format!("tab-{name}"),
        pane_id: format!("pane-{name}"),
        focused: false,
        status,
        timing_key: AgentTimingKey::Terminal(format!("{name}@terminal-{name}")),
        session_timing_key: None,
        state_change_seq: 0,
    }
}

fn agent_at_sequence(name: &str, status: AgentStatus, state_change_seq: u64) -> HerdrAgent {
    HerdrAgent {
        state_change_seq,
        ..agent(name, status)
    }
}

#[test]
fn tracks_active_time_for_the_latest_agent_request() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents.clear();
    panel.agent_timings.clear();
    let started = 1_000_000;

    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Idle, 1)],
        started,
    );
    assert_eq!(panel.agent_elapsed_at(0, started), None);

    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Working, 2)],
        started + 2_000,
    );
    assert_eq!(
        panel.agent_elapsed_at(0, started + 7_000),
        Some(Duration::from_secs(5))
    );

    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Blocked, 3)],
        started + 7_000,
    );
    assert_eq!(
        panel.agent_elapsed_at(0, started + 17_000),
        Some(Duration::from_secs(5))
    );

    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Working, 4)],
        started + 17_000,
    );
    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Idle, 5)],
        started + 20_000,
    );
    assert_eq!(
        panel.agent_elapsed_at(0, started + 40_000),
        Some(Duration::from_secs(8))
    );

    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Working, 6)],
        started + 40_000,
    );
    assert_eq!(
        panel.agent_elapsed_at(0, started + 42_000),
        Some(Duration::from_secs(2))
    );
}

#[test]
fn status_events_accumulate_agent_time_without_counting_blocked_time() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents.clear();
    panel.agent_timings.clear();
    let started = 1_000_000;
    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Idle, 1)],
        started,
    );
    let event = |status| herdr::AgentStatusEvent {
        workspace_id: "workspace".to_owned(),
        pane_id: "pane-alpha".to_owned(),
        status,
    };

    panel.apply_agent_status_event_at(event(AgentStatus::Working), started + 2_000);
    panel.apply_agent_snapshot_at(
        vec![agent_at_sequence("alpha", AgentStatus::Working, 2)],
        started + 3_000,
    );
    panel.apply_agent_status_event_at(event(AgentStatus::Blocked), started + 7_000);
    panel.apply_agent_status_event_at(event(AgentStatus::Working), started + 17_000);
    panel.apply_agent_status_event_at(event(AgentStatus::Done), started + 20_000);
    assert_eq!(
        panel.agent_elapsed_for_at(0, AgentTimeDisplay::LatestLoop, started + 40_000),
        Some(Duration::from_secs(8))
    );
    assert_eq!(
        panel.agent_elapsed_for_at(0, AgentTimeDisplay::AgentTotal, started + 40_000),
        Some(Duration::from_secs(8))
    );

    panel.apply_agent_status_event_at(event(AgentStatus::Working), started + 40_000);
    panel.apply_agent_status_event_at(event(AgentStatus::Idle), started + 44_000);
    assert_eq!(
        panel.agent_elapsed_for_at(0, AgentTimeDisplay::LatestLoop, started + 60_000),
        Some(Duration::from_secs(4))
    );
    assert_eq!(
        panel.agent_elapsed_for_at(0, AgentTimeDisplay::AgentTotal, started + 60_000),
        Some(Duration::from_secs(12))
    );
}

#[test]
fn keeps_timing_when_an_agent_switches_sessions() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents.clear();
    panel.agent_timings.clear();
    let started = 1_000_000;
    let mut first = agent_at_sequence("alpha", AgentStatus::Working, 1);
    first.pane_id = "shared-pane".to_owned();

    panel.apply_agent_snapshot_at(vec![first.clone()], started);
    assert_eq!(
        panel.agent_elapsed_at(0, started + 8_000),
        Some(Duration::from_secs(8))
    );

    let mut second = first;
    second.session_timing_key = Some(AgentTimingKey::Session(AgentSessionIdentity {
        source: "herdr:test".to_owned(),
        agent: "alpha".to_owned(),
        kind: "id".to_owned(),
        value: "session-beta".to_owned(),
    }));
    second.session_name = Some("Beta".to_owned());
    second.status = AgentStatus::Idle;
    second.state_change_seq = 2;
    panel.apply_agent_snapshot_at(vec![second.clone()], started + 8_000);
    assert_eq!(
        panel.agent_elapsed_for_at(0, AgentTimeDisplay::AgentTotal, started + 8_000),
        Some(Duration::from_secs(8))
    );

    second.status = AgentStatus::Working;
    second.state_change_seq = 3;
    panel.apply_agent_snapshot_at(vec![second], started + 10_000);
    assert_eq!(
        panel.agent_elapsed_for_at(0, AgentTimeDisplay::AgentTotal, started + 13_000),
        Some(Duration::from_secs(11))
    );
}

#[test]
fn keeps_timing_when_an_agent_terminal_moves_to_another_pane() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents.clear();
    panel.agent_timings.clear();
    let started = 1_000_000;
    let first = agent_at_sequence("alpha", AgentStatus::Working, 1);

    panel.apply_agent_snapshot_at(vec![first.clone()], started);
    let mut moved = first;
    moved.pane_id = "replacement-pane".to_owned();
    panel.apply_agent_snapshot_at(vec![moved], started + 5_000);

    assert_eq!(
        panel.agent_elapsed_at(0, started + 8_000),
        Some(Duration::from_secs(8))
    );
}

#[test]
fn shares_agent_timing_between_panels_and_across_restarts() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let groups_path = directory.path().join("workspace-groups.json");
    let started = 1_000_000;
    let working = agent_at_sequence("alpha", AgentStatus::Working, 10);

    let mut first = WorkspacePanel::new(true, Some(groups_path.clone()), None);
    first.apply_agent_snapshot_at(vec![working.clone()], started);

    let mut second = WorkspacePanel::new(true, Some(groups_path.clone()), None);
    second.apply_agent_snapshot_at(vec![working.clone()], started + 30_000);
    assert_eq!(
        second.agent_elapsed_at(0, started + 35_000),
        Some(Duration::from_secs(35))
    );

    drop(second);
    let mut restarted = WorkspacePanel::new(true, Some(groups_path.clone()), None);
    restarted.apply_agent_snapshot_at(vec![working.clone()], started + 60_000);
    assert_eq!(
        restarted.agent_elapsed_at(0, started + 65_000),
        Some(Duration::from_secs(65))
    );

    let blocked = agent_at_sequence("alpha", AgentStatus::Blocked, 11);
    restarted.apply_agent_snapshot_at(vec![blocked.clone()], started + 70_000);
    first.apply_agent_snapshot_at(vec![working], started + 80_000);
    assert_eq!(
        first.agent_elapsed_at(0, started + 100_000),
        Some(Duration::from_secs(70))
    );

    let mut after_stale_writer = WorkspacePanel::new(true, Some(groups_path), None);
    after_stale_writer.apply_agent_snapshot_at(vec![blocked], started + 100_000);
    assert_eq!(
        after_stale_writer.agent_elapsed_at(0, started + 120_000),
        Some(Duration::from_secs(70))
    );
}

#[test]
fn persists_agent_aliases_between_panel_restarts() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let groups_path = directory.path().join("workspace-groups.json");
    let agent = agent("alpha", AgentStatus::Idle);
    let identity = agent.timing_key.stable_id();

    let mut panel = WorkspacePanel::new(true, Some(groups_path.clone()), None);
    panel.agents = vec![agent.clone()];
    panel
        .rename_agent(identity.clone(), "Reviewer".to_owned())
        .unwrap();
    assert_eq!(panel.agent_display_name(0), Some("Reviewer"));

    let mut restored = WorkspacePanel::new(true, Some(groups_path.clone()), None);
    restored.agents = vec![agent.clone()];
    assert_eq!(restored.agent_display_name(0), Some("Reviewer"));

    restored.rename_agent(identity, String::new()).unwrap();
    let mut cleared = WorkspacePanel::new(true, Some(groups_path), None);
    cleared.agents = vec![agent];
    assert_eq!(cleared.agent_display_name(0), None);
}

#[test]
fn orders_agents_by_latest_state_change() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents = vec![
        agent_at_sequence("alpha", AgentStatus::Idle, 10),
        agent_at_sequence("beta", AgentStatus::Idle, 30),
        agent_at_sequence("gamma", AgentStatus::Working, 20),
    ];
    assert!(panel.select_agent(0));

    let selected = panel.selection_key();
    panel.apply_agent_snapshot(vec![
        agent_at_sequence("alpha", AgentStatus::Idle, 10),
        agent_at_sequence("beta", AgentStatus::Idle, 30),
        agent_at_sequence("gamma", AgentStatus::Working, 20),
    ]);
    panel.restore_selection(selected);
    assert_eq!(
        panel
            .agents
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>(),
        ["beta", "gamma", "alpha"]
    );
    assert_eq!(panel.selected, Some(panel.workspaces.len() + 2));

    let selected = panel.selection_key();
    panel.apply_agent_snapshot(vec![
        agent_at_sequence("gamma", AgentStatus::Idle, 20),
        agent_at_sequence("beta", AgentStatus::Idle, 30),
        agent_at_sequence("alpha", AgentStatus::Working, 40),
    ]);
    panel.restore_selection(selected);
    assert_eq!(
        panel
            .agents
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(panel.selected, Some(panel.workspaces.len()));
}

#[test]
fn agent_highlight_follows_herdr_focus_over_panel_selection() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents = vec![
        agent("alpha", AgentStatus::Idle),
        HerdrAgent {
            focused: true,
            ..agent("beta", AgentStatus::Idle)
        },
    ];
    assert!(panel.select_agent(0));

    assert!(!panel.agent_entry_state(0, true).selected);
    assert!(panel.agent_entry_state(1, true).selected);
    assert_eq!(panel.highlighted_agent_index(true), Some(1));

    panel.agents[1].focused = false;
    assert!(panel.agent_entry_state(0, true).selected);
    assert_eq!(panel.highlighted_agent_index(true), Some(0));
}

#[test]
fn focus_events_update_workspace_and_agent_highlights_immediately() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents = vec![
        HerdrAgent {
            focused: true,
            ..agent("alpha", AgentStatus::Idle)
        },
        agent("beta", AgentStatus::Idle),
    ];
    let workspace_id = panel.workspaces[1].id.clone();

    let pane_id = panel.agents[1].pane_id.clone();
    panel.apply_focus_event(herdr::FocusEvent {
        workspace_id,
        pane_id,
    });
    assert!(!panel.agents[0].focused);
    assert!(panel.agents[1].focused);
}

#[test]
fn parses_snapshot_and_tracks_workspace_and_agent_selection() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    assert_eq!(panel.workspaces.len(), 2);
    assert_eq!(panel.agents.len(), 1);
    assert_eq!(panel.workspaces[0].status, AgentStatus::Working);
    assert_eq!(
        panel.workspaces[0].path.as_deref(),
        Some(Path::new("/home/spoon/code/gitui"))
    );
    let mut worktree_snapshot = snapshot();
    worktree_snapshot["result"]["snapshot"]["workspaces"][0]["worktree"] =
        serde_json::json!({ "checkout_path": "/tmp/hunkle-worktree" });
    let (workspaces, _) = herdr::parse_snapshot(&worktree_snapshot).unwrap();
    assert_eq!(
        workspaces[0].path.as_deref(),
        Some(Path::new("/tmp/hunkle-worktree"))
    );
    assert_eq!(panel.agents[0].status, AgentStatus::Blocked);
    assert_eq!(panel.selected, Some(0));
    assert_eq!(panel.selected_visual_row(), Some(1));

    panel.move_selection(2);
    assert_eq!(panel.selected, Some(2));
    assert_eq!(panel.selected_visual_row(), Some(6));
    panel.move_selection(1);
    assert_eq!(panel.selected, Some(2));

    assert_eq!(
        panel.click_workspace(0),
        WorkspacePanelEffect::OpenWorkspace(PathBuf::from("/home/spoon/code/gitui"))
    );
    assert_eq!(panel.selected, Some(0));
}

#[test]
fn panel_scrolling_does_not_change_selection_and_can_reverse() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    let selected = panel.selected;

    panel.scroll_workspace(3);
    assert_eq!(panel.selected, selected);
    assert_eq!(panel.workspace_scroll, 3);
    panel.scroll_workspace(-1);
    assert_eq!(panel.workspace_scroll, 2);
    panel.scroll_workspace(-10);
    assert_eq!(panel.workspace_scroll, 0);

    assert!(panel.select_agent(0));
    let selected_agent = panel.selected;
    panel.scroll_agents(2);
    assert_eq!(panel.selected, selected_agent);
    panel.scroll_agents(-1);
    assert_eq!(panel.agent_scroll, 1);
}

#[test]
fn a_second_workspace_click_becomes_a_focus_request() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.workspaces[1].path = Some(PathBuf::from("/tmp/work-b"));

    assert!(!panel.register_workspace_click(1));
    assert!(panel.register_workspace_click(1));
}

#[test]
fn every_agent_click_requests_its_pane() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.agents.push(HerdrAgent {
        focused: true,
        ..agent("beta", AgentStatus::Idle)
    });

    for _ in 0..2 {
        assert_eq!(
            panel.click_agent(0),
            WorkspacePanelEffect::FocusAgent("w1:p1".to_owned())
        );
        assert_eq!(panel.selected, Some(panel.workspaces.len()));
        assert!(panel.agents[0].focused);
        assert!(!panel.agents[1].focused);
    }
}

#[test]
fn keeps_the_target_workspace_active_until_herdr_confirms_focus() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.next_refresh = Instant::now() + Duration::from_secs(60);
    assert!(panel.select_workspace(1));
    let request_id = panel.focus.begin("w2".to_owned());

    assert!(!panel.workspace_is_active(0));
    assert!(panel.workspace_is_active(1));

    let stale = herdr::parse_snapshot(&snapshot()).unwrap();
    panel
        .sender
        .send(Completion::Snapshot {
            result: Ok(stale),
            observed_at_ms: unix_time_ms(),
        })
        .unwrap();
    let poll = panel.poll();
    assert!(poll.changed);
    assert!(poll.notice.is_none());
    assert!(!poll.workspace_focus_succeeded);
    assert_eq!(panel.selected, Some(1));
    assert!(!panel.workspace_is_active(0));
    assert!(panel.workspace_is_active(1));
    assert!(panel.focus.pending.is_some());

    let mut confirmed = snapshot();
    confirmed["result"]["snapshot"]["workspaces"][0]["focused"] = false.into();
    confirmed["result"]["snapshot"]["workspaces"][1]["focused"] = true.into();
    panel
        .sender
        .send(Completion::Snapshot {
            result: Ok(herdr::parse_snapshot(&confirmed).unwrap()),
            observed_at_ms: unix_time_ms(),
        })
        .unwrap();
    panel.poll();

    assert!(panel.focus.pending.is_some());
    assert!(!panel.workspace_is_active(0));
    assert!(panel.workspace_is_active(1));

    panel
        .sender
        .send(Completion::WorkspaceFocus {
            request_id,
            result: Ok(()),
        })
        .unwrap();
    let poll = panel.poll();
    assert!(poll.workspace_focus_succeeded);
    assert!(panel.focus.pending.is_none());
    assert!(!panel.workspace_is_active(0));
    assert!(panel.workspace_is_active(1));
}

#[test]
fn herdr_focus_overrides_the_workspace_that_hosts_hunkle() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.focus.set_host(Some("w2".to_owned()));
    panel.restore_selection(None);

    assert!(panel.workspace_is_active(0));
    assert!(!panel.workspace_is_active(1));
    assert_eq!(panel.selected, Some(1));

    let stale = herdr::parse_snapshot(&snapshot()).unwrap();
    panel.workspaces = stale.0;
    assert!(panel.workspace_is_active(0));
    assert!(!panel.workspace_is_active(1));
}

#[test]
fn prepares_the_hidden_process_cursor_after_focusing_away() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.focus.set_host(Some("w1".to_owned()));
    assert!(panel.select_workspace(1));
    panel.loading = true;
    let request_id = panel.focus.begin("w2".to_owned());
    panel
        .sender
        .send(Completion::WorkspaceFocus {
            request_id,
            result: Ok(()),
        })
        .unwrap();

    let poll = panel.poll();

    assert!(poll.workspace_focus_succeeded);
    assert!(panel.focus.pending.is_none());
    assert_eq!(panel.selected, Some(0));
    assert_eq!(panel.selected_workspace_id(), Some("w1"));
    assert!(panel.workspace_is_active(0));
}

#[test]
fn rolls_back_only_the_current_failed_workspace_focus() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.loading = true;
    let old_request = panel.focus.begin("w2".to_owned());
    let current_request = panel.focus.begin("w1".to_owned());
    panel
        .sender
        .send(Completion::WorkspaceFocus {
            request_id: old_request,
            result: Err("old failure".to_owned()),
        })
        .unwrap();
    let poll = panel.poll();
    assert!(poll.notice.is_none());
    assert_eq!(
        panel
            .focus
            .pending
            .as_ref()
            .map(|pending| pending.request_id),
        Some(current_request)
    );
    assert!(panel.workspace_is_active(0));

    panel
        .sender
        .send(Completion::WorkspaceFocus {
            request_id: current_request,
            result: Err("focus failed".to_owned()),
        })
        .unwrap();
    let poll = panel.poll();
    assert_eq!(poll.notice.as_deref(), Some("focus failed"));
    assert!(panel.focus.pending.is_none());
    assert!(panel.workspace_is_active(0));
    assert!(!panel.workspace_is_active(1));
}

#[test]
fn confirms_workspace_close_or_linked_worktree_removal() {
    let mut value = snapshot();
    value["result"]["snapshot"]["workspaces"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "workspace_id": "w3",
            "label": "feature-worktree",
            "pane_count": 1,
            "focused": false,
            "agent_status": "idle",
            "worktree": {
                "checkout_path": "/tmp/worktrees/feature",
                "is_linked_worktree": true,
                "repo_key": "/home/spoon/code/gitui/.git",
                "repo_root": "/home/spoon/code/gitui"
            }
        }));
    let mut panel = WorkspacePanel::ready_for_test(&value);

    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert_eq!(
        panel.delete_dialog.as_ref().map(|dialog| &dialog.kind),
        Some(&WorkspaceDeleteKind::Workspace { pane_count: 2 })
    );
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert!(panel.delete_dialog.is_none());

    panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::CloseWorkspace("w1".to_owned())
    );
    assert!(panel.delete_dialog.is_none());

    assert!(panel.select_workspace(2));
    panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(
        panel.delete_dialog.as_ref().map(|dialog| &dialog.kind),
        Some(&WorkspaceDeleteKind::Worktree {
            path: Some(PathBuf::from("/tmp/worktrees/feature")),
            parent_path: Some(PathBuf::from("/home/spoon/code/gitui")),
        })
    );
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        WorkspacePanelEffect::DeleteWorktree {
            workspace_id: "w3".to_owned(),
            path: Some(PathBuf::from("/tmp/worktrees/feature")),
            parent_path: Some(PathBuf::from("/home/spoon/code/gitui")),
        }
    );

    assert!(panel.select_agent(0));
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        WorkspacePanelEffect::Notice("Select a workspace to close".to_owned())
    );
}

#[test]
fn renames_only_the_selected_workspace() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());

    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    let dialog = panel.rename_dialog.as_ref().unwrap();
    assert_eq!(
        dialog.target,
        WorkspaceRenameTarget::Workspace {
            workspace_id: "w1".to_owned()
        }
    );
    assert_eq!(dialog.original_label, "HUNKLE");
    assert_eq!(dialog.input.selection(), Some((0, "HUNKLE".len())));

    panel.paste("renamed");
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::RenameWorkspace {
            workspace_id: "w1".to_owned(),
            label: "renamed".to_owned(),
        }
    );
    assert!(panel.rename_dialog.is_none());

    panel.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    panel.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert_eq!(
        panel
            .rename_dialog
            .as_ref()
            .and_then(|dialog| dialog.error.as_deref()),
        Some("Workspace name is required")
    );
    panel.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(panel.rename_dialog.is_none());

    assert!(panel.select_agent(0));
    let agent_identity = panel.agents[0].timing_key.stable_id();
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    let agent_dialog = panel.rename_dialog.as_ref().unwrap();
    assert!(matches!(
        &agent_dialog.target,
        WorkspaceRenameTarget::Agent { .. }
    ));
    assert_eq!(agent_dialog.original_label, "terminal session");
    panel.paste("reviewer");
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::RenameAgent {
            identity: agent_identity,
            label: "reviewer".to_owned(),
        }
    );
}

#[test]
fn preserves_herdr_focus_when_removing_another_workspace() {
    let panel = WorkspacePanel::ready_for_test(&snapshot());

    assert_eq!(
        panel.focus_to_restore_after_removing("w2").as_deref(),
        Some("w1")
    );
    assert_eq!(panel.focus_to_restore_after_removing("w1"), None);
}

#[test]
fn reopens_the_parent_only_after_successful_worktree_removal() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.next_refresh = Instant::now() + Duration::from_secs(60);
    panel.loading = true;
    panel.destructive_actions_running = 1;
    let parent = PathBuf::from("/home/spoon/code/gitui");

    panel
        .sender
        .send(Completion::Action {
            result: Ok(()),
            reopen_path: Some(parent.clone()),
            warning: None,
            destructive: true,
        })
        .unwrap();
    let poll = panel.poll();
    assert!(poll.changed);
    assert_eq!(poll.notice, None);
    assert_eq!(poll.reopen_path, Some(parent.clone()));
    assert!(!panel.destructive_action_running());

    panel.next_refresh = Instant::now() + Duration::from_secs(60);
    panel.destructive_actions_running = 1;
    panel
        .sender
        .send(Completion::Action {
            result: Err("worktree has uncommitted changes".to_owned()),
            reopen_path: Some(parent),
            warning: None,
            destructive: true,
        })
        .unwrap();
    let poll = panel.poll();
    assert!(poll.changed);
    assert_eq!(
        poll.notice.as_deref(),
        Some("worktree has uncommitted changes")
    );
    assert_eq!(poll.reopen_path, None);
    assert!(!panel.destructive_action_running());
}

#[test]
fn preserves_parent_reopen_across_multiple_destructive_completions() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.next_refresh = Instant::now() + Duration::from_secs(60);
    panel.loading = true;
    panel.destructive_actions_running = 2;
    let parent = PathBuf::from("/home/spoon/code/gitui");

    panel
        .sender
        .send(Completion::Action {
            result: Ok(()),
            reopen_path: Some(parent.clone()),
            warning: None,
            destructive: true,
        })
        .unwrap();
    panel
        .sender
        .send(Completion::Action {
            result: Ok(()),
            reopen_path: None,
            warning: None,
            destructive: true,
        })
        .unwrap();

    let poll = panel.poll();
    assert_eq!(poll.notice, None);
    assert_eq!(poll.reopen_path, Some(parent));
    assert!(!panel.destructive_action_running());
}

#[test]
fn create_menu_requires_a_selected_workspace_for_worktrees() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.toggle_create_menu();
    assert!(panel.create_menu_open);
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert_eq!(panel.create_menu_choice, 1);
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::CreateWorktree("w1".to_owned())
    );
    assert!(!panel.create_menu_open);

    assert!(panel.select_agent(0));
    panel.toggle_create_menu();
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert_eq!(panel.create_menu_choice, 0);
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::CreateWorkspace
    );
}

#[test]
fn reads_branches_from_repositories_and_linked_worktrees() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("repository");
    let nested = repository.join("src/nested");
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        repository.join(".git/HEAD"),
        "ref: refs/heads/feature/panel\n",
    )
    .unwrap();
    assert_eq!(workspace_branch(&nested).as_deref(), Some("feature/panel"));

    let worktree = directory.path().join("worktree");
    let git_dir = directory.path().join("git-data");
    fs::create_dir_all(&worktree).unwrap();
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(worktree.join(".git"), "gitdir: ../git-data\n").unwrap();
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/topic/worktree\n").unwrap();
    assert_eq!(
        workspace_branch(&worktree).as_deref(),
        Some("topic/worktree")
    );
}

#[test]
fn nests_linked_worktrees_under_their_parent_and_prevents_dragging_them() {
    let mut value = snapshot();
    let workspaces = value["result"]["snapshot"]["workspaces"]
        .as_array_mut()
        .unwrap();
    workspaces.push(serde_json::json!({
        "workspace_id": "w3",
        "label": "feature-worktree",
        "pane_count": 1,
        "focused": false,
        "agent_status": "idle",
        "worktree": {
            "checkout_path": "/tmp/worktrees/feature",
            "is_linked_worktree": true,
            "repo_key": "/home/spoon/code/gitui/.git",
            "repo_root": "/home/spoon/code/gitui"
        }
    }));
    let mut panel = WorkspacePanel::ready_for_test(&value);

    assert_eq!(
        panel.workspaces[2].parent_workspace_id.as_deref(),
        Some("w1")
    );
    assert_eq!(
        &panel.rows()[..4],
        &[
            WorkspacePanelRow::Header,
            WorkspacePanelRow::Workspace(0),
            WorkspacePanelRow::Workspace(2),
            WorkspacePanelRow::Workspace(1),
        ]
    );
    assert_eq!(panel.workspace_indent(2), " ");
    assert!(!panel.begin_workspace_drag(2));

    panel.groups = vec![
        WorkspaceGroup {
            name: "Project".to_owned(),
            expanded: true,
            workspace_ids: vec!["w1".to_owned()],
        },
        WorkspaceGroup {
            name: "Old worktree group".to_owned(),
            expanded: true,
            workspace_ids: vec!["w3".to_owned()],
        },
    ];
    assert!(panel.reconcile_group_workspace_ids());
    assert!(panel.groups[1].workspace_ids.is_empty());
    assert_eq!(panel.group_for_workspace(2), Some(0));
    assert_eq!(panel.workspace_indent(2), "  ");
    assert_eq!(
        &panel.rows()[..5],
        &[
            WorkspacePanelRow::Header,
            WorkspacePanelRow::Spacer,
            WorkspacePanelRow::Group(0),
            WorkspacePanelRow::Workspace(0),
            WorkspacePanelRow::Workspace(2),
        ]
    );
    assert_eq!(panel.rows()[5], WorkspacePanelRow::Spacer);
    assert_eq!(panel.rows()[6], WorkspacePanelRow::Group(1));
}

#[test]
fn persists_groups_and_moves_workspaces_between_them() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace-groups.json");
    let mut panel = WorkspacePanel::new(true, Some(path.clone()), None);
    let (workspaces, agents) = herdr::parse_snapshot(&snapshot()).unwrap();
    panel.workspaces = workspaces;
    panel.agents = agents;
    panel.restore_selection(None);

    panel.begin_group();
    panel.paste("Zulu work");
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert!(panel.begin_workspace_drag(0));
    panel.update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
    assert_eq!(panel.finish_workspace_drag(), WorkspacePanelEffect::None);
    assert_eq!(panel.group_for_workspace(0), Some(0));
    assert_eq!(
        panel.agent_rows(),
        [
            WorkspacePanelRow::Spacer,
            WorkspacePanelRow::Agent(0),
            WorkspacePanelRow::AgentSession(0),
        ]
    );
    assert!(path.exists());

    assert!(panel.begin_workspace_drag(1));
    panel.update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
    assert_eq!(panel.finish_workspace_drag(), WorkspacePanelEffect::None);
    assert_eq!(panel.group_for_workspace(1), Some(0));

    panel.begin_group();
    panel.paste("alpha work");
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert_eq!(panel.groups[0].name, "alpha work");
    assert_eq!(panel.groups[1].name, "Zulu work");
    assert!(panel.begin_workspace_drag(0));
    panel.update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
    assert_eq!(panel.finish_workspace_drag(), WorkspacePanelEffect::None);
    assert_eq!(panel.group_for_workspace(0), Some(0));
    assert_eq!(panel.groups[1].workspace_ids, ["w2"]);
    assert_eq!(
        panel.agent_rows(),
        [
            WorkspacePanelRow::Spacer,
            WorkspacePanelRow::Agent(0),
            WorkspacePanelRow::AgentSession(0),
        ]
    );

    panel.toggle_group(0);
    assert_eq!(
        panel.agent_rows(),
        [
            WorkspacePanelRow::Spacer,
            WorkspacePanelRow::Agent(0),
            WorkspacePanelRow::AgentSession(0),
        ]
    );
    panel.toggle_group(0);

    panel.toggle_group(1);
    assert!(!panel.groups[1].expanded);
    assert!(!panel.rows().contains(&WorkspacePanelRow::Workspace(1)));

    let restored = WorkspacePanel::new(true, Some(path), None);
    assert_eq!(restored.groups[0].name, "alpha work");
    assert_eq!(restored.groups[0].workspace_ids, ["w1"]);
    assert_eq!(restored.groups[1].name, "Zulu work");
    assert!(!restored.groups[1].expanded);
    assert_eq!(restored.groups[1].workspace_ids, ["w2"]);
}

#[test]
fn sorts_grouped_workspaces_by_label_and_reorders_after_rename() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.workspaces[0].label = "Zulu".to_owned();
    panel.workspaces[0].path = Some(PathBuf::from("/work/zulu"));
    panel.workspaces[1].label = "alpha".to_owned();
    panel.workspaces[1].path = Some(PathBuf::from("/work/alpha"));
    panel.groups = vec![WorkspaceGroup {
        name: "Projects".to_owned(),
        expanded: true,
        workspace_ids: vec!["w1".to_owned(), "w2".to_owned()],
    }];

    assert_eq!(
        panel.workspace_rows(),
        [
            WorkspacePanelRow::Spacer,
            WorkspacePanelRow::Group(0),
            WorkspacePanelRow::Workspace(1),
            WorkspacePanelRow::Workspace(0),
        ]
    );
    assert_eq!(
        panel.linked_worktree_observation().candidates,
        [
            LinkedWorktreeCandidate {
                path: PathBuf::from("/work/alpha"),
                group: Some("Projects".to_owned()),
            },
            LinkedWorktreeCandidate {
                path: PathBuf::from("/work/zulu"),
                group: Some("Projects".to_owned()),
            }
        ]
    );

    panel.workspaces[0].label = "aardvark".to_owned();
    assert_eq!(
        panel.workspace_rows(),
        [
            WorkspacePanelRow::Spacer,
            WorkspacePanelRow::Group(0),
            WorkspacePanelRow::Workspace(0),
            WorkspacePanelRow::Workspace(1),
        ]
    );
}

#[test]
fn saves_loads_and_deletes_named_workspace_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("workspace-snapshots.json");
    let mut panel = WorkspacePanel::new(true, None, Some(path.clone()));
    let (mut workspaces, agents) = herdr::parse_snapshot(&snapshot()).unwrap();
    workspaces[1].path = Some(PathBuf::from("/home/spoon/docs"));
    panel.workspaces = workspaces;
    panel.agents = agents;
    panel.groups = vec![
        WorkspaceGroup {
            name: "Active work".to_owned(),
            expanded: false,
            workspace_ids: vec!["w1".to_owned()],
        },
        WorkspaceGroup {
            name: "Empty later".to_owned(),
            expanded: true,
            workspace_ids: Vec::new(),
        },
    ];

    panel.toggle_snapshot_menu();
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert!(panel.snapshot_editing);
    panel.paste("Daily setup");
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::Notice("Preset saved: Daily setup".to_owned())
    );
    assert_eq!(panel.snapshots.len(), 1);
    assert_eq!(panel.snapshots[0].workspace_count(), 2);

    panel.groups[0].expanded = true;
    panel.toggle_snapshot_menu();
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    panel.paste("Daily setup");
    assert_eq!(
        panel.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        WorkspacePanelEffect::Notice("Preset updated: Daily setup".to_owned())
    );
    assert_eq!(panel.snapshots.len(), 1);

    let mut restored = WorkspacePanel::new(true, None, Some(path.clone()));
    assert_eq!(restored.snapshots.len(), 1);
    assert_eq!(restored.snapshots[0].name, "Daily setup");
    assert_eq!(restored.snapshots[0].entries[0].label, "HUNKLE");
    assert!(restored.snapshots[0].entries[0].focused);
    assert_eq!(
        restored.snapshots[0].entries[0].group.as_deref(),
        Some("Active work")
    );
    assert_eq!(restored.snapshots[0].groups.len(), 2);
    assert!(restored.snapshots[0].groups[0].expanded);
    assert_eq!(restored.snapshots[0].groups[1].name, "Empty later");
    assert_eq!(
        restored.snapshots[0].entries[1].path,
        PathBuf::from("/home/spoon/docs")
    );

    restored.workspaces = panel.workspaces.clone();
    restored.toggle_snapshot_menu();
    assert_eq!(
        restored.activate_snapshot_choice(1),
        WorkspacePanelEffect::None
    );
    let dialog = restored.snapshot_load_dialog.as_ref().unwrap();
    assert_eq!(dialog.open_count, 0);
    assert_eq!(dialog.close_count, 0);
    assert_eq!(dialog.group_count, 2);
    assert_eq!(
        restored.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert!(restored.snapshot_load_dialog.is_none());

    restored.toggle_snapshot_menu();
    restored.snapshot_menu_choice = 1;
    assert_eq!(
        restored.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        WorkspacePanelEffect::None
    );
    assert!(restored.snapshots.is_empty());
    assert!(
        WorkspacePanel::new(true, None, Some(path))
            .snapshots
            .is_empty()
    );
}

#[test]
fn migrates_legacy_snapshot_groups_by_path_before_workspace_ids_change() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.workspaces[1].path = Some(PathBuf::from("/home/spoon/docs"));
    panel.groups = vec![
        WorkspaceGroup {
            name: "Code".to_owned(),
            expanded: false,
            workspace_ids: vec!["w1".to_owned()],
        },
        WorkspaceGroup {
            name: "Notes".to_owned(),
            expanded: true,
            workspace_ids: vec!["w2".to_owned()],
        },
    ];
    panel.snapshots = vec![WorkspaceSnapshot {
        name: "legacy".to_owned(),
        entries: vec![
            WorkspaceSnapshotEntry {
                label: "HUNKLE".to_owned(),
                path: PathBuf::from("/home/spoon/code/gitui"),
                focused: true,
                linked_worktree: false,
                group: None,
            },
            WorkspaceSnapshotEntry {
                label: "docs".to_owned(),
                path: PathBuf::from("/home/spoon/docs"),
                focused: false,
                linked_worktree: false,
                group: None,
            },
        ],
        groups: Vec::new(),
        groups_captured: false,
    }];

    assert_eq!(
        panel.activate_snapshot_choice(1),
        WorkspacePanelEffect::None
    );
    let migrated = &panel.snapshot_load_dialog.as_ref().unwrap().snapshot;
    assert!(migrated.groups_captured);
    assert!(panel.snapshots[0].groups_captured);
    assert_eq!(migrated.entries[0].group.as_deref(), Some("Code"));
    assert_eq!(migrated.entries[1].group.as_deref(), Some("Notes"));

    let groups =
        presets::groups_after_recall(migrated, &["new-code".to_owned(), "new-notes".to_owned()]);
    assert_eq!(groups[0].workspace_ids, ["new-code"]);
    assert_eq!(groups[1].workspace_ids, ["new-notes"]);
    assert!(!groups[0].expanded);
    assert!(groups[1].expanded);
}

#[test]
fn animates_the_status_marker_while_an_agent_is_working() {
    let mut panel = WorkspacePanel::ready_for_test(&snapshot());
    panel.set_visible(true);
    let now = Instant::now();
    panel.next_spinner = now;

    assert!(panel.poll_spinner(now));
    assert_eq!(panel.spinner_frame, 1);
    assert!(!panel.poll_spinner(now));
    assert!(panel.poll_spinner(now + SPINNER_INTERVAL));
    assert_eq!(panel.spinner_frame, 2);

    panel.workspaces[0].status = AgentStatus::Idle;
    assert!(!panel.poll_spinner(now + SPINNER_INTERVAL * 2));
    assert_eq!(panel.spinner_frame, 0);
}
