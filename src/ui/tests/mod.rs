pub(super) use std::{fs, process::Command, thread, time::Duration};

pub(super) use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
pub(super) use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::Position,
    style::{Color, Modifier},
};
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::app::{
    AgentPaneDirection, App, ChangesHitTarget, CommitMessageGenerator, ExplorerHitTarget,
    GraphColumn, GraphHitTarget, HeaderPickerItem, HeaderPickerKind, HerdrPaneLayout,
    HerdrPaneRect, HerdrSession, HitTarget, LeftPane, Mode, Settings, SettingsPage, SettingsStore,
    ShortcutAction, SqliteFocus, View,
};
pub(super) use crate::repo_path::RepoPath;

pub(super) use super::{
    AgentDestinationKind, BranchPickerStep, draw, lighter, palette, selected_display_range, text,
    wrapped_editor_cursor,
};

mod agents;
mod editor;
mod files;
mod header;
mod media;
mod sqlite;

fn assert_black_underlay(terminal: &Terminal<TestBackend>) {
    let background = &terminal.backend().buffer()[(0, 0)];
    assert_eq!(background.bg, Color::Rgb(0, 0, 0));
    assert!(background.modifier.contains(Modifier::DIM));
}

#[test]
fn background_startup_renders_one_stable_loading_surface() {
    let directory = tempfile::tempdir().unwrap();
    run_git(directory.path(), &["init", "-b", "main"]);
    let mut app = App::opening(directory.path().to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(app.mode, Mode::Normal);
    assert!(screen.contains("Loading workspace…"));
    assert!(app.regions.worktree.is_none());
    assert!(app.regions.explorer_list.is_none());
}

#[test]
fn clean_changes_view_uses_the_git_graph_as_its_detail_surface() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Graph Test"]);
    run_git(root, &["config", "user.email", "graph@example.com"]);
    fs::write(root.join("tracked.txt"), "clean\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    let mut app = App::new(root.to_path_buf());
    assert_eq!(app.changes.pane, LeftPane::Files);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.view, View::Changes);
    assert_eq!(app.visible_view(), View::Graph);

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.graph_table.is_some());
    assert!(app.regions.diff.is_none());

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.graph_commit_open);

    fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
    let mut dirty_app = App::new(root.to_path_buf());
    assert_eq!(dirty_app.changes.pane, LeftPane::Worktree);
    assert_eq!(dirty_app.visible_view(), View::Changes);
    terminal.draw(|frame| draw(frame, &mut dirty_app)).unwrap();
    assert!(dirty_app.regions.graph_table.is_none());
    assert!(dirty_app.regions.diff.is_some());
}

#[test]
fn renders_every_primary_surface() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("tracked.txt"), "first\n").unwrap();
    fs::create_dir(root.join("fixtures")).unwrap();
    for index in 0..40 {
        fs::write(
            root.join(format!("fixtures/file-{index:02}.txt")),
            format!("fixture {index}\n"),
        )
        .unwrap();
    }
    run_git(root, &["add", "."]);
    run_git(
        root,
        &[
            "commit",
            "-m",
            "initial commit",
            "-m",
            "Detailed body line.",
            "-m",
            "Final note.",
        ],
    );
    fs::write(root.join("second.txt"), "second\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(
        root,
        &[
            "-c",
            "user.name=Second Author",
            "-c",
            "user.email=second@example.com",
            "commit",
            "-m",
            "second commit",
        ],
    );
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();
    fs::write(root.join("untracked.txt"), "new\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.settings.graph_lane_width = 0;
    app.settings.graph_description_width = 0;
    app.settings.graph_changes_width = 11;
    app.settings.graph_date_width = 11;
    app.settings.graph_author_width = 16;
    app.settings.graph_commit_width = 7;
    app.commit_message_generator = CommitMessageGenerator::ready_for_test();
    let settings_path = root.join(".git/hunkle-test-config");
    app.settings_store = SettingsStore::at(settings_path.clone());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.worktree.unwrap().x, 0);
    assert_eq!(app.regions.worktree.unwrap().y, 1);
    assert_eq!(app.regions.diff.unwrap().right(), 120);
    let left = app.regions.worktree.unwrap();
    let right = app.regions.diff.unwrap();
    for point in [(left.x, left.y), (right.x, right.y)] {
        let cell = &terminal.backend().buffer()[point];
        assert_eq!(cell.symbol(), "▀");
        assert_eq!(cell.fg, super::palette().surface_alt);
        assert_eq!(cell.bg, super::palette().panel);
    }
    assert_eq!(
        terminal.backend().buffer()[(left.right(), left.y)].bg,
        super::palette().canvas
    );
    assert!(app.regions.changes.is_none());
    assert_eq!(app.regions.graph.unwrap().y, 35);
    assert_eq!(app.regions.help.unwrap().y, 35);
    assert!(app.regions.graph.unwrap().x > 0);
    assert_eq!(app.regions.help.unwrap().right(), 120);
    let buffer = terminal.backend().buffer();
    let agents = app.regions.agents_splitter.unwrap();
    let agents_offset = usize::from(agents.y) * 120 + usize::from(agents.x);
    assert_eq!(buffer.content[0].bg, super::palette().surface_alt);
    assert_eq!(
        buffer.content[36 * 120 - 1].bg,
        super::palette().surface_alt
    );
    assert_eq!(buffer.content[agents_offset].bg, super::palette().panel);
    let agents_header: String = (agents.x..agents.right())
        .map(|x| terminal.backend().buffer()[(x, agents.y)].symbol())
        .collect();
    assert!(agents_header.contains("AGENTS "));
    assert!(agents_header.contains('─'));
    assert!(!agents_header.contains("click focus"));
    assert!(
        (agents.x..agents.right())
            .all(|x| { terminal.backend().buffer()[(x, agents.y)].bg == super::palette().panel })
    );
    let header: String = terminal.backend().buffer().content[..120]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(header.contains("basetree"));
    assert!(header.contains("main"));
    let footer: String = terminal.backend().buffer().content[35 * 120..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(footer.contains("Tab Files"));
    assert!(footer.contains(&format!("{}:main", root.display())));
    assert!(!footer.contains("e Edit"));
    assert!(footer.contains("g Git Graph"));
    assert!(!footer.contains("W Worktrees"));
    assert!(!footer.contains("b Branches"));
    assert!(!footer.contains("r Refresh"));
    assert!(!footer.contains("1 Changes"));
    assert!(!footer.contains("2 Graph"));
    for shortcut in ["g Git Graph", "Tab Files", "o Explorer"] {
        let offset = footer.find(shortcut).unwrap();
        assert_eq!(
            terminal.backend().buffer().content[35 * 120 + offset].fg,
            super::palette().orange
        );
    }

    let graph_toggle = app.regions.graph.unwrap();
    click(&mut app, graph_toggle.x, graph_toggle.y);
    assert_eq!(app.view, View::Graph);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let commit = app.regions.commit.unwrap();
    let commit_text: String = (commit.y..commit.bottom())
        .flat_map(|y| (commit.x..commit.right()).map(move |x| (x, y)))
        .map(|position| terminal.backend().buffer()[position].symbol())
        .collect();
    assert!(commit_text.contains("Write a commit message"));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::CommitMessageGenerate)
            .is_some()
    );
    click(&mut app, graph_toggle.x, graph_toggle.y);
    assert_eq!(app.view, View::Changes);

    let generate = app
        .regions
        .hit_target_rect(HitTarget::CommitMessageGenerate)
        .unwrap();
    assert_eq!(generate.width, 3);
    assert_eq!(generate.x, app.regions.commit.unwrap().x);
    assert_eq!(generate.y, app.regions.commit.unwrap().bottom());
    assert_eq!(
        terminal.backend().buffer()[(generate.x + 1, generate.y)].bg,
        super::palette().raised
    );
    app.handle_mouse(mouse(MouseEventKind::Moved, generate.x + 1, generate.y));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(generate.x + 1, generate.y)].bg,
        super::palette().accent
    );
    app.handle_mouse(mouse(MouseEventKind::Moved, 0, 0));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let left_pane_toggle = app.regions.left_pane_toggle.unwrap();
    click(&mut app, left_pane_toggle.x, left_pane_toggle.y);
    assert_eq!(app.changes.pane, LeftPane::Files);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let footer: String = terminal.backend().buffer().content[35 * 120..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(footer.contains("Tab Changes"));
    assert!(footer.contains(&format!("{}:main", root.display())));
    assert!(!footer.contains("e Edit"));
    let left_pane_toggle = app.regions.left_pane_toggle.unwrap();
    click(&mut app, left_pane_toggle.x, left_pane_toggle.y);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let files_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::FilesTab))
        .unwrap();
    click(&mut app, files_tab.x, files_tab.y);
    assert_eq!(app.changes.pane, LeftPane::Files);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.commit.is_none());
    assert!(app.regions.agents_list.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.agents_list.is_none());
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.agents_list.is_some());
    let mut explorer = app.regions.explorer_list.unwrap();
    let directory_row = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "fixtures")
        })
        .unwrap();
    assert_eq!(
        app.changes.explorer_rows()[directory_row].directory_expanded,
        Some(false)
    );
    click(&mut app, explorer.x + 2, explorer.y + directory_row as u16);
    wait_for(&mut app, |app| {
        app.changes.explorer_rows().iter().any(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path.parent().is_some_and(|parent| parent == "fixtures"))
        })
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    explorer = app.regions.explorer_list.unwrap();
    let explorer_rows = app.changes.explorer_rows();
    let selected_file_row = explorer_rows
        .iter()
        .enumerate()
        .skip(app.changes.explorer_scroll)
        .take(usize::from(explorer.height))
        .find_map(|(index, row)| row.file_path.is_some().then_some(index))
        .unwrap();
    let selected_file = explorer_rows[selected_file_row]
        .file_path
        .as_ref()
        .unwrap()
        .clone();
    let selected_file_screen_row = selected_file_row - app.changes.explorer_scroll;
    click(
        &mut app,
        explorer.x + 2,
        explorer.y + selected_file_screen_row as u16,
    );
    wait_for_preview(&mut app);
    assert_eq!(app.selected_explorer_file_path(), Some(&selected_file));
    assert_eq!(
        app.changes.diff,
        fs::read_to_string(root.join(&selected_file)).unwrap()
    );
    let selected_before_scroll = app.changes.explorer_state.selected();
    let preview_before_scroll = app.changes.diff.clone();
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        explorer.x + 2,
        explorer.y + explorer.height.saturating_sub(1),
    ));
    assert_eq!(app.changes.explorer_scroll, 3);
    assert_eq!(
        app.changes.explorer_state.selected(),
        selected_before_scroll
    );
    assert_eq!(app.changes.diff, preview_before_scroll);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let visible_file = app.changes.explorer_rows()[app.changes.explorer_scroll..]
        .iter()
        .position(|row| row.file_path.is_some())
        .unwrap();
    click(&mut app, explorer.x + 2, explorer.y + visible_file as u16);
    assert_ne!(
        app.changes.explorer_state.selected(),
        selected_before_scroll
    );
    let file_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(file_screen.contains("FILE"));
    assert!(file_screen.contains("click to edit"));
    assert!(file_screen.contains("fixture"));

    let worktree_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    click(&mut app, worktree_tab.x, worktree_tab.y);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let stage_all = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::StageAll))
        .unwrap();
    assert!(stage_all.width > 2);
    click(&mut app, stage_all.x, stage_all.y);
    wait_for(&mut app, |app| {
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| change.staged)
    });
    assert!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| change.staged)
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let staged_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(staged_screen.contains('◉'));
    for (index, cell) in terminal.backend().buffer().content.iter().enumerate() {
        if cell.symbol() == "◉" {
            let trailing = &terminal.backend().buffer().content[index + 1];
            assert_eq!(trailing.symbol(), " ");
            assert_eq!(cell.bg, trailing.bg);
        }
    }
    let staged_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.label == "STAGED")
        .unwrap();
    let staged_target = app.changes.worktree_row_target(staged_row);
    let staged_title = app
        .regions
        .hit_target_rect(HitTarget::Changes(staged_target))
        .unwrap();
    click(&mut app, staged_title.x + 2, staged_title.y);
    assert_eq!(
        app.changes.selected_diff_section(),
        Some(crate::tree::WorktreeSection::Staged)
    );
    wait_for(&mut app, |app| {
        app.changes.diff.matches("diff --git").count() == 2
    });
    assert!(app.changes.diff.contains("tracked.txt"));
    assert!(app.changes.diff.contains("untracked.txt"));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let section_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(section_screen.contains("All staged changes"));
    let preview_body = app.regions.preview_body.unwrap();
    let changed_row = (preview_body.y..preview_body.bottom())
        .find(|row| {
            (preview_body.x..preview_body.right())
                .map(|column| terminal.backend().buffer()[(column, *row)].symbol())
                .collect::<String>()
                .contains("changed")
        })
        .unwrap();
    click(&mut app, preview_body.x + 8, changed_row);
    assert_eq!(app.mode, Mode::FileEdit);
    assert_eq!(
        app.file_editor.as_ref().unwrap().path(),
        &RepoPath::from("tracked.txt")
    );
    assert_eq!(app.file_editor.as_ref().unwrap().cursor_position().0, 0);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let stage_all = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::StageAll))
        .unwrap();
    click(&mut app, stage_all.x, stage_all.y);
    wait_for(&mut app, |app| {
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.staged)
    });
    assert!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.staged)
    );

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let unstaged_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(unstaged_screen.contains('○'));
    assert!(!unstaged_screen.contains("[ ]"));
    let selected = app.changes.worktree_state.selected().unwrap();
    let stage_target = app.changes.worktree_stage_target(selected);
    let status = app
        .regions
        .hit_target_rect(HitTarget::Changes(stage_target))
        .unwrap();
    assert_eq!(status.width, 2);
    click(&mut app, status.x, status.y);
    wait_for(&mut app, |app| {
        app.repository()
            .unwrap()
            .changes
            .iter()
            .filter(|change| change.staged)
            .count()
            == 1
    });
    assert_eq!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .filter(|change| change.staged)
            .count(),
        1
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let rows = app.changes.worktree_rows(app.repository().unwrap());
    assert!(rows.iter().any(|row| row.label == "STAGED"));
    assert!(rows.iter().any(|row| row.label == "UNSTAGED"));
    let selected = app.changes.worktree_state.selected().unwrap();
    let stage_target = app.changes.worktree_stage_target(selected);
    let status = app
        .regions
        .hit_target_rect(HitTarget::Changes(stage_target))
        .unwrap();
    click(&mut app, status.x, status.y);
    wait_for(&mut app, |app| {
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.staged)
    });
    assert!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.staged)
    );

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktree = app.regions.worktree_list.unwrap();
    let tracked_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.label == "tracked.txt")
        .unwrap();
    let tracked_y = worktree.y + (tracked_row - app.changes.worktree_scroll) as u16;
    click(&mut app, worktree.x + 10, tracked_y);
    assert_eq!(app.changes.worktree_state.selected(), Some(tracked_row));

    let splitter = app.regions.splitter.unwrap();
    let bounds = app.regions.split_bounds.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        splitter.x,
        splitter.y + 2,
    ));
    let target = bounds.x + 65;
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        target,
        splitter.y + 2,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        target,
        splitter.y + 2,
    ));
    assert_eq!(app.settings.worktree_width, 65);
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains("worktree_width=65")
    );
    assert!(!app.dragging_splitter);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agents_splitter = app.regions.agents_splitter.unwrap();
    let commit = app.regions.commit.unwrap();
    let actions = app.regions.actions.unwrap();
    let worktree = app.regions.worktree_list.unwrap();
    assert_eq!(actions.y, commit.bottom());
    assert_eq!(actions.right(), commit.right());
    assert_eq!(actions.bottom(), worktree.y);
    assert!(commit.bottom() <= agents_splitter.y);
    let agents_bounds = app.regions.agents_bounds.unwrap();
    let agents_target = agents_bounds.bottom().saturating_sub(9);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        agents_splitter.right().saturating_sub(2),
        agents_splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        agents_splitter.right().saturating_sub(2),
        agents_target,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        agents_splitter.right().saturating_sub(2),
        agents_target,
    ));
    assert_eq!(app.settings.agents_height, 9);
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains("agents_height=9")
    );
    assert!(!app.dragging_agents);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [],
                "agents": [{
                    "agent": "opencode",
                    "agent_status": "idle",
                    "focused": true,
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                }]
            }
        }
    }));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let agents = app.regions.agents_list.unwrap();
    click(&mut app, agents.x + 2, agents.y);
    assert_eq!(app.mode, Mode::Normal);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktree = app.regions.worktree_list.unwrap();
    let tracked_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.label == "tracked.txt")
        .unwrap();
    let tracked_y = worktree.y + (tracked_row - app.changes.worktree_scroll) as u16;
    click(&mut app, worktree.x + 2, tracked_y);
    wait_for_preview(&mut app);
    assert!(app.changes.diff.contains("tracked.txt"));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let diff = app.regions.diff.unwrap();
    let summary_row: String = (diff.x..diff.right())
        .map(|column| terminal.backend().buffer()[(column, diff.y + 4)].symbol())
        .collect();
    let files_row: String = (diff.x..diff.right())
        .map(|column| terminal.backend().buffer()[(column, diff.y + 5)].symbol())
        .collect();
    assert!(summary_row.contains("CHANGES"));
    assert!(summary_row.contains("+1"));
    assert!(summary_row.contains("-1"));
    assert!(files_row.contains("FILES"));
    assert!(files_row.contains("tracked.txt"));
    let tracked_diff = app.changes.diff.clone();
    app.changes.set_diff(
        concat!(
            "diff --git a/tracked.txt b/tracked.txt\n",
            "--- a/tracked.txt\n",
            "+++ b/tracked.txt\n",
            "@@ -1 +1 @@\n-old one\n+new one\n",
            "@@ -3 +3 @@\n-old two\n+new two\n",
        )
        .to_owned(),
    );
    app.changes.diff_scroll = 0;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let hunk_target = app.changes.hunk_action_target(0);
    let normal_hunk_y = app
        .regions
        .hit_target_rect(HitTarget::Changes(hunk_target))
        .unwrap()
        .y;
    let normal_scroll_max = app.regions.diff_scroll_max;
    let normal_scroll_thumb = app.regions.diff_scroll_thumb;
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        app.regions
            .hit_target_rect(HitTarget::Changes(hunk_target))
            .unwrap()
            .y,
        normal_hunk_y
    );
    assert_eq!(app.regions.diff_scroll_max, normal_scroll_max);
    assert_eq!(app.regions.diff_scroll_thumb, normal_scroll_thumb);
    assert_eq!(app.regions.diff_hunks.len(), 2);
    let pinned_hunk_y = app
        .regions
        .hit_target_rect(HitTarget::Changes(hunk_target))
        .unwrap()
        .y;
    let second_hunk = app.regions.diff_hunks[1].rect;
    app.handle_mouse(mouse(
        MouseEventKind::Moved,
        second_hunk.x + 1,
        second_hunk.y,
    ));
    assert_eq!(app.changes.hunk_selection, Some(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let selected_hunk = app
        .regions
        .diff_hunks
        .iter()
        .find(|hunk| hunk.index == 1)
        .unwrap();
    assert_eq!(selected_hunk.index, 1);
    let selected_hunk_target = app.changes.hunk_action_target(1);
    assert_eq!(
        app.regions
            .hit_target_rect(HitTarget::Changes(selected_hunk_target))
            .unwrap()
            .y,
        pinned_hunk_y
    );
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(1));
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, None);
    app.changes.set_diff(format!(
        "@@ -1,80 +1,80 @@\n{}",
        (0..80)
            .map(|line| format!(" line {line}\n"))
            .collect::<String>()
    ));
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.diff_hunks[0].continues_below);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    assert_eq!(app.changes.diff_scroll, 10);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.changes.diff_scroll, 10);
    assert!(app.regions.diff_hunks[0].continues_above);
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.changes.diff_scroll, 0);
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    app.changes.set_diff(tracked_diff);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.diff_hunks.len(), 1);
    let hunk_target = app.changes.hunk_action_target(0);
    let rect = app
        .regions
        .hit_target_rect(HitTarget::Changes(hunk_target))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let offset = usize::from(rect.y) * usize::from(buffer.area.width) + usize::from(rect.x);
    let button: String = buffer.content[offset..offset + 3]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(button, "[+]");
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.changes.hunk_selection, Some(0));
    wait_for(&mut app, |app| {
        app.repository()
            .unwrap()
            .changes
            .iter()
            .any(|change| change.path == "tracked.txt" && change.staged)
    });
    let rows = app.changes.worktree_rows(app.repository().unwrap());
    assert!(rows.iter().any(|row| row.label == "STAGED"));
    assert!(rows.iter().any(|row| row.label == "UNSTAGED"));

    app.changes.set_diff(
        (0..100)
            .map(|line| format!("+scrollbar line {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.changes.diff_scroll = 0;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let scrollbar = app.regions.diff_scrollbar.unwrap();
    assert_eq!(scrollbar.width, 1);
    assert_eq!(scrollbar.right(), 120);
    assert!(app.regions.diff_scroll_max > 0);
    assert!(app.regions.diff_scroll_thumb.is_some());
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        scrollbar.x,
        scrollbar.bottom() - 1,
    ));
    assert!(app.dragging_diff_scrollbar);
    assert!(app.changes.diff_scroll > 0);
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        scrollbar.x,
        scrollbar.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        scrollbar.x,
        scrollbar.y,
    ));
    assert_eq!(app.changes.diff_scroll, 0);
    assert!(!app.dragging_diff_scrollbar);

    app.changes.set_diff(
        (0..30_001)
            .map(|line| format!("+{line:05} {}", "x".repeat(200)))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.changes.diff_wrap = true;
    app.changes.diff_scroll = usize::MAX;
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [],
                "agents": [{
                    "agent": "opencode",
                    "agent_status": "working",
                    "focused": true,
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                }]
            }
        }
    }));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.changes.preview_presentation.is_windowed());
    assert!(app.changes.diff_scroll > usize::from(u16::MAX));
    app.changes.diff_wrap = false;

    let changes_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(changes_screen.contains("Write a commit message"));
    assert!(changes_screen.contains("AGENTS"));
    assert!(changes_screen.contains("terminal session"));
    assert!(changes_screen.contains("unassigned"));
    assert!(changes_screen.contains('⠋'));
    assert!(!changes_screen.contains("WORKING"));
    assert!(changes_screen.contains("ACTIONS"));
    assert!(app.regions.actions.is_some());
    assert!(app.regions.actions.unwrap().bottom() <= app.regions.worktree_list.unwrap().y);
    assert!(!changes_screen.contains("HEAD"));
    assert!(!changes_screen.contains("Render Test"));
    assert!(!changes_screen.contains("[Commit]"));
    assert!(!changes_screen.contains("COMMIT"));
    assert!(!changes_screen.contains('┌'));
    let actions = app.regions.actions.unwrap();
    click(&mut app, actions.x + 2, actions.y);
    assert_eq!(app.mode, Mode::ActionMenu);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let action_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(action_screen.contains("Pull --rebase"));
    assert!(action_screen.contains("Commit"));
    assert!(action_screen.contains("Run Git command"));
    let action_list = app.regions.action_list.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Moved,
        action_list.x + 2,
        action_list.y + 4,
    ));
    assert_eq!(app.actions.selection, 4);
    let background_before_command = terminal.backend().buffer().content[0].clone();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Command);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let command_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(command_screen.contains("GIT COMMAND"));
    assert!(command_screen.contains("Shell pipes"));
    assert!(app.regions.command_output.is_some());
    let command_overlay = app.regions.command_overlay.unwrap();
    let command_output = app.regions.command_output.unwrap();
    assert_eq!(
        command_output.bottom().saturating_add(1),
        command_overlay.bottom().saturating_sub(5)
    );
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let background = &buffer.content[0];
    let modal =
        &buffer.content[usize::from(command_overlay.y) * width + usize::from(command_overlay.x)];
    assert!(background.modifier.contains(Modifier::DIM));
    assert_eq!(background.fg, background_before_command.fg);
    assert_eq!(background.bg, Color::Rgb(0, 0, 0));
    assert!(!modal.modifier.contains(Modifier::DIM));
    assert_eq!(modal.bg, super::palette().surface_alt);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    let commit = app.regions.commit.unwrap();
    click(&mut app, commit.x + 2, commit.y + 1);
    assert_eq!(app.mode, Mode::Commit);
    app.commit_input.set("ac");
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    assert_eq!(app.commit_input.text(), "abc");
    app.commit_input.set("alpha beta");
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));
    assert_eq!(app.commit_input.text(), "alpha ");
    app.commit_input.set("alpha beta");
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    assert_eq!(app.commit_input.text(), "alpha ");
    app.commit_input.set("replace me");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let input_cell =
        &buffer.content[usize::from(commit.y) * width + usize::from(commit.x.saturating_add(1))];
    let focus_edge = &buffer.content[usize::from(commit.y) * width + usize::from(commit.x)];
    assert_eq!(input_cell.bg, super::palette().selected);
    assert_eq!(focus_edge.bg, super::palette().canvas);
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(app.commit_input.text(), "x");
    app.commit_input.set("Subject");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.commit_input.text(), "Subject\n");
    app.commit_input.insert("Body");
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    click(&mut app, commit.x + 3, commit.y + 1);
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    assert_eq!(app.commit_input.text(), "Subject\nBoXdy");
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE));
    assert_eq!(app.commit_input.text(), "SubYject\nBoXdy");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let unfocused_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(unfocused_screen.contains("SubYject"));
    assert!(unfocused_screen.contains("BoXdy"));

    app.mode = Mode::Commit;
    app.commit_input.set(
        (1..=10)
            .map(|line| format!("message line {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.commit_scroll = None;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let scroll_max = app.regions.commit_scroll_max;
    assert!(scroll_max > 2);
    assert_eq!(app.regions.commit_scroll, scroll_max);
    app.handle_mouse(mouse(MouseEventKind::ScrollUp, commit.x + 1, commit.y + 1));
    assert_eq!(app.commit_scroll, Some(scroll_max - 2));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.commit_scroll, scroll_max - 2);
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        commit.x + 1,
        commit.y + 1,
    ));
    assert_eq!(app.commit_scroll, Some(scroll_max));
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.commit_scroll, None);

    app.mode = Mode::Normal;
    app.commit_input
        .set(format!("wrap-start {} wrap-end", "x".repeat(90)));
    app.commit_scroll = None;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let wrapped_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(wrapped_screen.contains("wrap-start"));
    assert!(wrapped_screen.contains("wrap-end"));
    app.commit_input.set("Subject\nBody");

    app.mode = Mode::Commit;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let diff = app.regions.diff.unwrap();
    click(&mut app, diff.x + 1, diff.y + 1);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.commit_input.text(), "Subject\nBody");

    app.mode = Mode::Commit;
    app.commit_input.clear();
    app.notice = None;
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert_eq!(
        app.notice.as_deref(),
        Some("Commit message cannot be empty")
    );

    app.view = View::Graph;
    app.mode = Mode::Normal;
    let visible_oid = app.repository().unwrap().commits[0].oid.clone();
    wait_for(&mut app, |app| {
        app.commit_summaries.get(&visible_oid).is_some()
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let visible_summary = app.commit_summaries.get(&visible_oid).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("AUTHOR"));
    assert!(screen.contains("CHANGES"));
    assert!(screen.contains("DATE"));
    assert!(!screen.contains("ALL BRANCHES"));
    assert!(!screen.contains("date order"));
    assert!(screen.contains(&format!(
        "+{} -{}",
        visible_summary.additions, visible_summary.deletions
    )));
    assert!(screen.contains("HEAD"));
    assert!(screen.contains("Render Test"));
    assert!(!screen.contains("Detailed body line."));
    assert!(screen.contains("CHANGES"));
    assert!(screen.contains("o Explorer"));
    assert!(!screen.contains("scrollbar line"));
    assert_eq!(app.regions.graph_columns.len(), 5);
    assert!(
        app.regions
            .graph_columns
            .iter()
            .any(|column| column.right == GraphColumn::Description)
    );
    assert!(
        app.regions
            .graph_columns
            .iter()
            .any(|column| column.right == GraphColumn::Changes)
    );
    assert!(app.regions.graph_columns.iter().all(|column| {
        terminal.backend().buffer()[(column.splitter.x, column.splitter.y)].symbol() == "│"
    }));

    let date_column = app
        .regions
        .graph_columns
        .iter()
        .find(|column| column.right == GraphColumn::Date)
        .copied()
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        date_column.splitter.x,
        date_column.splitter.y,
    ));
    let resized_date_start = date_column.splitter.x.saturating_sub(1);
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        resized_date_start,
        date_column.splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        resized_date_start,
        date_column.splitter.y,
    ));
    let expected_date_width = date_column.right_width + 1;
    assert_eq!(app.settings.graph_date_width, expected_date_width);
    assert_eq!(app.settings.graph_changes_width, date_column.left_width - 1);
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains(&format!("graph_date_width={expected_date_width}"))
    );
    assert!(app.dragging_graph_column.is_none());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let commit_column = app
        .regions
        .graph_columns
        .iter()
        .find(|column| column.right == GraphColumn::Commit)
        .copied()
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        commit_column.splitter.x,
        commit_column.splitter.y,
    ));
    let wider_commit_start = commit_column.splitter.x.saturating_sub(3);
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        wider_commit_start,
        commit_column.splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        wider_commit_start,
        commit_column.splitter.y,
    ));
    let expected_commit_width = commit_column.right_width + 3;
    assert_eq!(app.settings.graph_commit_width, expected_commit_width);
    assert_eq!(
        app.settings.graph_author_width,
        commit_column.left_width - 3
    );
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains(&format!("graph_commit_width={expected_commit_width}"))
    );
    assert!(app.dragging_graph_column.is_none());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let worktree = app.regions.worktree.unwrap();
    let graph = app.regions.graph_table.unwrap();
    assert!(graph.x >= worktree.right());
    assert!(app.regions.diff.is_none());

    let author_header = app
        .regions
        .hit_target_rect(HitTarget::Graph(GraphHitTarget::AuthorHeader))
        .unwrap();
    assert_eq!(author_header.y, worktree.y + 1);
    click(&mut app, author_header.x, author_header.y);
    assert_eq!(app.mode, Mode::AuthorFilter);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let second_author = app
        .author_filter
        .entries()
        .iter()
        .position(|entry| entry.name == "Second Author")
        .unwrap();
    let second_author_row = app
        .regions
        .hit_target_rect(HitTarget::Graph(GraphHitTarget::FilterItem(second_author)))
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: second_author_row.x + 1,
        row: second_author_row.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.author_filter.state.selected(), Some(second_author));
    click(&mut app, second_author_row.x + 1, second_author_row.y);
    assert_eq!(app.visible_graph_indices().len(), 1);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(app.visible_graph_indices().len(), 2);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);

    let graph_offset = app.graph_state.offset();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: graph.x + 1,
        row: graph.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.graph_state.selected(), Some(1));
    assert_eq!(app.graph_state.offset(), graph_offset);
    assert!(!app.graph_commit_open);
    click(&mut app, graph.x + 1, graph.y + 1);
    assert_eq!(app.graph_state.selected(), Some(1));
    assert!(app.graph_commit_open);
    wait_for_preview(&mut app);
    assert!(app.changes.diff.contains("tracked.txt"));
    let commit_oid = app.repository().unwrap().commits[1].oid.clone();
    wait_for(&mut app, |app| {
        app.commit_summaries.get(&commit_oid).is_some()
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let commit_diff_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(commit_diff_screen.contains("initial commit"));
    assert!(commit_diff_screen.contains("Detailed body line."));
    assert!(commit_diff_screen.contains("Final note."));
    assert!(commit_diff_screen.contains("CHANGES"));
    assert!(commit_diff_screen.contains("FILES"));
    assert!(
        commit_diff_screen.contains("diff --git a/fixtures/file-00"),
        "the first file heading should be visible"
    );
    let file_header = app.regions.diff_file_headers[0].clone();
    let commit_diff = app.regions.diff.unwrap();
    let mut metadata_text = String::new();
    for row in commit_diff.y..file_header.rect.y {
        for column in commit_diff.x..commit_diff.right() {
            metadata_text.push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
        metadata_text.push('\n');
    }
    let selected_commit = app
        .repository()
        .unwrap()
        .commits
        .iter()
        .find(|commit| commit.oid == commit_oid)
        .unwrap()
        .clone();
    assert!(metadata_text.contains("COMMIT"));
    assert!(metadata_text.contains(&selected_commit.oid[..7]));
    assert!(metadata_text.contains(&selected_commit.author));
    assert!(metadata_text.contains(&selected_commit.date));
    assert!(metadata_text.contains("MESSAGE"));
    assert!(metadata_text.contains("FILES"));
    assert!(!metadata_text.contains("CHANGES"));
    let top_row = (commit_diff.x..commit_diff.right())
        .map(|column| terminal.backend().buffer()[(column, commit_diff.y + 1)].symbol())
        .collect::<String>();
    assert!(!top_row.contains("DIFF"));
    assert_eq!(file_header.path, RepoPath::from("fixtures/file-00.txt"));
    assert!(file_header.line > 0);
    app.handle_mouse(mouse(
        MouseEventKind::Moved,
        file_header.rect.x + 1,
        file_header.rect.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(file_header.rect.x, file_header.rect.y)].bg,
        super::palette().raised
    );
    click(&mut app, file_header.rect.x + 1, file_header.rect.y);
    assert_eq!(app.mode, Mode::FileEdit);
    assert_eq!(app.file_editor.as_ref().unwrap().path(), &file_header.path);
    assert_eq!(
        app.file_editor.as_ref().unwrap().cursor_position().0,
        file_header.line - 1
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let files_row = (commit_diff.y..commit_diff.bottom())
        .find(|row| {
            (commit_diff.x..commit_diff.right())
                .map(|column| terminal.backend().buffer()[(column, *row)].symbol())
                .collect::<String>()
                .contains("FILES")
        })
        .unwrap();
    let mut automatic_file_summary = String::new();
    for row in files_row..files_row.saturating_add(6).min(commit_diff.bottom()) {
        for column in commit_diff.x..commit_diff.right() {
            automatic_file_summary.push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
    }
    assert!(
        automatic_file_summary.contains("fixtures/file-05.txt"),
        "{automatic_file_summary:?}"
    );
    assert_eq!(
        terminal.backend().buffer()[(commit_diff.x + 1, files_row)].bg,
        super::palette().surface_alt
    );
    assert!(app.regions.graph_table.is_none());
    app.changes.diff_scroll = 2;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let mut scrolled_commit_diff = String::new();
    for row in commit_diff.y..commit_diff.bottom() {
        for column in commit_diff.x..commit_diff.right() {
            scrolled_commit_diff.push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
    }
    assert!(!scrolled_commit_diff.contains(&format!("COMMIT  {}", &commit_oid[..7])));
    assert!(scrolled_commit_diff.contains("MESSAGE"));
    assert!(!scrolled_commit_diff.contains("CHANGES"));
    app.changes.diff_scroll = 0;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.changes.diff_wrap = false;
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let mut wrapped_patch_file_summary = String::new();
    for row in files_row..files_row.saturating_add(6).min(commit_diff.bottom()) {
        for column in commit_diff.x..commit_diff.right() {
            wrapped_patch_file_summary
                .push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
    }
    assert!(wrapped_patch_file_summary.contains("fixtures/file-05.txt"));
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view, View::Graph);
    assert!(!app.graph_commit_open);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.graph_table.is_some());
    assert!(app.regions.diff_hunks.is_empty());
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.graph_state.selected(), Some(0));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.graph_commit_open);
    wait_for_preview(&mut app);
    assert!(app.changes.diff.contains("second.txt"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(
        app.workspace_explorer.directory,
        fs::canonicalize(root).unwrap()
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
    for character in "Project".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let explorer_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(explorer_screen.contains("EXPLORER"));
    assert!(explorer_screen.contains("Switch working directory"));
    assert!(explorer_screen.contains("AROUND HERE"));
    assert!(explorer_screen.contains("CONTENTS"));
    assert!(explorer_screen.contains("★ Project"));
    assert!(!explorer_screen.contains("F2 WORKTREES"));
    assert!(!explorer_screen.contains("F3 BRANCHES"));
    assert!(!explorer_screen.contains("OPEN REPOSITORY"));
    assert!(!explorer_screen.contains('┌'));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::SurroundingsPane))
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::EntriesPane))
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Explorer(
                app.workspace_explorer.favorite_target(0)
            ))
            .is_some()
    );
    let left_width = app
        .regions
        .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::SurroundingsPane))
        .unwrap()
        .width;
    let splitter = app
        .regions
        .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::Splitter))
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        splitter.x,
        splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        splitter.x + 8,
        splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        splitter.x + 8,
        splitter.y,
    ));
    assert!(!app.workspace_explorer.dragging_splitter);
    assert_eq!(
        app.settings_store.load().explorer_left_pane_width,
        app.workspace_explorer.left_pane_width
    );
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::SurroundingsPane))
            .unwrap()
            .width
            > left_width
    );
    let path = app
        .regions
        .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::Path))
        .unwrap();
    click(&mut app, path.x + 2, path.y + 1);
    assert!(app.workspace_explorer.editing_path);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let explorer_search_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(explorer_search_screen.contains("PATH MATCHES"));
    assert!(explorer_search_screen.contains("LIVE PREVIEW"));
    assert!(explorer_search_screen.contains("Ctrl/Alt+BS segment"));
    assert!(!explorer_search_screen.contains('⌫'));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::PreviewPane))
            .is_some()
    );

    app.mode = Mode::Settings;
    app.settings = Settings::default();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let settings_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(settings_screen.contains("Auto-fetch remotes"));
    assert!(settings_screen.contains("Fetch interval"));
    assert!(settings_screen.contains("Format on save"));
    assert!(settings_screen.contains("Cross-workspace agents"));
    assert!(settings_screen.contains("Agent harness"));
    assert!(settings_screen.contains("Agent time"));
    assert!(settings_screen.contains("Latest loop"));
    assert!(settings_screen.contains("Agent timing history"));
    assert!(settings_screen.contains("Media protocol"));
    assert!(settings_screen.contains("Auto"));
    assert!(settings_screen.contains("Editor command"));
    assert!(!settings_screen.contains('┌'));
    let auto_fetch = app.regions.auto_fetch.unwrap();
    let auto_switch_x = auto_fetch.right().saturating_sub(6);
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(auto_switch_x + 1, auto_fetch.y)].symbol(), "◼");
    assert!(
        (auto_switch_x..auto_switch_x + 5)
            .all(|x| buffer[(x, auto_fetch.y)].bg == super::palette().faint)
    );
    assert!(app.regions.fetch_interval_up.is_some());
    let format_on_save_setting = app.regions.format_on_save_setting.unwrap();
    let cross_workspace_setting = app.regions.cross_workspace_agents_setting.unwrap();
    let agent_harness_setting = app.regions.agent_harness_setting.unwrap();
    let agent_time_setting = app.regions.agent_time_setting.unwrap();
    let clear_agent_timings_setting = app.regions.clear_agent_timings_setting.unwrap();
    let media_preview_setting = app.regions.media_preview_setting.unwrap();
    let editor_setting = app.regions.editor_setting.unwrap();
    assert_eq!(cross_workspace_setting.y, format_on_save_setting.y + 4);
    assert_eq!(agent_harness_setting.y, cross_workspace_setting.y + 2);
    assert_eq!(agent_time_setting.y, agent_harness_setting.y + 2);
    assert_eq!(clear_agent_timings_setting.y, agent_time_setting.y + 2);
    assert_eq!(media_preview_setting.y, clear_agent_timings_setting.y + 2);
    assert_eq!(editor_setting.y, media_preview_setting.y + 2);
    let harness_switch_x = agent_harness_setting.right().saturating_sub(6);
    assert_eq!(
        buffer[(harness_switch_x + 1, agent_harness_setting.y)].symbol(),
        "◼"
    );
    assert!(
        (harness_switch_x..harness_switch_x + 5)
            .all(|x| buffer[(x, agent_harness_setting.y)].bg == super::palette().faint)
    );

    app.settings_page = SettingsPage::OpenCode;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let opencode_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(opencode_screen.contains("OpenCode"));
    assert!(opencode_screen.contains("deepseek-v4-flash-free"));
    assert!(opencode_screen.contains("Reasoning"));
    assert!(opencode_screen.contains("Max"));
    let model_row = app.regions.opencode_model_setting.unwrap();
    click(&mut app, model_row.x + 1, model_row.y);
    assert!(app.opencode_model_input.is_some());
    app.opencode_model_input = None;

    app.settings_page = SettingsPage::Shortcuts;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let shortcuts_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(shortcuts_screen.contains("Shortcuts"));
    assert!(shortcuts_screen.contains("Changes / files"));
    assert!(shortcuts_screen.contains("Show / hide Git graph"));
    assert!(!app.regions.shortcut_rows.is_empty());
    let explorer_row = app
        .regions
        .shortcut_rows
        .iter()
        .find(|(action, _)| *action == ShortcutAction::OpenExplorer)
        .map(|(_, rect)| *rect)
        .unwrap();
    click(&mut app, explorer_row.x + 1, explorer_row.y);
    assert!(app.shortcut_capture);
    app.shortcut_capture = false;
    app.settings_page = SettingsPage::General;

    app.mode = Mode::Editor;
    app.editor_input = "nvim".to_owned();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let editor_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(editor_screen.contains("EDITOR COMMAND"));
    assert!(editor_screen.contains("nvim"));
    assert!(editor_screen.contains("Saved for next time"));
    assert!(app.regions.editor_overlay.is_some());

    app.mode = Mode::Help;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let help_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(help_screen.contains("KEYBOARD"));
    assert!(help_screen.contains("Ctrl+Enter"));
    assert!(help_screen.contains("Explorer"));
    assert!(!help_screen.contains('┌'));

    let mut narrow = Terminal::new(TestBackend::new(60, 16)).unwrap();
    narrow.draw(|frame| draw(frame, &mut app)).unwrap();
}

fn wait_for_preview(app: &mut App) {
    for _ in 0..100 {
        let _ = app.poll_worker();
        if app.changes.diff != "Loading preview…" || app.changes.preview_image.is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!("preview did not complete");
}

fn wait_for_halfblock_render(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let _ = app.poll_worker();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let width = usize::from(buffer.area.width);
        if buffer
            .content
            .iter()
            .enumerate()
            .any(|(index, cell)| cell.symbol() == "▀" && index / width != 1)
        {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("media preview did not render");
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let _ = app.poll_worker();
        if predicate(app) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("application state did not update");
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn click(app: &mut App, column: u16, row: u16) {
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), column, row));
    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), column, row));
}

fn run_git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_output(root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
