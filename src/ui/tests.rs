use std::{fs, process::Command, thread, time::Duration};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::Position,
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    App, BrowserTab, ChangesHitTarget, CommitMessageGenerator, ExplorerHitTarget, ExplorerTab,
    GraphHitTarget, HitTarget, LeftPane, Mode, PullRequest, RemoteItems,
    RepositoryBrowserHitTarget, Settings, SettingsStore, SqliteFocus, View, WorkspaceDropTarget,
    WorkspacePanel, WorkspacePanelHitTarget, WorktreeManagerHitTarget, WorktreeManagerRow,
};
use crate::repo_path::RepoPath;

use super::draw;

fn assert_black_underlay(terminal: &Terminal<TestBackend>) {
    let background = &terminal.backend().buffer()[(0, 0)];
    assert_eq!(background.bg, Color::Rgb(0, 0, 0));
    assert!(background.modifier.contains(Modifier::DIM));
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
    app.commit_message_generator = CommitMessageGenerator::ready_for_test();
    assert_eq!(app.changes.history_state.selected(), None);
    let settings_path = root.join(".git/hunkle-test-config");
    app.settings_store = SettingsStore::at(settings_path.clone());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.worktree.unwrap().x, 0);
    assert_eq!(app.regions.worktree.unwrap().y, 1);
    assert_eq!(app.regions.diff.unwrap().right(), 120);
    assert!(app.regions.changes.is_none());
    assert_eq!(app.regions.graph.unwrap().y, 35);
    assert_eq!(app.regions.help.unwrap().y, 35);
    assert!(app.regions.graph.unwrap().x > 0);
    assert_eq!(app.regions.help.unwrap().right(), 120);
    let buffer = terminal.backend().buffer();
    let history = app.regions.history_splitter.unwrap();
    let history_offset = usize::from(history.y) * 120 + usize::from(history.x);
    assert_eq!(buffer.content[0].bg, super::palette().surface_alt);
    assert_eq!(
        buffer.content[36 * 120 - 1].bg,
        super::palette().surface_alt
    );
    assert_eq!(
        buffer.content[history_offset].bg,
        super::palette().surface_alt
    );
    let header: String = terminal.backend().buffer().content[..120]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(header.trim_end().ends_with("main"));
    let footer: String = terminal.backend().buffer().content[35 * 120..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(footer.contains("f Files"));
    assert!(footer.contains("e Edit"));
    assert!(footer.contains("Tab Git Graph"));
    assert!(!footer.contains("W Worktrees"));
    assert!(!footer.contains("b Branches"));
    assert!(!footer.contains("r Refresh"));
    assert!(!footer.contains("1 Changes"));
    assert!(!footer.contains("2 Graph"));
    for shortcut in ["Tab Git Graph", "f Files", "o Explorer"] {
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
    assert!(footer.contains("f Changes"));
    assert!(footer.contains("e Edit"));
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
    assert!(app.regions.history_list.is_none());
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
        .position(|row| row.file_path.is_some())
        .unwrap();
    let selected_file = explorer_rows[selected_file_row]
        .file_path
        .as_ref()
        .unwrap()
        .clone();
    click(
        &mut app,
        explorer.x + 2,
        explorer.y + selected_file_row as u16,
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
        explorer.y + 2,
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
    assert!(file_screen.contains("read-only"));
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
    let history_splitter = app.regions.history_splitter.unwrap();
    let commit = app.regions.commit.unwrap();
    let actions = app.regions.actions.unwrap();
    let worktree = app.regions.worktree_list.unwrap();
    assert_eq!(actions.y, commit.bottom());
    assert_eq!(actions.right(), commit.right());
    assert_eq!(actions.bottom(), worktree.y);
    assert!(commit.bottom() <= history_splitter.y);
    let history_bounds = app.regions.history_bounds.unwrap();
    let history_target = history_bounds.bottom().saturating_sub(9);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        history_splitter.right().saturating_sub(2),
        history_splitter.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        history_splitter.right().saturating_sub(2),
        history_target,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        history_splitter.right().saturating_sub(2),
        history_target,
    ));
    assert_eq!(app.settings.history_height, 9);
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains("history_height=9")
    );
    assert!(!app.dragging_history);

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let history = app.regions.history_list.unwrap();
    click(&mut app, history.x + 2, history.y + 2);
    wait_for_preview(&mut app);
    assert_eq!(app.changes.history_state.selected(), Some(1));
    assert!(app.changes.history_focused);
    assert!(app.changes.diff.contains("diff --git"));

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
    assert_eq!(app.changes.history_state.selected(), None);
    assert!(!app.changes.history_focused);
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
    assert!(changes_screen.contains("HISTORY"));
    assert!(changes_screen.contains("ACTIONS"));
    assert!(app.regions.actions.is_some());
    assert!(app.regions.actions.unwrap().bottom() <= app.regions.worktree_list.unwrap().y);
    assert!(changes_screen.contains("HEAD"));
    let history_oid: String = app.repository().unwrap().history[0]
        .oid
        .chars()
        .take(7)
        .collect();
    let history_date = app.repository().unwrap().history[0].date.clone();
    assert!(changes_screen.contains(&history_oid));
    assert!(!changes_screen.contains(&history_date));
    assert!(!changes_screen.contains("Render Test"));
    assert!(!changes_screen.contains('●'));
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
    let worktree = app.regions.worktree.unwrap();
    let mut graph = app.regions.graph_table.unwrap();
    assert!(graph.x >= worktree.right());
    assert!(app.regions.diff.is_none());

    let author_header = app
        .regions
        .hit_target_rect(HitTarget::Graph(GraphHitTarget::AuthorHeader))
        .unwrap();
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

    let branch_history = app.regions.history_list.unwrap();
    click(&mut app, branch_history.x + 1, branch_history.y);
    assert_eq!(app.changes.history_state.selected(), Some(0));
    assert!(app.graph_commit_open);
    wait_for_preview(&mut app);
    assert!(!app.changes.diff.is_empty());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.graph_table.is_none());
    assert!(app.regions.diff.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    graph = app.regions.graph_table.unwrap();

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
    assert!(commit_diff_screen.contains("DIFF"));
    assert!(commit_diff_screen.contains("initial commit"));
    assert!(commit_diff_screen.contains("Detailed body line."));
    assert!(commit_diff_screen.contains("Final note."));
    assert!(commit_diff_screen.contains("CHANGES"));
    assert!(commit_diff_screen.contains("FILES"));
    assert!(
        commit_diff_screen.contains("diff --git a/fixtures/file-00"),
        "the first file heading should be visible"
    );
    let commit_diff = app.regions.diff.unwrap();
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
    let scrolled_commit_diff: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        !scrolled_commit_diff.contains("MESSAGE"),
        "commit metadata should scroll with the patch"
    );
    assert!(scrolled_commit_diff.contains("CHANGES"));
    app.changes.diff_scroll = 0;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let mut wrapped_patch_file_summary = String::new();
    for row in files_row..files_row.saturating_add(6).min(commit_diff.bottom()) {
        for column in commit_diff.x..commit_diff.right() {
            wrapped_patch_file_summary
                .push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
    }
    assert!(wrapped_patch_file_summary.contains("fixtures/file-05.txt"));
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
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
    assert!(explorer_screen.contains("F1  EXPLORER"));
    assert!(explorer_screen.contains("F2  WORKTREES"));
    assert!(explorer_screen.contains("F3  BRANCHES"));
    assert!(explorer_screen.contains("Switch working directory"));
    assert!(explorer_screen.contains("AROUND HERE"));
    assert!(explorer_screen.contains("CONTENTS"));
    assert!(explorer_screen.contains("★ Project"));
    assert!(!explorer_screen.contains("OPEN REPOSITORY"));
    assert!(!explorer_screen.contains('┌'));
    let worktrees_tab = app
        .regions
        .hit_target_rect(HitTarget::ExplorerTab(ExplorerTab::Worktrees))
        .unwrap();
    click(&mut app, worktrees_tab.x + 1, worktrees_tab.y);
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Worktrees);
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(app.explorer_tab, ExplorerTab::Explorer);
    let branches_tab = app
        .regions
        .hit_target_rect(HitTarget::ExplorerTab(ExplorerTab::Branches))
        .unwrap();
    click(&mut app, branches_tab.x + 1, branches_tab.y);
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Branches);
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(app.explorer_tab, ExplorerTab::Explorer);
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

    app.mode = Mode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Branches);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let browser_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(browser_screen.contains("PULL REQUESTS"));
    assert!(browser_screen.contains("LOCAL & REMOTE"));
    assert!(browser_screen.contains("F3  BRANCHES"));
    assert!(browser_screen.contains("Navigate repository work"));
    assert!(browser_screen.contains("FILTER  TYPE TO SEARCH"));
    assert!(browser_screen.contains("BRANCH DETAILS"));
    assert!(browser_screen.contains("LATEST COMMIT"));
    assert!(browser_screen.contains("CURRENT"));
    assert!(browser_screen.contains("main"));
    assert!(
        app.regions
            .hit_target_rect(HitTarget::RepositoryBrowser(
                RepositoryBrowserHitTarget::List,
            ))
            .is_some()
    );
    let browser_overlay = app
        .regions
        .hit_target_rect(HitTarget::RepositoryBrowser(
            RepositoryBrowserHitTarget::Overlay,
        ))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let background = &buffer[(0, 0)];
    let modal = &buffer[(browser_overlay.x, browser_overlay.y)];
    assert_eq!(background.bg, Color::Rgb(0, 0, 0));
    assert!(background.modifier.contains(Modifier::DIM));
    assert_eq!(modal.bg, super::palette().surface_alt);
    assert!(!modal.modifier.contains(Modifier::DIM));

    let target_oid = app.repository().unwrap().commits[1].oid.clone();
    let mut target_branch = app.repository_browser.branches[0].clone();
    target_branch.name = "test/hover-target".to_owned();
    target_branch.oid = target_oid;
    target_branch.current = false;
    app.repository_browser.branches.push(target_branch);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app
        .regions
        .hit_target_rect(HitTarget::RepositoryBrowser(
            RepositoryBrowserHitTarget::List,
        ))
        .unwrap();
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: list.x + 4,
        row: list.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.repository_browser.state.selected(), Some(1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(list.x + 4, list.y + 1)].bg,
        super::palette().selected
    );
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(app.repository_browser.branch_delete_open());
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(app.explorer_tab, ExplorerTab::Branches);
    assert!(app.repository_browser.branch_delete_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let delete_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(delete_screen.contains("DELETE BRANCH"));
    assert!(delete_screen.contains("Delete local branch test/hover-target?"));
    assert!(delete_screen.contains("Local only"));
    assert!(delete_screen.contains("Force local"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.repository_browser.branch_delete_open());
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, View::Graph);
    assert_eq!(app.graph_state.selected(), Some(1));
    assert_eq!(app.graph_state.offset(), 0);
    assert!(!app.graph_scroll_to_selection);

    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app
        .regions
        .hit_target_rect(HitTarget::RepositoryBrowser(
            RepositoryBrowserHitTarget::List,
        ))
        .unwrap();
    let branch_oid = app.repository_browser.branches[0].oid.clone();
    let branch_tip = app
        .repository()
        .unwrap()
        .commits
        .iter()
        .position(|commit| commit.oid.starts_with(&branch_oid))
        .unwrap();
    app.graph_state
        .select(Some(usize::from(branch_tip == 0).min(1)));
    click(&mut app, list.x + 4, list.y);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, View::Graph);
    assert_eq!(app.graph_state.selected(), Some(branch_tip));
    assert_eq!(app.graph_state.offset(), branch_tip.saturating_sub(5));

    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));

    app.repository_browser.pull_requests = RemoteItems::ready(vec![
        PullRequest {
            number: 42,
            title: "Improve browser readability".to_owned(),
            branch: "feature/browser".to_owned(),
            author: "octocat".to_owned(),
            draft: true,
        },
        PullRequest {
            number: 43,
            title: "Polish metadata colors".to_owned(),
            branch: "feature/colors".to_owned(),
            author: "hubot".to_owned(),
            draft: false,
        },
    ]);
    app.repository_browser.set_tab(BrowserTab::PullRequests);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let pull_request_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(pull_request_screen.contains("PULL REQUEST  selected"));
    assert!(pull_request_screen.contains("HEAD BRANCH"));
    assert!(pull_request_screen.contains("feature/browser"));
    let list = app
        .regions
        .hit_target_rect(HitTarget::RepositoryBrowser(
            RepositoryBrowserHitTarget::List,
        ))
        .unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(list.x + 4, list.y + 1)].fg, super::palette().ink);
    assert_eq!(buffer[(list.x + 4, list.y + 3)].fg, super::palette().cyan);
    click(&mut app, list.x + 4, list.y + 2);
    assert_eq!(app.repository_browser.state.selected(), Some(1));
    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(app.repository_browser.query, "m");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);

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
    assert!(settings_screen.contains("Workspace pane"));
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
    let workspace_setting = app.regions.workspace_panel_setting.unwrap();
    let agent_harness_setting = app.regions.agent_harness_setting.unwrap();
    let agent_time_setting = app.regions.agent_time_setting.unwrap();
    let clear_agent_timings_setting = app.regions.clear_agent_timings_setting.unwrap();
    let media_preview_setting = app.regions.media_preview_setting.unwrap();
    let editor_setting = app.regions.editor_setting.unwrap();
    assert_eq!(agent_harness_setting.y, workspace_setting.y + 2);
    assert_eq!(agent_time_setting.y, agent_harness_setting.y + 2);
    assert_eq!(clear_agent_timings_setting.y, agent_time_setting.y + 2);
    assert_eq!(media_preview_setting.y, clear_agent_timings_setting.y + 2);
    assert_eq!(editor_setting.y, media_preview_setting.y + 2);
    let switch_x = workspace_setting.right().saturating_sub(6);
    assert_eq!(buffer[(switch_x + 3, workspace_setting.y)].symbol(), "◼");
    assert!(
        (switch_x..switch_x + 5)
            .all(|x| buffer[(x, workspace_setting.y)].bg == super::palette().green)
    );
    let harness_switch_x = agent_harness_setting.right().saturating_sub(6);
    assert_eq!(
        buffer[(harness_switch_x + 1, agent_harness_setting.y)].symbol(),
        "◼"
    );
    assert!(
        (harness_switch_x..harness_switch_x + 5)
            .all(|x| buffer[(x, agent_harness_setting.y)].bg == super::palette().faint)
    );

    app.settings.workspace_panel_enabled = false;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(switch_x + 1, workspace_setting.y)].symbol(), "◼");
    assert!(
        (switch_x..switch_x + 5)
            .all(|x| buffer[(x, workspace_setting.y)].bg == super::palette().faint)
    );

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

#[test]
fn renders_herdr_workspaces_and_agents_as_an_app_level_rail() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    let settings_path = directory.path().join("config");
    app.settings_store = SettingsStore::at(settings_path.clone());
    app.settings.workspace_panel_width = 26;
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
                "agents": [
                    {
                        "agent": "opencode",
                        "agent_status": "working",
                        "focused": true,
                        "pane_id": "w1:p1",
                        "tab_id": "w1:t1",
                        "terminal_title_stripped": "OC | Refine workspace timers",
                        "workspace_id": "w1"
                    },
                    {
                        "agent": "opencode",
                        "agent_status": "idle",
                        "focused": false,
                        "pane_id": "w1:p2",
                        "tab_id": "w1:t1",
                        "terminal_title_stripped": "OC | Review panel spacing",
                        "workspace_id": "w1"
                    }
                ]
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(directory.path().to_path_buf());
    app.workspace_panel.workspaces[0].branch = Some("topic".to_owned());
    app.workspace_panel.loading = true;
    app.mode = Mode::WorkspacePanel;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    assert_eq!(app.regions.workspace_panel.unwrap().width, 26);
    assert_eq!(app.regions.worktree.unwrap().x, 27);
    let panel_area = app.regions.workspace_panel.unwrap();
    let workspace_section = app.regions.workspace_panel_workspaces.unwrap();
    let agent_section = app.regions.workspace_panel_agents.unwrap();
    assert_eq!(agent_section.y, panel_area.y + panel_area.height / 2);
    assert_eq!(workspace_section.height, agent_section.height);
    assert_eq!(
        app.regions
            .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Collapse,))
            .unwrap()
            .y,
        app.regions.worktree.unwrap().y + 1
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::WorkspacePanel(
                WorkspacePanelHitTarget::Workspace(0)
            ))
            .is_some()
    );
    assert!(
        app.regions
            .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(0)))
            .is_some()
    );
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("WORKSPACES  +"));
    assert!(rendered.contains("Load"));
    assert!(rendered.contains("HUNKLE"));
    assert!(rendered.contains("topic"));
    assert!(rendered.contains("AGENTS"));
    assert!(!rendered.contains("opencode"));
    assert!(!rendered.contains('↻'));
    app.settings.show_agent_harness = true;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let rendered_with_harness: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered_with_harness.contains("0:00 opencode /"));
    assert!(rendered_with_harness.contains("Refine workspace"));
    assert!(rendered_with_harness.contains("Review panel spacing"));
    let workspace_row = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::Workspace(0),
        ))
        .unwrap();
    let workspace_status =
        &terminal.backend().buffer()[(workspace_row.right() - 1, workspace_row.y)];
    assert_eq!(workspace_status.symbol(), "⠋");
    assert_eq!(workspace_status.fg, super::palette().yellow);
    let agent_row = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(0)))
        .unwrap();
    assert_eq!(agent_row.height, 2);
    assert!(
        (agent_row.x..agent_row.right())
            .all(|x| terminal.backend().buffer()[(x, agent_row.y - 1)].symbol() == " ")
    );
    let agent_timer = &terminal.backend().buffer()[(agent_row.x + 2, agent_row.y)];
    assert_eq!(agent_timer.symbol(), "0");
    assert_eq!(agent_timer.bg, super::palette().inactive_selected);
    let agent_status = &terminal.backend().buffer()[(agent_row.right() - 1, agent_row.y)];
    assert_eq!(agent_status.symbol(), "⠋");
    assert_eq!(agent_status.fg, super::palette().yellow);
    let agent_session = &terminal.backend().buffer()[(agent_row.x + 2, agent_row.y + 1)];
    assert_eq!(agent_session.symbol(), "R");
    assert_eq!(agent_session.fg, super::palette().yellow);
    assert_eq!(agent_session.bg, super::palette().inactive_selected);
    let second_agent_row = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(1)))
        .unwrap();
    assert_eq!(second_agent_row.y, agent_row.bottom() + 1);
    let branch_cell = &terminal.backend().buffer()[(workspace_row.right() - 7, workspace_row.y)];
    assert_eq!(branch_cell.symbol(), "t");
    assert_eq!(branch_cell.fg, super::palette().accent);
    assert_eq!(branch_cell.bg, super::palette().selected);
    let workspace_marker = &terminal.backend().buffer()[(workspace_row.x, workspace_row.y)];
    assert_eq!(workspace_marker.symbol(), "•");
    assert_eq!(workspace_marker.fg, super::palette().yellow);
    let workspace_label = &terminal.backend().buffer()[(workspace_row.x + 2, workspace_row.y)];
    assert_eq!(workspace_label.symbol(), "H");
    assert_eq!(workspace_label.fg, super::palette().yellow);
    assert!(workspace_label.modifier.contains(Modifier::BOLD));
    let create_button = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::CreateMenu,
        ))
        .unwrap();
    assert_eq!(create_button.width, 3);
    assert_eq!(create_button.x, app.regions.workspace_panel.unwrap().x + 12);
    let load_button = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::SnapshotMenu,
        ))
        .unwrap();
    assert_eq!(load_button.width, 6);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        load_button.x + 1,
        load_button.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        load_button.x + 1,
        load_button.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.mode, Mode::WorkspacePresets);
    let preset_overlay = app.regions.workspace_presets_overlay.unwrap();
    assert_eq!(preset_overlay.height, 8);
    assert_eq!(preset_overlay.width, 50);
    assert!(
        app.regions
            .hit_target_rect(HitTarget::WorkspacePanel(
                WorkspacePanelHitTarget::SaveSnapshot,
            ))
            .is_some()
    );
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("WORKSPACE PRESETS"));
    assert!(rendered.contains("Create preset from current setup"));
    let footer_start = 29 * 120;
    let footer: String = terminal.backend().buffer().content[footer_start..]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    for shortcut in ["f Changes", "w Workspaces", "o Explorer"] {
        let offset = footer
            .find(shortcut)
            .unwrap_or_else(|| panic!("missing {shortcut:?} in footer {footer:?}"));
        assert_eq!(
            terminal.backend().buffer().content[footer_start + offset].fg,
            super::palette().orange
        );
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    app.handle_paste("Daily setup");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.workspace_panel.snapshots.len(), 1);
    assert_eq!(app.notice.as_deref(), Some("Preset saved: Daily setup"));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    assert_eq!(app.notice.as_deref(), Some("Preset updated: Daily setup"));
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(app.workspace_panel.snapshots.is_empty());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::WorkspacePresets);
    assert!(app.workspace_panel.snapshot_load_dialog.is_none());
    let mut narrow = Terminal::new(TestBackend::new(60, 16)).unwrap();
    narrow.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.mode, Mode::WorkspacePresets);
    assert!(app.regions.workspace_presets_overlay.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.mode = Mode::WorkspacePanel;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(create_button.x + 1, create_button.y)].bg,
        super::palette().raised
    );
    app.handle_mouse(mouse(
        MouseEventKind::Moved,
        create_button.x + 1,
        create_button.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(create_button.x + 1, create_button.y)].bg,
        super::palette().accent
    );
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        create_button.x + 1,
        create_button.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        create_button.x + 1,
        create_button.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.workspace_panel.create_menu_open);
    let new_workspace = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::CreateWorkspace,
        ))
        .unwrap();
    let new_worktree = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(
            WorkspacePanelHitTarget::CreateWorktree,
        ))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("New workspace"));
    assert!(rendered.contains("New worktree"));
    assert_eq!(
        terminal.backend().buffer()[(new_workspace.x + 2, new_workspace.y)].bg,
        super::palette().selected
    );
    assert_eq!(
        terminal.backend().buffer()[(new_worktree.x + 2, new_worktree.y)].fg,
        super::palette().ink
    );
    assert!(app.workspace_panel.select_agent(0));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(new_worktree.x + 2, new_worktree.y)].fg,
        super::palette().faint
    );
    app.workspace_panel.close_create_menu();
    assert!(app.workspace_panel.select_workspace(0));
    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let delete_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(delete_screen.contains("CLOSE WORKSPACE"));
    assert!(delete_screen.contains("Close workspace HUNKLE?"));
    assert!(delete_screen.contains("all 2 panes"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.workspace_panel.delete_dialog.is_none());

    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let rename_screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rename_screen.contains("RENAME WORKSPACE"));
    assert!(rename_screen.contains("Rename HUNKLE"));
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::WorkspacePanel);
    assert_eq!(
        app.workspace_panel
            .rename_dialog
            .as_ref()
            .map(|dialog| dialog.input.text()),
        Some("p")
    );
    let agent_row = app
        .regions
        .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(0)))
        .unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        agent_row.x,
        agent_row.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        agent_row.x,
        agent_row.y,
    ));
    assert_eq!(app.workspace_panel.selected, Some(0));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.workspace_panel.rename_dialog.is_none());
    app.mode = Mode::Normal;

    let divider = app.regions.workspace_panel_splitter.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        divider.x,
        divider.y + 2,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        35,
        divider.y + 2,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        35,
        divider.y + 2,
    ));
    assert_eq!(app.settings.workspace_panel_width, 35);
    assert!(!app.dragging_workspace_panel_splitter);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.workspace_panel.unwrap().width, 35);
    assert_eq!(app.regions.worktree.unwrap().x, 36);

    app.workspace_panel.begin_group();
    app.workspace_panel.paste("Current work");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.workspace_panel.begin_workspace_drag(0));
    app.workspace_panel
        .update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
    app.workspace_panel.finish_workspace_drag();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Group(0)))
            .is_some()
    );
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(rendered.matches("Current work").count(), 1);

    app.workspace_panel.cycle_placement();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.workspace_panel.unwrap().x, 85);
    assert_eq!(app.regions.worktree.unwrap().x, 0);

    let divider = app.regions.workspace_panel_splitter.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        divider.x,
        divider.y + 2,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        78,
        divider.y + 2,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        78,
        divider.y + 2,
    ));
    assert_eq!(app.settings.workspace_panel_width, 41);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.regions.workspace_panel.unwrap().x, 79);
    assert_eq!(app.regions.workspace_panel.unwrap().width, 41);
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains("workspace_panel_width=41")
    );

    app.workspace_panel.cycle_placement();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.workspace_panel.is_none());
    assert_eq!(app.regions.worktree.unwrap().x, 0);

    app.workspace_panel.show_left();

    let mut narrow = Terminal::new(TestBackend::new(78, 24)).unwrap();
    narrow.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.workspace_panel.is_none());
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(
        app.notice.as_deref(),
        Some("Workspaces need a wider terminal")
    );

    app.settings.workspace_panel_enabled = false;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.workspace_panel.is_none());
    let placement = app.workspace_panel.placement;
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    assert_eq!(app.workspace_panel.placement, placement);

    app.settings.workspace_panel_enabled = true;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(app.regions.workspace_panel.is_some());
}

#[test]
fn distinguishes_the_active_herdr_workspace_from_the_loaded_workspace() {
    let loaded = tempfile::tempdir().unwrap();
    let active = tempfile::tempdir().unwrap();
    let mut app = App::new(loaded.path().to_path_buf());
    app.settings.workspace_panel_width = 26;
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
    let active_marker = &buffer[(active_row.x, active_row.y)];
    let active_label = &buffer[(active_row.x + 2, active_row.y)];
    let loaded_marker = &buffer[(loaded_row.x, loaded_row.y)];
    let loaded_label = &buffer[(loaded_row.x + 2, loaded_row.y)];

    assert_eq!(active_marker.symbol(), " ");
    assert_eq!(active_label.fg, super::palette().yellow);
    assert!(active_label.modifier.contains(Modifier::BOLD));
    assert_eq!(loaded_marker.symbol(), "•");
    assert_eq!(loaded_marker.fg, super::palette().yellow);
    assert_eq!(loaded_label.fg, super::palette().muted);
    assert!(!loaded_label.modifier.contains(Modifier::BOLD));
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
fn clicking_empty_worktree_keeps_the_open_history_diff() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "History Diff Test"]);
    run_git(root, &["config", "user.email", "history-diff@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.changes
            .worktree_rows(app.repository().unwrap())
            .is_empty()
    );

    let history = app.regions.history_list.unwrap();
    click(&mut app, history.x + 1, history.y);
    wait_for_preview(&mut app);
    let history_diff = app.changes.diff.clone();
    assert!(app.changes.history_focused);
    assert!(history_diff.contains("diff --git"));

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let worktree = app.regions.worktree_list.unwrap();
    click(&mut app, worktree.x + 1, worktree.y + 1);

    assert!(app.changes.history_focused);
    assert_eq!(app.changes.history_state.selected(), Some(0));
    assert_eq!(app.changes.diff, history_diff);
}

#[test]
fn right_clicking_worktree_rows_toggles_staging() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Right Click Test"]);
    run_git(root, &["config", "user.email", "right-click@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    for staged in [true, false] {
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let repo = app.repository().unwrap();
        let row = app
            .changes
            .worktree_rows(repo)
            .iter()
            .position(|row| {
                row.change_index
                    .and_then(|index| repo.changes.get(index))
                    .is_some_and(|change| change.path == "tracked.txt")
            })
            .unwrap();
        let target = app.changes.worktree_row_target(row);
        let rect = app
            .regions
            .hit_target_rect(HitTarget::Changes(target))
            .unwrap();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Right),
            rect.x,
            rect.y,
        ));
        wait_for(&mut app, |app| {
            app.repository().is_some_and(|repo| {
                repo.changes
                    .iter()
                    .find(|change| change.path == "tracked.txt")
                    .is_some_and(|change| change.staged == staged)
            })
        });
    }
}

#[test]
fn graph_replacement_clears_hidden_diff_targets() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Graph Target Test"]);
    run_git(root, &["config", "user.email", "graph-target@example.com"]);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    wait_for_preview(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let target = app.changes.hunk_action_target(0);
    let action = app
        .regions
        .hit_target_rect(HitTarget::Changes(target))
        .unwrap();

    app.view = View::Graph;
    app.graph_commit_open = false;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_rect(HitTarget::Changes(target))
            .is_none()
    );
    click(&mut app, action.x, action.y);
    for _ in 0..20 {
        let _ = app.poll_worker();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        app.repository()
            .unwrap()
            .changes
            .iter()
            .all(|change| !change.staged)
    );
}

#[test]
fn renders_colored_file_type_icons_in_the_files_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for path in [
        "main.rs",
        "app.ts",
        "config.json",
        "README.md",
        "styles.css",
        "run.sh",
        "asset.png",
        "notes.xyz",
        "readme_parser.rs",
    ] {
        fs::write(root.join(path), "fixture\n").unwrap();
    }

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    for (path, symbol, color) in [
        ("main.rs", "R", super::palette().orange),
        ("app.ts", "T", super::palette().cyan),
        ("config.json", "{", super::palette().yellow),
        ("README.md", "#", super::palette().cyan),
        ("styles.css", "#", super::palette().purple),
        ("run.sh", ">", super::palette().green),
        ("asset.png", "@", super::palette().purple),
        ("notes.xyz", "?", super::palette().faint),
        ("readme_parser.rs", "R", super::palette().orange),
    ] {
        let row_index = rows
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|file| file == path))
            .unwrap();
        let row = &rows[row_index];
        let x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        let icon = &terminal.backend().buffer()[(x, y)];
        assert_eq!(icon.symbol(), symbol, "{path}");
        assert_eq!(icon.fg, color, "{path}");
    }
}

#[test]
fn keeps_file_tree_connectors_faint_and_folder_names_bright() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("src/nested/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("root.txt"), "root\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let src = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src")
        })
        .unwrap();
    app.changes.explorer_state.select(Some(src));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.explorer_rows().iter().any(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src/nested")
        })
    });
    let nested = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src/nested")
        })
        .unwrap();
    app.changes.explorer_state.select(Some(nested));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.explorer_rows().iter().any(|row| {
            row.file_path
                .as_ref()
                .is_some_and(|path| path == "src/nested/main.rs")
        })
    });

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();

    let src = rows
        .iter()
        .position(|row| {
            row.directory_path
                .as_ref()
                .is_some_and(|path| path == "src")
        })
        .unwrap();
    let src_y = list.y + src.saturating_sub(app.changes.explorer_scroll) as u16;
    assert_eq!(
        terminal.backend().buffer()[(list.x + 2, src_y)].fg,
        super::palette().ink
    );

    for path in ["root.txt", "src/nested/main.rs"] {
        let row_index = rows
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|file| file == path))
            .unwrap();
        let row = &rows[row_index];
        let x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16 + 2;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        assert_eq!(
            terminal.backend().buffer()[(x, y)].fg,
            super::palette().soft,
            "{path}"
        );
    }

    let mut saw_connector = false;
    for (row_index, row) in rows.iter().enumerate() {
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        for (offset, character) in row.prefix.chars().enumerate() {
            if !character.is_whitespace() {
                saw_connector = true;
                assert_eq!(
                    terminal.backend().buffer()[(list.x + offset as u16, y)].fg,
                    super::palette().faint
                );
            }
        }
    }
    assert!(saw_connector);
}

#[test]
fn colors_changed_files_in_the_files_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("modified.txt"), "original\n").unwrap();
    fs::write(root.join("deleted.txt"), "deleted\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    fs::write(root.join("modified.txt"), "changed\n").unwrap();
    fs::write(root.join("added.txt"), "added\n").unwrap();
    run_git(root, &["add", "added.txt"]);
    fs::write(root.join("new.txt"), "new\n").unwrap();
    run_git(root, &["rm", "deleted.txt"]);
    fs::write(root.join("deleted.txt"), "replacement\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.changes.pane = LeftPane::Files;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    for (path, expected) in [
        ("added.txt", super::palette().accent),
        ("deleted.txt", super::palette().red),
        ("modified.txt", super::palette().yellow),
        ("new.txt", super::palette().green),
    ] {
        let row_index = rows
            .iter()
            .position(|row| row.file_path.as_ref().is_some_and(|file| file == path))
            .unwrap();
        let row = &rows[row_index];
        let x = list.x + row.prefix.chars().count() as u16 + 2;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        assert_eq!(terminal.backend().buffer()[(x, y)].fg, expected, "{path}");
    }
}

#[test]
fn shows_worktree_file_status_letters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("modified.txt"), "original\n").unwrap();
    fs::write(root.join("deleted.txt"), "deleted\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    fs::write(root.join("modified.txt"), "changed\n").unwrap();
    fs::remove_file(root.join("deleted.txt")).unwrap();
    fs::write(root.join("new.txt"), "new\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.worktree_list.unwrap();
    let rows = app.changes.worktree_rows(app.repository().unwrap());
    for (path, status, color) in [
        ("deleted.txt", 'D', super::palette().red),
        ("modified.txt", 'M', super::palette().yellow),
        ("new.txt", 'U', super::palette().green),
    ] {
        let row_index = rows.iter().position(|row| row.label == path).unwrap();
        let row = &rows[row_index];
        let y = list.y + row_index.saturating_sub(app.changes.worktree_scroll) as u16;
        let status_x = list.x + UnicodeWidthStr::width(row.prefix.as_str()) as u16;
        assert_eq!(
            terminal.backend().buffer()[(status_x, y)].symbol(),
            status.to_string()
        );
        assert_eq!(terminal.backend().buffer()[(status_x, y)].fg, color);
    }
}

#[test]
fn colors_collapsed_folders_for_the_changes_they_contain() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Render Test"]);
    run_git(root, &["config", "user.email", "render@example.com"]);
    for path in ["modified/file.txt", "deleted/file.txt", "deleted/keep.txt"] {
        fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        fs::write(root.join(path), "original\n").unwrap();
    }
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);

    fs::write(root.join("modified/file.txt"), "changed\n").unwrap();
    fs::create_dir_all(root.join("added")).unwrap();
    fs::write(root.join("added/file.txt"), "added\n").unwrap();
    run_git(root, &["add", "added/file.txt"]);
    fs::create_dir_all(root.join("untracked")).unwrap();
    fs::write(root.join("untracked/file.txt"), "new\n").unwrap();
    run_git(root, &["rm", "deleted/file.txt"]);

    let mut app = App::new(root.to_path_buf());
    app.changes.pane = LeftPane::Files;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let list = app.regions.explorer_list.unwrap();
    let rows = app.changes.explorer_rows();
    for (path, expected) in [
        ("added", super::palette().accent),
        ("deleted", super::palette().red),
        ("modified", super::palette().yellow),
        ("untracked", super::palette().green),
    ] {
        let row_index = rows
            .iter()
            .position(|row| {
                row.directory_path
                    .as_ref()
                    .is_some_and(|directory| directory == path)
            })
            .unwrap();
        assert_eq!(rows[row_index].directory_expanded, Some(false));
        let x = list.x + rows[row_index].prefix.chars().count() as u16;
        let y = list.y + row_index.saturating_sub(app.changes.explorer_scroll) as u16;
        assert_eq!(terminal.backend().buffer()[(x, y)].fg, expected, "{path}");
    }
}

#[test]
fn files_click_waits_for_release_without_styling_every_file_as_a_drop_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("first.txt"), "first\n").unwrap();
    fs::write(root.join("second.txt"), "second\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app.regions.explorer_list.unwrap();
    let selected_before = app.changes.explorer_state.selected();
    let target = app
        .changes
        .explorer_rows()
        .iter()
        .enumerate()
        .find_map(|(index, row)| {
            (row.file_path.is_some() && Some(index) != selected_before).then_some(index)
        })
        .unwrap();
    let y = list.y + target.saturating_sub(app.changes.explorer_scroll) as u16;

    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        list.x + 1,
        y,
    ));
    assert_eq!(app.changes.explorer_state.selected(), selected_before);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    for (index, row) in app.changes.explorer_rows().iter().enumerate() {
        if row.file_path.is_some() && Some(index) != selected_before {
            let y = list.y + index.saturating_sub(app.changes.explorer_scroll) as u16;
            assert_ne!(
                terminal.backend().buffer()[(list.x, y)].bg,
                super::palette().inactive_selected
            );
        }
    }

    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), list.x + 1, y));
    assert_eq!(app.changes.explorer_state.selected(), Some(target));
}

#[test]
fn opens_plain_directories_as_file_workspaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::create_dir_all(root.join("config/nested")).unwrap();
    fs::write(root.join("README.md"), "local workspace\n").unwrap();
    fs::write(root.join("config/nested/settings.toml"), "theme = 'test'\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.repository().unwrap().is_local());
    assert_eq!(app.changes.pane, LeftPane::Files);
    wait_for_preview(&mut app);
    assert_eq!(app.changes.diff, "local workspace\n");

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("CHANGES"));
    assert!(screen.contains("FILES"));
    assert!(screen.contains("README.md"));
    assert!(screen.contains("local workspace"));

    let add = app.regions.files_add.unwrap();
    click(&mut app, add.x, add.y);
    assert_eq!(app.mode, Mode::Files);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let popover = app.regions.file_dialog_overlay.unwrap();
    assert_eq!(popover.y, add.bottom());
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!screen.contains("ADD TO FILES"));
    assert!(screen.contains("New file"));
    assert!(screen.contains("New folder"));
    let new_file = app.regions.file_dialog_primary.unwrap();
    click(&mut app, new_file.x, new_file.y);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("NEW FILE"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    let worktree_tab = app
        .regions
        .hit_target_rect(HitTarget::Changes(ChangesHitTarget::WorktreeTab))
        .unwrap();
    click(&mut app, worktree_tab.x, worktree_tab.y);
    assert_eq!(app.changes.pane, LeftPane::Worktree);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Working tree clean"));
    assert!(screen.contains("LOCAL WORKSPACE"));
    assert!(screen.contains("Local file workspace"));
}

#[test]
fn fuzzy_searches_and_opens_repository_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::create_dir_all(root.join("src/components")).unwrap();
    fs::write(
        root.join("src/components/profile_card.rs"),
        "pub struct ProfileCard;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/components/button.rs"),
        "pub struct Button;\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "search fixture\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::FileSearch);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.view = View::Graph;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    for character in "profile card".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(app.mode, Mode::FileSearch);
    assert_eq!(app.file_search.match_count, 1);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);

    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("FIND FILE"));
    assert!(screen.contains("profile_card.rs"));
    assert!(screen.contains("src/components"));
    assert!(app.regions.file_search_overlay.is_some());
    assert!(app.regions.file_search_list.is_some());

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for_preview(&mut app);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, View::Changes);
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(
        app.selected_explorer_file_path().map(RepoPath::display),
        Some("src/components/profile_card.rs".to_owned())
    );
    assert_eq!(app.changes.diff, "pub struct ProfileCard;\n");
    assert!(
        app.changes
            .explorer_rows()
            .iter()
            .any(|row| row.label == "profile_card.rs")
    );
}

#[test]
fn left_pane_files_take_over_the_preview_from_graph() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Graph Test"]);
    run_git(root, &["config", "user.email", "graph@example.com"]);
    fs::write(root.join("tracked.txt"), "first\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("tracked.txt"), "changed\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    app.view = View::Graph;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let worktree = app.regions.worktree_list.unwrap();
    let file_row = app
        .changes
        .worktree_rows(app.repository().unwrap())
        .iter()
        .position(|row| row.change_index.is_some())
        .unwrap();
    let stage_target = app.changes.worktree_stage_target(file_row);
    let status = app
        .regions
        .hit_target_rect(HitTarget::Changes(stage_target))
        .unwrap();
    let file_y = worktree.y + (file_row - app.changes.worktree_scroll) as u16;
    click(&mut app, status.x, file_y);
    assert_eq!(app.view, View::Graph, "staging should not close Graph");

    click(&mut app, worktree.x + 3, file_y);
    assert_eq!(app.view, View::Changes);
    assert!(!app.graph_commit_open);

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.changes.pane, LeftPane::Files);
    app.view = View::Graph;
    app.graph_commit_open = true;
    app.mode = Mode::WorkspacePanel;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let explorer = app.regions.explorer_list.unwrap();
    let file_row = app
        .changes
        .explorer_rows()
        .iter()
        .position(|row| row.file_path.is_some())
        .unwrap();
    click(&mut app, explorer.x + 3, explorer.y + file_row as u16);
    wait_for_preview(&mut app);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.view, View::Changes);
    assert!(!app.graph_commit_open);
    assert_eq!(app.changes.diff, "changed\n");
}

#[test]
fn double_clicking_worktree_files_opens_them_in_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.name", "Double Click Test"]);
    run_git(root, &["config", "user.email", "double-click@example.com"]);
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/unstaged.txt"), "initial\n").unwrap();
    fs::write(root.join("staged.txt"), "initial\n").unwrap();
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial commit"]);
    fs::write(root.join("nested/unstaged.txt"), "unstaged content\n").unwrap();
    fs::write(root.join("staged.txt"), "staged content\n").unwrap();
    run_git(root, &["add", "staged.txt"]);

    let mut app = App::new(root.to_path_buf());
    let mut terminal = Terminal::new(TestBackend::new(100, 50)).unwrap();

    for (path, content) in [
        ("nested/unstaged.txt", "unstaged content\n"),
        ("staged.txt", "staged content\n"),
    ] {
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let worktree = app.regions.worktree_list.unwrap();
        let repo = app.repository().unwrap();
        let row = app
            .changes
            .worktree_rows(repo)
            .iter()
            .position(|row| {
                row.change_index
                    .and_then(|index| repo.changes.get(index))
                    .is_some_and(|change| change.path == path)
            })
            .unwrap();
        let y = worktree.y + (row - app.changes.worktree_scroll) as u16;
        assert!(worktree.contains(Position::new(worktree.x + 4, y)));

        click(&mut app, worktree.x + 4, y);
        assert_eq!(app.changes.pane, LeftPane::Worktree);
        click(&mut app, worktree.x + 4, y);
        wait_for_preview(&mut app);

        assert_eq!(app.changes.pane, LeftPane::Files);
        assert_eq!(app.view, View::Changes);
        assert_eq!(
            app.selected_explorer_file_path().map(RepoPath::display),
            Some(path.to_owned())
        );
        assert_eq!(app.changes.diff, content);

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.changes.pane, LeftPane::Worktree);
    }
}

#[test]
fn renders_markdown_files_and_toggles_back_to_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    let mut markdown = "# Markdown Title\n\nA **strong** [link](https://example.com).\n".to_owned();
    for section in 1..=40 {
        markdown.push_str(&format!(
            "\n## Section {section}\n\nParagraph {section} has enough content to remain visible.\n"
        ));
    }
    fs::write(root.join("README.md"), markdown).unwrap();

    let mut app = App::new(root.to_path_buf());
    app.settings.worktree_width = 48;
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    wait_for_preview(&mut app);
    assert_eq!(app.changes.pane, LeftPane::Files);
    assert_eq!(
        app.selected_explorer_file_path().map(RepoPath::display),
        Some("README.md".to_owned())
    );

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let preview_button = (0..30)
        .find_map(|row| {
            let text: String = (0..100)
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            text.find("Preview").map(|column| (column as u16 + 1, row))
        })
        .expect("Markdown files should show a Preview button");
    let source: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(source.contains("# Markdown Title"));

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.markdown_preview_rendered());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Source"));
    assert!(rendered.contains("Markdown Title"));
    assert!(!rendered.contains("# Markdown Title"));
    assert!(rendered.contains("https://"));
    assert!(rendered.contains("    1  Markdown Title"));

    app.changes.diff_scroll = 7;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.changes.diff_scroll, 7);
    click(&mut app, preview_button.0, preview_button.1);
    assert!(!app.markdown_preview_rendered());
    assert_eq!(app.changes.diff_scroll, 0);
    app.changes.diff_scroll = 40;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_eq!(app.changes.diff_scroll, 40);

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.markdown_preview_rendered());
    assert_eq!(app.changes.diff_scroll, 7);
    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(!app.markdown_preview_rendered());
    assert_eq!(app.changes.diff_scroll, 40);

    terminal.backend_mut().resize(50, 10);
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_at(Position::new(preview_button.0, preview_button.1))
            .is_none()
    );

    terminal.backend_mut().resize(100, 30);
    app.view = View::Graph;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        app.regions
            .hit_target_at(Position::new(preview_button.0, preview_button.1))
            .is_none()
    );
}

#[test]
fn workspace_focus_passes_through_application_shortcuts() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().to_path_buf());

    app.mode = Mode::WorkspacePanel;
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::WorkspacePanel);

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::Settings);

    app.mode = Mode::WorkspacePanel;
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert_eq!(app.mode, Mode::FileSearch);
}

#[test]
fn selects_visible_text_and_suppresses_clicks_after_dragging() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run_git(root, &["init", "-b", "main"]);
    fs::write(root.join("selected.txt"), "select me\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.changes.set_diff("select me".to_owned());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let diff = app.regions.diff.unwrap();
    let buffer = terminal.backend().buffer();
    let start = (diff.y..diff.bottom())
        .find_map(|row| {
            let text: String = (diff.x..diff.right())
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            text.find("select")
                .map(|column| (diff.x + column as u16, row))
        })
        .expect("rendered preview should contain selectable text");
    let end = (start.0 + 5, start.1);
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        start.0,
        start.1,
    ));
    app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), end.0, end.1));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    let buffer = terminal.backend().buffer();
    let index = usize::from(start.1) * usize::from(buffer.area.width) + usize::from(start.0);
    assert_eq!(buffer.content[index].bg, super::palette().accent);

    app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), end.0, end.1));
    assert_eq!(app.take_copy_request().as_deref(), Some("select"));

    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let graph = app.regions.graph.unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        graph.x + 2,
        graph.y,
    ));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    app.handle_mouse(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        graph.x + 4,
        graph.y,
    ));
    app.handle_mouse(mouse(
        MouseEventKind::Up(MouseButton::Left),
        graph.x + 4,
        graph.y,
    ));
    assert_eq!(app.view, View::Changes);
    assert!(app.take_copy_request().is_some());
}

#[test]
fn worktree_manager_renders_and_uses_semantic_rows() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main");
    let linked = directory.path().join("feature");
    fs::create_dir(&root).unwrap();
    run_git(&root, &["init", "-b", "main"]);
    run_git(&root, &["config", "user.name", "Render Test"]);
    run_git(&root, &["config", "user.email", "render@example.com"]);
    fs::write(root.join("tracked.txt"), "first\n").unwrap();
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "initial commit"]);
    let linked_argument = linked.to_string_lossy().into_owned();
    run_git(
        &root,
        &["worktree", "add", "-b", "feature/modal", &linked_argument],
    );

    let mut app = App::new(root.clone());
    app.handle_key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT));
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Worktrees);
    wait_for(&mut app, |app| !app.worktree_manager.loading);

    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert_black_underlay(&terminal);
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("WORKTREES"));
    assert!(screen.contains("feature/modal"));
    assert!(!screen.contains("ACTIVE REPOSITORY"));
    assert!(!screen.contains("MAIN ·"));
    assert!(!screen.contains("PRIMARY"));
    assert!(!screen.contains("2 CHECKOUTS"));
    assert!(screen.contains("WORKTREE DETAILS"));
    assert!(screen.contains("AVAILABLE ACTIONS"));
    assert!(screen.contains("Native Git worktree"));
    assert!(!screen.contains("HERDR"));

    let mut compact_terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    compact_terminal
        .draw(|frame| draw(frame, &mut app))
        .unwrap();
    let compact_screen = compact_terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(compact_screen.contains("AVAILABLE ACTIONS"));

    let rows = app.worktree_manager.rows();
    let linked_row = rows
        .iter()
        .position(|row| match row {
            WorktreeManagerRow::Worktree {
                repository,
                worktree,
            } => app.worktree_manager.repositories[*repository].worktrees[*worktree].path == linked,
            _ => false,
        })
        .unwrap();
    let target = HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
        generation: app.worktree_manager.content_generation(),
        row: linked_row,
    });
    let row_area = app.regions.hit_target_rect(target).unwrap();
    assert_eq!(row_area.height, 1);

    app.handle_mouse(mouse(MouseEventKind::Moved, row_area.x + 1, row_area.y));
    assert_eq!(app.worktree_manager.state.selected(), Some(linked_row));
    compact_terminal
        .draw(|frame| draw(frame, &mut app))
        .unwrap();
    let row_area = app.regions.hit_target_rect(target).unwrap();
    let buffer = compact_terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let selected_cell = &buffer.content
        [usize::from(row_area.y) * width + usize::from(row_area.x.saturating_add(1))];
    let selected_branch_cell = &buffer.content
        [usize::from(row_area.y) * width + usize::from(row_area.x.saturating_add(7))];
    let selected_path_cell = &buffer.content
        [usize::from(row_area.y) * width + usize::from(row_area.x.saturating_add(22))];
    assert_eq!(selected_cell.bg, super::palette().raised);
    assert_eq!(selected_branch_cell.fg, super::palette().accent);
    assert_eq!(selected_path_cell.fg, super::palette().soft);
    let selected_screen = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(selected_screen.contains("REMOVE Ready"));

    let current_row = rows
        .iter()
        .position(|row| match row {
            WorktreeManagerRow::Worktree {
                repository,
                worktree,
            } => app.worktree_manager.repositories[*repository].worktrees[*worktree].path == root,
            _ => false,
        })
        .unwrap();
    let current_area = app
        .regions
        .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
            generation: app.worktree_manager.content_generation(),
            row: current_row,
        }))
        .unwrap();
    let current_cell = &buffer.content
        [usize::from(current_area.y) * width + usize::from(current_area.x.saturating_add(1))];
    let repository_cell = &buffer.content
        [usize::from(current_area.y) * width + usize::from(current_area.x.saturating_add(2))];
    assert_eq!(current_cell.bg, super::palette().add_bg);
    assert!(repository_cell.modifier.contains(Modifier::BOLD));
    click(&mut app, row_area.x + 1, row_area.y);
    assert_eq!(app.mode, Mode::Explorer);
    assert_eq!(app.explorer_tab, ExplorerTab::Worktrees);

    app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
    assert!(app.worktree_manager.create_dialog.is_some());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let create_dialog = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(create_dialog.contains("CREATE WORKTREE"));
    assert!(create_dialog.contains("Managed by Herdr"));
    assert!(create_dialog.contains("feature/modal"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(app.worktree_manager.remove_dialog_open());
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let dialog = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(dialog.contains("REMOVE WORKTREE"));

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    click(&mut app, 0, 0);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn worktree_manager_separates_groups_with_a_top_margin() {
    let directory = tempfile::tempdir().unwrap();
    let alpha = directory.path().join("alpha");
    let zulu = directory.path().join("zulu");
    for repository in [&alpha, &zulu] {
        fs::create_dir(repository).unwrap();
        run_git(repository, &["init", "-b", "main"]);
        run_git(repository, &["config", "user.name", "Render Test"]);
        run_git(repository, &["config", "user.email", "render@example.com"]);
        fs::write(repository.join("tracked.txt"), "first\n").unwrap();
        run_git(repository, &["add", "."]);
        run_git(repository, &["commit", "-m", "initial commit"]);
    }

    let mut app = App::new(alpha.clone());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    { "workspace_id": "w1", "label": "alpha", "number": 1 },
                    { "workspace_id": "w2", "label": "zulu", "number": 2 }
                ],
                "agents": []
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(alpha.clone());
    app.workspace_panel.workspaces[1].path = Some(zulu.clone());
    app.workspace_panel.begin_group();
    app.workspace_panel.paste("First");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.workspace_panel.begin_workspace_drag(0));
    app.workspace_panel
        .update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
    let _ = app.workspace_panel.finish_workspace_drag();
    app.workspace_panel.begin_group();
    app.workspace_panel.paste("Second");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.workspace_panel.begin_workspace_drag(1));
    app.workspace_panel
        .update_workspace_drag(Some(WorkspaceDropTarget::Group(1)));
    let _ = app.workspace_panel.finish_workspace_drag();
    app.workspace_panel.loading = true;

    let candidates = app.workspace_panel.worktree_candidates();
    let _ = app.worktree_manager.open(
        candidates,
        app.workspace_panel.linked_herdr_worktrees(),
        Some(alpha.clone()),
        app.workspace_panel.is_enabled(),
        app.workspace_panel.worktree_inventory_verified(),
    );
    app.mode = Mode::Explorer;
    app.explorer_tab = ExplorerTab::Worktrees;
    wait_for(&mut app, |app| !app.worktree_manager.loading);
    let rows = app.worktree_manager.rows();
    assert_eq!(
        rows,
        [
            WorktreeManagerRow::Group(0),
            WorktreeManagerRow::Worktree {
                repository: 0,
                worktree: 0,
            },
            WorktreeManagerRow::Group(1),
            WorktreeManagerRow::Worktree {
                repository: 1,
                worktree: 0,
            },
        ]
    );

    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app
        .regions
        .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::List))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let line_at = |y: u16| {
        (list.x..list.right())
            .map(|x| buffer.content[usize::from(y) * width + usize::from(x)].symbol())
            .collect::<String>()
    };
    let row_y = |row: usize| {
        app.regions
            .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                generation: app.worktree_manager.content_generation(),
                row,
            }))
            .unwrap()
            .y
    };
    let alpha_y = row_y(1);
    let zulu_y = row_y(3);
    assert_eq!(zulu_y - alpha_y, 3);
    let group_line = line_at(zulu_y - 1);
    let gap_line = line_at(zulu_y - 2);
    assert!(group_line.contains("SECOND"));
    assert!(
        !gap_line.contains("alpha")
            && !gap_line.contains("zulu")
            && !gap_line.contains("FIRST")
            && !gap_line.contains("SECOND")
    );
    assert!(line_at(alpha_y - 1).contains("FIRST"));
}

#[test]
fn worktree_manager_groups_repositories_into_aligned_columns() {
    let directory = tempfile::tempdir().unwrap();
    let alpha = directory.path().join("alpha");
    let zulu = directory.path().join("zulu");
    for repository in [&alpha, &zulu] {
        fs::create_dir(repository).unwrap();
        run_git(repository, &["init", "-b", "main"]);
        run_git(repository, &["config", "user.name", "Render Test"]);
        run_git(repository, &["config", "user.email", "render@example.com"]);
        fs::write(repository.join("tracked.txt"), "first\n").unwrap();
        run_git(repository, &["add", "."]);
        run_git(repository, &["commit", "-m", "initial commit"]);
    }

    let mut app = App::new(alpha.clone());
    app.workspace_panel = WorkspacePanel::ready_for_test(&serde_json::json!({
        "result": {
            "snapshot": {
                "workspaces": [
                    { "workspace_id": "w1", "label": "alpha", "number": 1 },
                    { "workspace_id": "w2", "label": "zulu", "number": 2 }
                ],
                "agents": []
            }
        }
    }));
    app.workspace_panel.workspaces[0].path = Some(alpha.clone());
    app.workspace_panel.workspaces[1].path = Some(zulu.clone());
    app.workspace_panel.begin_group();
    app.workspace_panel.paste("Projects");
    app.workspace_panel
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for workspace in 0..2 {
        assert!(app.workspace_panel.begin_workspace_drag(workspace));
        app.workspace_panel
            .update_workspace_drag(Some(WorkspaceDropTarget::Group(0)));
        app.workspace_panel.finish_workspace_drag();
    }
    app.workspace_panel.loading = true;

    let candidates = app.workspace_panel.worktree_candidates();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.group.as_deref() == Some("Projects"))
    );
    let _ = app.worktree_manager.open(
        candidates,
        app.workspace_panel.linked_herdr_worktrees(),
        Some(alpha.clone()),
        app.workspace_panel.is_enabled(),
        app.workspace_panel.worktree_inventory_verified(),
    );
    app.mode = Mode::Explorer;
    app.explorer_tab = ExplorerTab::Worktrees;
    wait_for(&mut app, |app| !app.worktree_manager.loading);
    let rows = app.worktree_manager.rows();
    assert_eq!(
        rows,
        [
            WorktreeManagerRow::Group(0),
            WorktreeManagerRow::Worktree {
                repository: 0,
                worktree: 0,
            },
            WorktreeManagerRow::Worktree {
                repository: 1,
                worktree: 0,
            },
        ]
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let list = app
        .regions
        .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::List))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    let line_at = |y: u16| {
        (list.x..list.right())
            .map(|x| buffer.content[usize::from(y) * width + usize::from(x)].symbol())
            .collect::<String>()
    };
    let row_y = |row: usize| {
        app.regions
            .hit_target_rect(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                generation: app.worktree_manager.content_generation(),
                row,
            }))
            .unwrap()
            .y
    };
    let group_line = line_at(row_y(1) - 1);
    let alpha_line = line_at(row_y(1));
    let zulu_line = line_at(row_y(2));
    assert!(group_line.contains("PROJECTS"));
    assert!(alpha_line.contains("alpha"));
    assert!(zulu_line.contains("zulu"));
    let column_of =
        |line: &str, needle: &str| line.find(needle).map(|byte| line[..byte].chars().count());
    let branch_column = column_of(&alpha_line, "main").unwrap();
    assert_eq!(column_of(&zulu_line, "main"), Some(branch_column));
    let path_column = column_of(&alpha_line, "/tmp").unwrap();
    assert_eq!(column_of(&zulu_line, "/tmp"), Some(path_column));
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

#[test]
fn renders_static_media_and_clears_it_for_text_and_overlays() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let image = image::RgbaImage::from_fn(40, 40, |_x, y| {
        if (y / 10) % 2 == 0 {
            image::Rgba([220, 40, 30, 255])
        } else {
            image::Rgba([20, 80, 210, 255])
        }
    });
    image.save(root.join("a-preview.png")).unwrap();
    fs::write(root.join("b-notes.txt"), "plain text preview\n").unwrap();

    let mut app = App::new(root.to_path_buf());
    app.settings.media_preview_protocol = crate::media::MediaPreviewProtocol::Halfblocks;
    assert_eq!(
        app.selected_explorer_file_path().map(|path| path.display()),
        Some("a-preview.png".to_string())
    );
    wait_for(&mut app, |app| app.changes.preview_image.is_some());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    wait_for_halfblock_render(&mut terminal, &mut app);
    let preview_body = app.regions.diff.unwrap();
    let image_cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.symbol() == "▀")
        .collect();
    assert!(!image_cells.is_empty());
    assert!(image_cells.iter().all(|(index, _)| {
        let x = (*index % 100) as u16;
        let y = (*index / 100) as u16;
        x >= preview_body.x
            && x < preview_body.right()
            && y >= preview_body.y
            && y < preview_body.bottom()
    }));

    app.mode = Mode::Settings;
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    assert!(
        !terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "▀")
    );

    app.mode = Mode::Normal;
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes.preview_image.is_none() && app.changes.diff == "plain text preview\n"
    });
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("plain text preview"));
    assert!(!screen.contains('▀'));
}

#[test]
fn corrupt_image_shows_an_error_as_text() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("broken.png"), b"not a png\0\xff").unwrap();
    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| {
        app.changes
            .diff
            .starts_with("Could not read image dimensions:")
    });
    assert!(app.changes.preview_image.is_none());
    assert!(
        app.changes
            .diff
            .starts_with("Could not read image dimensions:")
    );
}

#[test]
fn renders_sqlite_databases_from_the_files_view() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
             INSERT INTO people (name) VALUES ('Ada'), ('Grace');",
        )
        .unwrap();
    drop(connection);

    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| app.changes.sqlite_browser.is_some());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(screen.contains("DATABASE  app.sqlite  read-only"));
    assert!(screen.contains("OBJECTS  1"));
    assert!(screen.contains("people  TABLE"));
    assert!(screen.contains("name · TEXT"));
    assert!(screen.contains("Ada"));
    assert!(screen.contains("Enter explore"));
}

#[test]
fn explores_and_pages_sqlite_databases_with_keys_and_mouse() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("app.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT); \
             WITH RECURSIVE sequence(value) AS ( \
                VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 105 \
             ) INSERT INTO events SELECT value, printf('event-%03d', value) FROM sequence; \
             CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT); \
             INSERT INTO people VALUES (1, 'Ada');",
        )
        .unwrap();
    drop(connection);

    let mut app = App::new(directory.path().to_path_buf());
    wait_for(&mut app, |app| app.changes.sqlite_browser.is_some());
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.changes.sqlite_browser.as_ref().unwrap().focus,
        SqliteFocus::Rows
    );
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    wait_for(&mut app, |app| {
        app.changes
            .sqlite_browser
            .as_ref()
            .and_then(|browser| browser.page.as_ref())
            .is_some_and(|page| page.key.offset == 100 && page.rows.len() == 5)
    });

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    let objects = app.regions.sqlite_objects.unwrap();
    click(&mut app, objects.x + 2, objects.y + 1);
    assert!(app.changes.sqlite_browser.as_ref().unwrap().active);
    assert_eq!(
        app.changes
            .sqlite_browser
            .as_ref()
            .unwrap()
            .selected_object()
            .unwrap()
            .name,
        "people"
    );
    wait_for(&mut app, |app| {
        app.changes
            .sqlite_browser
            .as_ref()
            .and_then(|browser| browser.page.as_ref())
            .is_some_and(|page| page.key.object == "people" && page.rows[0][1] == "Ada")
    });
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.changes.sqlite_browser.as_ref().unwrap().active);
}

fn wait_for_halfblock_render(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let _ = app.poll_worker();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        if terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "▀")
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
