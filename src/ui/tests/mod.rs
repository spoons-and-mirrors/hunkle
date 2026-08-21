pub(super) use std::{fs, process::Command, thread, time::Duration};

pub(super) use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
pub(super) use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::{Position, Rect},
    style::{Color, Modifier},
};
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::app::{
    AgentActivityPreview, AgentListMode, AgentPaneDirection, App, ChangesHitTarget,
    CommitMessageGenerator, ExplorerHitTarget, FOOTER_MARQUEE_PAUSE, FOOTER_MARQUEE_STEP,
    GraphColumn, GraphHitTarget, HeaderPickerItem, HeaderPickerKind, HerdrPaneLayout,
    HerdrPaneRect, HerdrSession, HitTarget, LeftPane, Mode, SchedulerHitTarget, ScrollTarget,
    Settings, SettingsHitTarget, SettingsPage, SettingsStore, ShortcutAction, SqliteFocus,
    StashedAgent, View,
};
pub(super) use crate::repo_path::RepoPath;

pub(super) use super::{
    BranchPickerStep, changes, display_path, draw, marquee_window, notice_is_error, palette,
    selected_display_range, text, wrapped_editor_cursor,
};

mod agents;
mod editor;
mod files;
mod header;
mod media;
mod scheduler;
mod sqlite;

fn assert_black_underlay(terminal: &Terminal<TestBackend>) {
    let background = &terminal.backend().buffer()[(0, 0)];
    assert_eq!(background.bg, Color::Rgb(0, 0, 0));
    assert!(background.modifier.contains(Modifier::DIM));
}

fn enable_herdr(app: &mut App) {
    app.herdr = HerdrSession::ready_for_test(&serde_json::json!({
        "result": { "snapshot": {
            "workspaces": [],
            "agents": [],
            "panes": []
        } }
    }));
}

fn screen_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn columns_render_one_empty_workspace_without_a_repository() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().join("missing"));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal
        .draw(|frame| {
            changes::draw(
                frame,
                &mut app,
                changes::ChangesPlan::Columns {
                    areas: [Rect::new(0, 0, 38, 30), Rect::new(39, 0, 61, 30)],
                    sidebar_pane: LeftPane::Worktree,
                    preview_pane: Some(LeftPane::Worktree),
                    agents: changes::ColumnAgents::Hidden,
                },
            );
        })
        .unwrap();

    assert_eq!(
        screen_text(&terminal)
            .matches("Open a repository to inspect its changes")
            .count(),
        1
    );
}

#[test]
fn pull_request_preview_composes_description_and_diff() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    app.changes.set_pull_request_for_test(
        "## Context\nReview this change.",
        concat!(
            "diff --git a/file.txt b/file.txt\n",
            "--- a/file.txt\n",
            "+++ b/file.txt\n",
            "@@ -1 +1 @@\n",
            "-before\n",
            "+after\n",
        )
        .to_owned(),
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal
        .draw(|frame| {
            changes::draw(
                frame,
                &mut app,
                changes::ChangesPlan::Columns {
                    areas: [Rect::new(0, 0, 38, 30), Rect::new(39, 0, 61, 30)],
                    sidebar_pane: LeftPane::Files,
                    preview_pane: Some(LeftPane::Files),
                    agents: changes::ColumnAgents::Hidden,
                },
            );
        })
        .unwrap();

    let rendered = screen_text(&terminal);
    assert!(rendered.contains("PULL #17"), "{rendered}");
    assert!(rendered.contains("CHANGES topic -> main"));
    assert!(rendered.contains("DESCRIPTION"));
    assert!(rendered.contains("Review this change."));
    assert!(rendered.contains("file.txt"));
    assert!(rendered.contains("+after"));
}

#[test]
fn footer_abbreviates_paths_under_home() {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
        return;
    };
    let path = std::path::PathBuf::from(home).join("project");
    assert_eq!(display_path(&path), "~/project");
}

#[test]
fn successful_stash_notice_is_not_an_error_when_the_session_title_contains_error() {
    assert!(!notice_is_error(
        "Stashed agent Sleev gateway launchctl bootstrap error"
    ));
    assert!(notice_is_error(
        "Could not stash agent: Sleev gateway launchctl bootstrap error"
    ));
}

#[test]
fn overflowing_footer_path_scrolls_out_waits_then_scrolls_back() {
    let path = " /projects/a-very-long-repository:main";
    let width = 20;
    assert_eq!(marquee_window(path, width, 0), " /projects/a-very-lo");
    assert_eq!(marquee_window(path, width, 1), "/projects/a-very-lon");

    let travel = UnicodeWidthStr::width(path) - width;
    let pause_frames =
        (FOOTER_MARQUEE_PAUSE.as_millis() / FOOTER_MARQUEE_STEP.as_millis()) as usize;
    assert_eq!(marquee_window(path, width, travel), "long-repository:main");
    assert_eq!(
        marquee_window(path, width, travel + pause_frames),
        "long-repository:main"
    );
    assert_eq!(
        marquee_window(path, width, travel + pause_frames + 1),
        "-long-repository:mai"
    );
    assert_eq!(
        marquee_window(path, width, travel * 2 + pause_frames),
        " /projects/a-very-lo"
    );
}

#[test]
fn narrow_footer_only_shows_the_repository_path() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let footer: String = terminal.backend().buffer().content[47 * 49..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(footer.trim(), format!("{}:main", root.display()));
    assert!(app.regions.changes.is_none());
    assert!(app.regions.graph.is_none());
    assert!(app.regions.left_pane_toggle.is_none());
    assert!(app.regions.explorer.is_none());
    assert!(app.regions.settings.is_none());
    assert!(app.regions.help.is_none());
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
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.view(), View::Graph);
    assert_eq!(app.visible_view(), View::Graph);

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.graph_table.is_some());
    assert!(app.regions.diff.is_none());

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.graph_commit_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("COMMIT"));
    assert!(screen.contains("MESSAGE"));

    let mut narrow_terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    narrow_terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.worktree.is_none());
    assert_eq!(app.regions.diff.unwrap().width, 49);
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(!app.graph_commit_open());
    narrow_terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.graph_table.is_some());
    assert!(app.regions.diff.is_none());

    fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
    let mut dirty_app = App::new(root.to_path_buf());
    assert_eq!(dirty_app.sidebar_pane(), LeftPane::Worktree);
    assert_eq!(dirty_app.visible_view(), View::Changes);
    terminal.draw(|frame| draw(frame, &mut dirty_app)).unwrap();
    assert!(dirty_app.regions.graph_table.is_none());
    assert!(dirty_app.regions.diff.is_some());
}

#[test]
fn narrow_explorer_splitter_drag_stays_owned_by_the_modal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::write(root.join("file.txt"), "content\n").unwrap();
    let mut app = App::new(root.to_path_buf());
    wait_for(&mut app, |app| !app.workspace_loading_initial_state());
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Explorer);

    let mut terminal = Terminal::new(TestBackend::new(49, 48)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let splitter = app
        .regions
        .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::Splitter))
        .unwrap();
    let initial_width = app.workspace_explorer.left_pane_width;

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        splitter.x,
        splitter.y,
    ));
    assert!(app.workspace_explorer.dragging_splitter);
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        splitter.x + 4,
        splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        splitter.x + 4,
        splitter.y,
    ));

    assert!(!app.workspace_explorer.dragging_splitter);
    assert!(app.workspace_explorer.left_pane_width > initial_width);
}

#[test]
fn standalone_hides_herdr_surfaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = screen_text(&terminal);
    assert!(app.regions.agents_splitter.is_none());
    assert!(
        app.regions
            .hit_target_rect(HitTarget::HeaderAgent)
            .is_none()
    );
    assert!(!screen.contains("F3 Agents"));

    app.mode = Mode::Help;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = screen_text(&terminal);
    for label in [
        "Toggle fullscreen",
        "Show Agents",
        "Send to Herdr",
        "Start agent",
        "Cycle agents",
    ] {
        assert!(!screen.contains(label));
    }

    app.mode = Mode::Settings;
    app.settings_state.page = SettingsPage::General;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = screen_text(&terminal);
    assert!(screen.contains("Media protocol"));
    assert!(screen.contains("Editor command"));
    for label in [
        "Cross-workspace agents",
        "Agent harness",
        "Agent card click",
        "Agent preview split",
        "Agent time",
        "Agent timing history",
    ] {
        assert!(!screen.contains(label));
    }
    for target in [
        SettingsHitTarget::CrossWorkspaceAgents,
        SettingsHitTarget::AgentHarness,
        SettingsHitTarget::AgentCardClick,
        SettingsHitTarget::AgentPreviewSplit,
        SettingsHitTarget::AgentTime,
        SettingsHitTarget::ClearAgentTimings,
    ] {
        assert!(
            app.regions
                .hit_target_rect(HitTarget::Settings(target))
                .is_none()
        );
    }

    app.settings_state.page = SettingsPage::Shortcuts;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = screen_text(&terminal);
    assert!(screen.contains("Show Changes"));
    assert!(!screen.contains("Show Agents"));
    assert!(!screen.contains("Send to Herdr pane"));
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
    enable_herdr(&mut app);
    app.settings.graph_lane_width = 0;
    app.settings.graph_description_width = 0;
    app.settings.graph_changes_width = 12;
    app.settings.graph_date_width = 12;
    app.settings.graph_author_width = 16;
    app.settings.graph_commit_width = 7;
    app.commit_message_generator = CommitMessageGenerator::ready_for_test();
    let settings_path = root.join(".git/hunkle-test-config");
    app.settings_store = SettingsStore::at(settings_path.clone());
    let mut terminal = Terminal::new(TestBackend::new(120, 37)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.worktree.unwrap().x, 0);
    assert_eq!(app.regions.worktree.unwrap().y, 2);
    assert_eq!(app.regions.diff.unwrap().right(), 120);
    let left = app.regions.worktree.unwrap();
    let right = app.regions.diff.unwrap();
    let footer_y = terminal.backend().buffer().area.height - 1;
    let transition_y = footer_y - 1;
    assert_eq!(left.bottom(), transition_y);
    assert_eq!(right.bottom(), transition_y);
    assert_eq!(app.regions.preview_body.unwrap().bottom(), transition_y);
    for point in [(left.x, left.y), (right.right().saturating_sub(1), right.y)] {
        let cell = &terminal.backend().buffer()[point];
        assert_eq!(cell.symbol(), " ");
        assert_eq!(cell.bg, super::palette().canvas);
    }
    for x in [left.x, right.right().saturating_sub(1)] {
        let cell = &terminal.backend().buffer()[(x, transition_y)];
        assert_eq!(cell.symbol(), "▀");
        assert_eq!(cell.fg, super::palette().panel);
        assert_eq!(cell.bg, super::palette().canvas);
    }
    let transition_splitter = &terminal.backend().buffer()[(left.right(), transition_y)];
    assert_eq!(transition_splitter.symbol(), " ");
    assert_eq!(transition_splitter.bg, super::palette().canvas);
    let splitter_top = &terminal.backend().buffer()[(left.right(), left.y)];
    assert_eq!(splitter_top.bg, super::palette().canvas);
    app.dragging_splitter = true;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(left.right(), left.y)].bg,
        super::palette().canvas
    );
    assert_eq!(
        terminal.backend().buffer()[(left.right(), left.y + 1)].bg,
        super::palette().accent
    );
    app.dragging_splitter = false;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(left.right(), left.y + 2)].bg,
        super::palette().canvas
    );
    assert!(app.regions.changes.is_none());
    assert_eq!(app.regions.graph.unwrap().y, 36);
    assert_eq!(app.regions.help.unwrap().y, 36);
    assert!(app.regions.graph.unwrap().x > 0);
    let schedule = app
        .regions
        .hit_target_rect(HitTarget::HeaderSchedule)
        .unwrap();
    assert_eq!(app.regions.help.unwrap().right(), schedule.x);
    assert_eq!(schedule.right(), 120);
    let buffer = terminal.backend().buffer();
    let agents = app.regions.agents_splitter.unwrap();
    let agents_offset = usize::from(agents.y) * 120 + usize::from(agents.x);
    assert_eq!(buffer.content[0].bg, super::palette().canvas);
    assert_eq!(buffer.content[37 * 120 - 1].bg, super::palette().canvas);
    assert_eq!(buffer.content[agents_offset].bg, super::palette().panel);
    let agents_header: String = (agents.x..agents.right())
        .map(|x| terminal.backend().buffer()[(x, agents.y)].symbol())
        .collect();
    assert!(agents_header.contains(" SCHEDULED "));
    assert!(!agents_header.contains("click focus"));
    let stash_toggle = app
        .regions
        .hit_target_rect(HitTarget::AgentListModeToggle)
        .unwrap();
    assert!(
        (agents.x..stash_toggle.x)
            .chain(stash_toggle.right()..agents.right())
            .all(|x| { terminal.backend().buffer()[(x, agents.y)].bg == super::palette().panel })
    );
    assert!(agents_header.contains("AGENTS "));
    assert!(agents_header.contains('─'));
    let header: String = terminal.backend().buffer().content[120..240]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(header.contains("basetree"));
    assert!(header.contains("main"));
    let footer: String = terminal.backend().buffer().content[36 * 120..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(footer.contains("F2 Files"));
    assert!(footer.contains(&format!("{}:main", root.display())));
    assert!(!footer.contains("e Edit"));
    assert!(footer.contains("g Git Graph"));
    assert!(!footer.contains("W Worktrees"));
    assert!(!footer.contains("b Branches"));
    assert!(!footer.contains("r Refresh"));
    assert!(!footer.contains("1 Changes"));
    assert!(!footer.contains("2 Graph"));
    for shortcut in ["g Git Graph", "F2 Files", "o Explorer"] {
        let offset = footer.find(shortcut).unwrap();
        assert_eq!(
            terminal.backend().buffer().content[36 * 120 + offset].fg,
            super::palette().orange
        );
    }

    let graph_toggle = app.regions.graph.unwrap();
    click(&mut app, graph_toggle.x, graph_toggle.y);
    assert_eq!(app.view(), View::Graph);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.graph_table.unwrap().bottom(), transition_y);
    let commit = app.regions.commit.unwrap();
    assert_eq!(
        terminal.backend().buffer()[(commit.x, commit.y)].symbol(),
        "▐"
    );
    assert_eq!(
        terminal.backend().buffer()[(commit.right() - 1, commit.y)].symbol(),
        "▌"
    );
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
    assert_eq!(app.view(), View::Changes);

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
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let footer: String = terminal.backend().buffer().content[36 * 120..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(footer.contains("F3 Agents"));
    assert!(footer.contains(&format!("{}:main", root.display())));
    assert!(!footer.contains("e Edit"));
    let left_pane_toggle = app.regions.left_pane_toggle.unwrap();
    click(&mut app, left_pane_toggle.x, left_pane_toggle.y);
    assert!(app.agents_pane_visible());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let files_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::FilesTab))
        .unwrap();
    click(&mut app, files_tab.x, files_tab.y);
    assert_eq!(app.sidebar_pane(), LeftPane::Files);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.commit.is_none());
    assert!(app.regions.agents_list.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.agents_list.is_some());
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Scheduled);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.agents_list.is_some());
    assert_eq!(app.herdr.agent_list_mode(), AgentListMode::Stash);
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
        app.changes.preview.text().unwrap(),
        fs::read_to_string(root.join(&selected_file)).unwrap()
    );
    let selected_before_scroll = app.changes.explorer_state.selected();
    let preview_before_scroll = app.changes.preview.text().unwrap().to_owned();
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
    assert_eq!(
        app.changes.preview.text(),
        Some(preview_before_scroll.as_str())
    );
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
    assert_eq!(app.sidebar_pane(), LeftPane::Worktree);
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
    let staged_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.label == "STAGED")
        .unwrap();
    app.changes.worktree_scroll = staged_row;
    app.changes.worktree_scroll_to_selection = false;
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
        app.changes
            .preview
            .text()
            .unwrap_or_default()
            .matches("diff --git")
            .count()
            == 2
    });
    assert!(app.changes.preview.text().unwrap().contains("tracked.txt"));
    assert!(
        app.changes
            .preview
            .text()
            .unwrap()
            .contains("untracked.txt")
    );
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
    assert_eq!(actions.bottom().saturating_add(1), worktree.y);
    assert!(commit.bottom() <= agents_splitter.y);
    let agents_bounds = app.regions.agents_bounds.unwrap();
    assert_eq!(agents_bounds.y, worktree.y.saturating_add(1));
    let agents_target = agents_bounds.bottom().saturating_sub(9);
    let agents_resize_x = agents_splitter.x;
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        agents_resize_x,
        agents_splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        agents_resize_x,
        agents_target,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        agents_resize_x,
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
                }],
                "panes": [{
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1",
                    "focused": true
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
    assert!(app.changes.preview.text().unwrap().contains("tracked.txt"));
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
    let tracked_diff = app.changes.preview.text().unwrap().to_owned();
    app.changes.set_diff_for_test(
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
    app.changes.set_diff_for_test(format!(
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
    app.changes.set_diff_for_test(tracked_diff);
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

    app.changes.set_diff_for_test(
        (0..100)
            .map(|line| format!("+scrollbar line {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.changes.diff_scroll = 0;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.changes.diff_scroll = 2;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let diff = app.regions.diff.unwrap();
    let buffer = terminal.backend().buffer();
    let scrolled_diff = (diff.y..diff.bottom())
        .flat_map(|row| (diff.x..diff.right()).map(move |column| buffer[(column, row)].symbol()))
        .collect::<String>();
    assert!(scrolled_diff.contains("DIFF"));
    assert!(!scrolled_diff.contains("CHANGES"));
    assert!(scrolled_diff.contains("FILES"));
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

    app.changes.set_diff_for_test(
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
                }],
                "panes": [{
                    "pane_id": "w1:p1",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1",
                    "focused": true
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
    assert_eq!(
        app.regions
            .scroll_target_at(Position::new(action_list.x, action_list.y)),
        Some(ScrollTarget::ActionMenu)
    );
    let diff = app.regions.diff.unwrap();
    let workspace_point = Position::new(diff.right() - 1, diff.bottom() - 1);
    assert!(!action_list.contains(workspace_point));
    assert_eq!(app.regions.scroll_target_at(workspace_point), None);
    app.changes.diff_scroll = 0;
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        workspace_point.x,
        workspace_point.y,
    ));
    assert_eq!(app.changes.diff_scroll, 0);
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
        app.regions
            .scroll_target_at(Position::new(command_output.x, command_output.y)),
        Some(ScrollTarget::CommandOutput)
    );
    let workspace_point = (diff.y..diff.bottom())
        .flat_map(|y| (diff.x..diff.right()).map(move |x| Position::new(x, y)))
        .find(|point| !command_output.contains(*point))
        .unwrap();
    assert_eq!(app.regions.scroll_target_at(workspace_point), None);
    app.changes.diff_scroll = 0;
    app.handle_mouse(mouse(
        MouseEventKind::ScrollDown,
        workspace_point.x,
        workspace_point.y,
    ));
    assert_eq!(app.changes.diff_scroll, 0);
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
    assert_eq!(focus_edge.symbol(), "▐");
    assert_eq!(focus_edge.fg, super::palette().canvas);
    assert_eq!(focus_edge.bg, super::palette().panel);
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

    app.set_view_for_test(View::Graph);
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
    assert!(screen.contains(&format!("+{}", visible_summary.additions)));
    assert!(screen.contains(&format!("-{}", visible_summary.deletions)));
    assert!(screen.contains("HEAD"));
    assert!(screen.contains("Render Test"));
    assert!(!screen.contains("Detailed body line."));
    assert!(screen.contains("Press Space and search by description"));
    assert!(screen.contains("CHANGES"));
    assert!(screen.contains("o Explorer"));
    assert!(!screen.contains("scrollbar line"));
    assert_eq!(app.regions.graph_columns.len(), 5);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(app.graph_search_focused);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let graph_search = app
        .regions
        .hit_target_rect(HitTarget::Graph(GraphHitTarget::Search))
        .unwrap();
    click(&mut app, graph_search.x + 2, graph_search.y);
    assert!(app.graph_search_focused);
    app.handle_paste("initial commit");
    assert_eq!(app.visible_graph_indices(), &[0, 1]);
    assert_eq!(app.graph_state.selected(), Some(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(screen_text(&terminal).contains("1/1"));
    let graph = app.regions.graph_table.unwrap();
    assert_eq!(graph_search.x + 1, graph.x);
    assert_eq!(graph_search.right(), graph.right() + 1);
    assert_eq!(
        terminal.backend().buffer()[(graph_search.right() - 3, graph_search.y)].symbol(),
        "1"
    );
    assert_eq!(
        terminal.backend().buffer()[(graph_search.right() - 2, graph_search.y)].symbol(),
        " "
    );
    assert_eq!(
        terminal.backend().buffer()[(graph_search.right() - 1, graph_search.y)].symbol(),
        " "
    );
    assert_eq!(
        terminal.backend().buffer()[(graph.x, graph.y)].bg,
        palette().surface_alt
    );
    assert_eq!(
        terminal.backend().buffer()[(graph.x, graph.y + 1)].bg,
        palette().selected
    );
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.graph_search_focused);
    assert!(app.graph_commit_open());
    app.set_graph_commit_open_for_test(false);
    click(&mut app, graph_search.x + 2, graph_search.y);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.graph_search_focused);
    assert_eq!(app.visible_graph_indices(), &[0, 1]);
    click(&mut app, graph_search.x + 2, graph_search.y);
    app.handle_paste("commit");
    assert_eq!(app.graph_state.selected(), Some(0));
    assert_eq!(app.graph_search.match_status(), Some((1, 2)));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.graph_state.selected(), Some(1));
    assert_eq!(app.graph_search.match_status(), Some((2, 2)));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.graph_state.selected(), Some(0));
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.graph_state.selected(), Some(1));
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.graph_state.selected(), Some(0));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let graph = app.regions.graph_table.unwrap();
    let head = &app.repository().unwrap().commits[0];
    assert_eq!(
        terminal.backend().buffer()[(graph.x, graph.y)].bg,
        super::history::commit_graph_highlight(head)
    );
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
        terminal.backend().buffer()[(column.splitter.right() - 1, column.splitter.y)].symbol()
            == "│"
    }));
    assert!(
        app.regions
            .graph_columns
            .iter()
            .all(|column| column.splitter.width == 2)
    );

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
    assert!(app.dragging_graph_column.is_some());
    assert_eq!(app.settings.graph_changes_width, date_column.left_width);
    assert_eq!(app.settings.graph_date_width, date_column.right_width);
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        date_column.splitter.x,
        date_column.splitter.y,
    ));
    assert!(app.dragging_graph_column.is_none());
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
    assert_eq!(author_header.y, graph_search.y + 2);
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
    assert_eq!(
        app.regions
            .scroll_target_at(Position::new(second_author_row.x, second_author_row.y)),
        Some(ScrollTarget::AuthorFilter)
    );
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
    assert!(!app.graph_commit_open());
    click(&mut app, graph.x + 1, graph.y + 1);
    assert_eq!(app.graph_state.selected(), Some(1));
    assert!(app.graph_commit_open());
    wait_for_preview(&mut app);
    assert!(app.changes.preview.text().unwrap().contains("tracked.txt"));
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
    assert_eq!(app.view(), View::Graph);
    assert!(!app.graph_commit_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.graph_table.is_some());
    assert!(app.regions.diff_hunks.is_empty());
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.graph_state.selected(), Some(0));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.graph_commit_open());
    wait_for_preview(&mut app);
    assert!(app.changes.preview.text().unwrap().contains("second.txt"));
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
    let surroundings = app
        .regions
        .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::SurroundingsPane))
        .unwrap();
    assert_eq!(
        app.regions
            .scroll_target_at(Position::new(surroundings.x, surroundings.y)),
        Some(ScrollTarget::WorkspaceExplorerSurroundings)
    );
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
    assert_eq!(
        app.regions.scroll_target_at(Position::new(path.x, path.y)),
        Some(ScrollTarget::WorkspaceExplorer)
    );
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
    assert!(settings_screen.contains("Agent card click"));
    assert!(settings_screen.contains("Layout · Ctrl preview"));
    assert!(settings_screen.contains("Agent preview split"));
    assert!(settings_screen.contains("120 cols"));
    assert!(settings_screen.contains("Agent time"));
    assert!(settings_screen.contains("Latest loop"));
    assert!(settings_screen.contains("Agent timing history"));
    assert!(settings_screen.contains("Media protocol"));
    assert!(settings_screen.contains("Auto"));
    assert!(settings_screen.contains("Editor command"));
    assert!(!settings_screen.contains('┌'));
    let auto_fetch = app
        .regions
        .hit_target_rect(HitTarget::Settings(SettingsHitTarget::AutoFetch))
        .unwrap();
    let auto_switch_x = auto_fetch.right().saturating_sub(6);
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(auto_switch_x + 1, auto_fetch.y)].symbol(), "◼");
    assert!(
        (auto_switch_x..auto_switch_x + 5)
            .all(|x| buffer[(x, auto_fetch.y)].bg == super::palette().faint)
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Settings(SettingsHitTarget::FetchIntervalUp))
            .is_some()
    );
    let setting_rect = |target| {
        app.regions
            .hit_target_rect(HitTarget::Settings(target))
            .unwrap()
    };
    let format_on_save_setting = setting_rect(SettingsHitTarget::FormatOnSave);
    let cross_workspace_setting = setting_rect(SettingsHitTarget::CrossWorkspaceAgents);
    let agent_harness_setting = setting_rect(SettingsHitTarget::AgentHarness);
    let agent_card_click_setting = setting_rect(SettingsHitTarget::AgentCardClick);
    let agent_preview_split_setting = setting_rect(SettingsHitTarget::AgentPreviewSplit);
    let agent_time_setting = setting_rect(SettingsHitTarget::AgentTime);
    let clear_agent_timings_setting = setting_rect(SettingsHitTarget::ClearAgentTimings);
    let media_preview_setting = setting_rect(SettingsHitTarget::MediaPreview);
    let editor_setting = setting_rect(SettingsHitTarget::Editor);
    assert_eq!(cross_workspace_setting.y, format_on_save_setting.y + 4);
    assert_eq!(agent_harness_setting.y, cross_workspace_setting.y + 2);
    assert_eq!(agent_card_click_setting.y, agent_harness_setting.y + 2);
    assert_eq!(
        agent_preview_split_setting.y,
        agent_card_click_setting.y + 2
    );
    assert_eq!(agent_time_setting.y, agent_preview_split_setting.y + 2);
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

    app.settings_state.page = SettingsPage::OpenCode;
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
    let model_row = app
        .regions
        .hit_target_rect(HitTarget::Settings(SettingsHitTarget::OpenCodeModel))
        .unwrap();
    click(&mut app, model_row.x + 1, model_row.y);
    assert!(app.settings_state.opencode_model_input.is_some());
    app.settings_state.opencode_model_input = None;

    app.settings_state.page = SettingsPage::Discord;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let discord_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(discord_screen.contains("DISCORD WEBHOOKS"));
    assert!(discord_screen.contains("No webhooks configured"));
    let webhook_row = app
        .regions
        .hit_target_rect(HitTarget::Settings(SettingsHitTarget::DiscordWebhook))
        .unwrap();
    click(&mut app, webhook_row.x + 1, webhook_row.y);
    assert!(app.settings_state.discord_webhook_editor.is_some());
    app.settings_state
        .discord_webhook_editor
        .as_mut()
        .unwrap()
        .url
        .set("https://discord.com/api/webhooks/123456/token");
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let masked_screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!masked_screen.contains("discord.com"));
    assert!(!masked_screen.contains("token"));
    app.settings_state.discord_webhook_editor = None;

    app.settings_state.page = SettingsPage::Shortcuts;
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
    assert!(
        app.regions
            .scroll_state(&ScrollTarget::SettingsShortcuts)
            .is_some()
    );
    let explorer_row = app
        .regions
        .hit_target_rect(HitTarget::Settings(SettingsHitTarget::Shortcut(
            ShortcutAction::OpenExplorer,
        )))
        .unwrap();
    assert_eq!(
        app.regions
            .scroll_target_at(Position::new(explorer_row.x, explorer_row.y)),
        Some(ScrollTarget::SettingsShortcuts)
    );
    click(&mut app, explorer_row.x + 1, explorer_row.y);
    assert!(app.settings_state.shortcut_capture);
    app.settings_state.shortcut_capture = false;
    app.settings_state.page = SettingsPage::General;

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
        if app.changes.preview.text() != Some("Loading preview…")
            || app.changes.preview.image(false).is_some()
        {
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
        let preview = app.regions.diff.unwrap();
        if buffer.content.iter().enumerate().any(|(index, cell)| {
            let x = (index % width) as u16;
            let y = (index / width) as u16;
            cell.symbol() == "▀"
                && y != 2
                && x >= preview.x
                && x < preview.right()
                && y >= preview.y
                && y < preview.bottom()
        }) {
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
