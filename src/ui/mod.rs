mod changes;
mod history;
mod overlays;
pub(crate) mod preview;
mod sqlite;
mod text;
mod workspace_panel;

#[cfg(test)]
mod tests;

pub(super) use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
pub(super) use unicode_segmentation::UnicodeSegmentation;
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::{
    app::{
        App, BranchPickerStep, ExplorerTab, FileDialogKind, GraphHitTarget, HeaderPickerItem,
        HeaderPickerKind, HitTarget, LeftPane, Mode, Regions, ShortcutAction, TAB_WIDTH, View,
        WorkspacePanelHitTarget,
    },
    theme::{Palette, load_theme},
};

mod header;
use header::*;

fn palette() -> &'static Palette {
    static THEME: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
    THEME.get_or_init(|| load_theme().palette)
}

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    app.regions = Regions::default();
    app.regions.screen = Some(frame.area());
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().canvas).fg(palette().ink)),
        frame.area(),
    );

    if frame.area().width < 60 || frame.area().height < 16 {
        app.reset_media_presentation();
        let message = if app.mode == Mode::FileEdit {
            format!(
                "hunkle editor needs at least 60 columns and 16 rows\n\n{}  save + close    esc  close",
                app.settings.shortcuts.label(ShortcutAction::SaveOrFormat)
            )
        } else {
            format!(
                "hunkle needs at least 60 columns and 16 rows\n\n{}  quit",
                app.settings.shortcuts.label(ShortcutAction::Quit)
            )
        };
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .style(Style::default().fg(palette().ink)),
            frame.area(),
        );
        finish_selection(frame, app);
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .split(frame.area());

    draw_header(frame, app, layout[0]);
    let content = layout[1];
    app.workspace_panel
        .set_visible(app.mode == Mode::WorkspacePanel);
    let main_content = content;
    if app.workspace_loading_initial_state() {
        app.reset_media_presentation();
        draw_empty(frame, main_content, "Loading workspace…");
        draw_navigation(frame, app, layout[2]);
        finish_selection(frame, app);
        return;
    }
    let visible_view = app.visible_view();
    changes::draw(
        frame,
        app,
        main_content,
        visible_view != View::Graph || app.graph_commit_open,
    );
    if visible_view == View::Graph && !app.graph_commit_open {
        let graph_area = app.regions.diff.unwrap_or(main_content);
        frame.render_widget(Clear, graph_area);
        app.regions.diff = None;
        app.regions.diff_scrollbar = None;
        app.regions.diff_scroll_thumb = None;
        app.regions.diff_scroll_max = 0;
        app.regions.diff_hunks.clear();
        app.regions.clear_hit_targets_in(graph_area);
        let graph_regions = history::draw_graph(
            frame,
            graph_area,
            history::GraphView {
                repo: app.session.data(),
                summaries: &app.commit_summaries,
                author_filter: &app.author_filter,
                state: &mut app.graph_state,
                scroll_to_selection: &mut app.graph_scroll_to_selection,
                settings: &app.settings,
                dragging_column: app.dragging_graph_column.map(|drag| drag.right),
            },
        );
        app.regions.graph_table = graph_regions.table;
        app.regions.graph_columns = graph_regions.columns;
        for (target, rect) in graph_regions.targets {
            app.regions.register_hit_target(target, rect);
        }
    }
    draw_main_top_padding(frame, app, layout[1]);
    draw_navigation(frame, app, layout[2]);
    match app.mode {
        Mode::FileSearch => {
            dim(frame);
            let files = app
                .session
                .data()
                .map_or(&[][..], |repo| repo.files.as_slice());
            let regions = overlays::draw_file_search(
                frame,
                &mut app.file_search,
                files,
                &app.settings.shortcuts,
            );
            app.regions.file_search_overlay = Some(regions.overlay);
            app.regions.file_search_list = Some(regions.list);
        }
        Mode::Explorer => {
            dim(frame);
            let targets = match app.explorer_tab {
                ExplorerTab::Explorer => overlays::draw_explorer(
                    frame,
                    &mut app.workspace_explorer,
                    &app.settings.shortcuts,
                ),
                ExplorerTab::Worktrees => overlays::draw_worktree_manager(
                    frame,
                    &mut app.worktree_manager,
                    &app.settings.shortcuts,
                ),
                ExplorerTab::Branches => overlays::draw_repository_browser(
                    frame,
                    &mut app.repository_browser,
                    &app.settings.shortcuts,
                ),
            };
            for (target, rect) in targets {
                app.regions.register_hit_target(target, rect);
            }
            if app.explorer_tab == ExplorerTab::Worktrees {
                if let Some(dialog) = &app.worktree_manager.create_dialog {
                    dim(frame);
                    overlays::draw_worktree_create_dialog(frame, dialog);
                } else if let Some(dialog) = &app.worktree_manager.remove_dialog {
                    dim(frame);
                    overlays::draw_worktree_remove_dialog(frame, dialog);
                }
            } else if app.explorer_tab == ExplorerTab::Branches
                && let Some(dialog) = &app.repository_browser.branch_delete
            {
                dim(frame);
                overlays::draw_branch_delete_dialog(frame, dialog);
            }
        }
        Mode::Settings => {
            dim(frame);
            let regions = overlays::draw_settings(
                frame,
                overlays::SettingsView {
                    settings: &app.settings,
                    page: app.settings_page,
                    selection: app.settings_selection,
                    shortcut_selection: app.shortcut_selection,
                    shortcut_scroll: app.shortcut_scroll,
                    shortcut_capture: app.shortcut_capture,
                    shortcut_error: app.shortcut_error.as_deref(),
                    opencode_selection: app.opencode_selection,
                    opencode_model_input: app.opencode_model_input.as_deref(),
                    opencode_error: app.opencode_error.as_deref(),
                },
                app.fetch_running(),
            );
            app.regions.settings_overlay = Some(regions.overlay);
            app.regions.settings_general_tab = Some(regions.general_tab);
            app.regions.settings_shortcuts_tab = Some(regions.shortcuts_tab);
            app.regions.settings_opencode_tab = Some(regions.opencode_tab);
            app.regions.auto_fetch = regions.auto_fetch;
            app.regions.fetch_interval = regions.fetch_interval;
            app.regions.fetch_interval_down = regions.fetch_interval_down;
            app.regions.fetch_interval_up = regions.fetch_interval_up;
            app.regions.format_on_save_setting = regions.format_on_save;
            app.regions.opencode_model_setting = regions.opencode_model;
            app.regions.opencode_reasoning_setting = regions.opencode_reasoning;
            app.regions.workspace_panel_setting = regions.workspace_panel;
            app.regions.agent_harness_setting = regions.agent_harness;
            app.regions.agent_time_setting = regions.agent_time;
            app.regions.clear_agent_timings_setting = regions.clear_agent_timings;
            app.regions.media_preview_setting = regions.media_preview;
            app.regions.editor_setting = regions.editor;
            app.regions.shortcut_rows = regions.shortcut_rows;
        }
        Mode::AuthorFilter => {
            let anchor = app
                .regions
                .hit_target_rect(HitTarget::Graph(GraphHitTarget::AuthorHeader))
                .unwrap_or(Rect::new(main_content.x, main_content.y, 1, 1));
            for (target, rect) in history::draw_author_filter(
                frame,
                anchor,
                &mut app.author_filter,
                &app.settings.shortcuts,
            ) {
                app.regions.register_hit_target(target, rect);
            }
        }
        Mode::ActionMenu => {
            let anchor = app.regions.actions.unwrap_or(Rect::new(
                main_content.x.saturating_add(1),
                main_content.y,
                1,
                1,
            ));
            let regions = overlays::draw_action_menu(frame, anchor, app.actions.selection);
            app.regions.action_menu = Some(regions.overlay);
            app.regions.action_list = Some(regions.list);
        }
        Mode::Command => {
            dim(frame);
            let regions = overlays::draw_command(frame, &mut app.actions);
            app.regions.command_overlay = Some(regions.overlay);
            app.regions.command_output = Some(regions.output);
        }
        Mode::HerdrPrompt => {
            dim(frame);
            app.regions.herdr_prompt_overlay = Some(overlays::draw_herdr_prompt(
                frame,
                &app.herdr_prompt,
                &app.settings.shortcuts,
            ));
        }
        Mode::FileEdit => {
            draw_file_editor(frame, app);
            draw_main_top_padding(frame, app, layout[1]);
        }
        Mode::Editor => {
            dim(frame);
            app.regions.editor_overlay = Some(overlays::draw_editor(
                frame,
                &app.editor_input,
                app.editor_error.as_deref(),
                app.editor_configure_only,
            ));
        }
        Mode::Files => {
            if let Some(dialog) = &app.file_dialog {
                let regions = if matches!(dialog.kind, FileDialogKind::Add { .. }) {
                    let anchor = app.regions.files_add.unwrap_or(Rect::new(
                        content.right().saturating_sub(1),
                        content.y,
                        1,
                        1,
                    ));
                    overlays::draw_file_add_popover(frame, anchor, dialog.choice)
                } else {
                    dim(frame);
                    overlays::draw_file_dialog(frame, dialog)
                };
                app.regions.file_dialog_overlay = Some(regions.overlay);
                app.regions.file_dialog_primary = Some(regions.primary);
                app.regions.file_dialog_secondary = Some(regions.secondary);
            }
        }
        Mode::Help => {
            dim(frame);
            overlays::draw_help(frame, &app.settings.shortcuts);
        }
        Mode::WorkspacePanel => {
            dim(frame);
            let panel_area = workspace_panel::drawer_area(frame.area());
            app.regions.workspace_panel = Some(panel_area);
            let (workspace_section, agent_section) = workspace_panel::section_areas(panel_area);
            app.regions.workspace_panel_workspaces = Some(workspace_section);
            app.regions.workspace_panel_agents = Some(agent_section);
            let workspace_panel_hover = match app.hovered_hit_target {
                Some(HitTarget::WorkspacePanel(target)) => Some(target),
                _ => None,
            };
            let loaded_workspace_path = app.repository().map(|repository| repository.root.clone());
            for (target, rect) in workspace_panel::draw(
                frame,
                &mut app.workspace_panel,
                panel_area,
                workspace_panel_hover,
                &app.settings,
                loaded_workspace_path.as_deref(),
            ) {
                app.regions.register_hit_target(target, rect);
            }
            if let Some(dialog) = &app.workspace_panel.rename_dialog {
                dim(frame);
                overlays::draw_workspace_rename_dialog(frame, dialog);
            } else if let Some(dialog) = &app.workspace_panel.snapshot_load_dialog {
                dim(frame);
                overlays::draw_snapshot_load_dialog(frame, dialog);
            } else if let Some(dialog) = &app.workspace_panel.delete_dialog {
                dim(frame);
                overlays::draw_workspace_delete_dialog(frame, dialog);
            }
        }
        Mode::WorkspacePresets => {
            dim(frame);
            if let Some(dialog) = &app.workspace_panel.snapshot_load_dialog {
                overlays::draw_snapshot_load_dialog(frame, dialog);
            } else {
                let (overlay, targets) = overlays::draw_workspace_presets(
                    frame,
                    &app.workspace_panel,
                    &app.settings.shortcuts,
                );
                app.regions.workspace_presets_overlay = Some(overlay);
                for (target, rect) in targets {
                    app.regions.register_hit_target(target, rect);
                }
            }
        }
        Mode::Normal | Mode::Commit => {}
    }
    if app.header_picker.is_open() {
        dim_except_header_controls(frame, app);
        draw_header_picker(frame, app);
    }
    finish_selection(frame, app);
}

fn draw_file_editor(frame: &mut Frame<'_>, app: &mut App) {
    let Some(panel) = app.regions.diff else {
        return;
    };
    let header = Rect::new(
        panel.x.saturating_add(1),
        panel.y.saturating_add(1),
        panel.width.saturating_sub(2),
        1,
    );
    let body = Rect::new(
        header.x,
        header.y.saturating_add(2),
        header.width,
        panel.bottom().saturating_sub(header.y.saturating_add(3)),
    );
    const LINE_NUMBER_WIDTH: u16 = 7;
    let gutter_width = LINE_NUMBER_WIDTH.min(body.width);
    let gutter = Rect::new(body.x, body.y, gutter_width, body.height);
    let editor_body = Rect::new(
        body.x.saturating_add(gutter_width),
        body.y,
        body.width.saturating_sub(gutter_width),
        body.height,
    );
    app.regions.preview_body = Some(editor_body);
    app.regions.diff_scrollbar = None;
    app.regions.diff_scroll_thumb = None;
    app.regions.diff_scroll_max = 0;
    app.regions.editor_rows.clear();
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().panel).fg(palette().ink)),
        panel,
    );

    let save_label = app.settings.shortcuts.label(ShortcutAction::SaveOrFormat);
    let wrapped = app.changes.diff_wrap;
    let Some(editor) = &mut app.file_editor else {
        return;
    };
    let (cursor_line, cursor_column) = editor.cursor_position();
    let path = editor.path().display();
    let dirty = if editor.dirty() { "modified" } else { "saved" };
    let title = format!(
        "EDIT  {path}  {dirty}  Ln {}, Col {}  {save_label} save + close  esc close",
        cursor_line.saturating_add(1),
        cursor_column.saturating_add(1)
    );
    frame.render_widget(
        Paragraph::new(truncate_width(&title, usize::from(header.width))).style(
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        header,
    );

    let viewport_height = usize::from(editor_body.height);
    let viewport_width = usize::from(editor_body.width).max(1);
    let (lines, line_numbers, cursor_row, rendered_cursor_column) = if wrapped {
        let (cursor_row, rendered_cursor_column) =
            wrapped_editor_cursor(editor.text(), viewport_width, cursor_line, cursor_column);
        if let Some(anchor) = app.file_editor_anchor.take() {
            let row = usize::from(anchor.y.saturating_sub(editor_body.y))
                .min(viewport_height.saturating_sub(1));
            editor.anchor_wrapped_cursor_at(cursor_row, row);
        }
        editor.ensure_wrapped_cursor_visible(cursor_row, viewport_height);
        let (lines, line_numbers, rows) = wrapped_editor_view(
            editor.text(),
            &path,
            viewport_width,
            editor.wrap_scroll_row,
            viewport_height,
        );
        app.regions.editor_rows = rows;
        (
            lines,
            line_numbers,
            cursor_row.saturating_sub(editor.wrap_scroll_row),
            rendered_cursor_column,
        )
    } else {
        if let Some(anchor) = app.file_editor_anchor.take() {
            let row = usize::from(anchor.y.saturating_sub(editor_body.y))
                .min(viewport_height.saturating_sub(1));
            let column = usize::from(anchor.x.saturating_sub(editor_body.x))
                .min(viewport_width.saturating_sub(1));
            editor.anchor_cursor_at(row, column);
        }
        editor.ensure_cursor_visible(viewport_height, viewport_width);
        let lines = text::styled_source_window(
            editor.text(),
            &path,
            0,
            editor.scroll_line,
            viewport_height,
        );
        let mut lines = editor_visible_lines(lines, editor.scroll_column, viewport_width);
        while lines.len() <= cursor_line.saturating_sub(editor.scroll_line) {
            lines.push(Line::default().style(Style::default().bg(palette().panel)));
        }
        let line_count = editor.visible_line_count();
        let line_numbers = (0..viewport_height)
            .map(|row| {
                let line = editor.scroll_line.saturating_add(row);
                editor_line_number((line < line_count).then_some(line))
            })
            .collect::<Vec<_>>();
        (
            lines,
            line_numbers,
            cursor_line.saturating_sub(editor.scroll_line),
            cursor_column.saturating_sub(editor.scroll_column),
        )
    };
    frame.render_widget(Paragraph::new(line_numbers), gutter);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(palette().panel)),
        editor_body,
    );
    let cursor_x = editor_body
        .x
        .saturating_add(u16::try_from(rendered_cursor_column).unwrap_or(u16::MAX));
    let cursor_y = editor_body
        .y
        .saturating_add(u16::try_from(cursor_row).unwrap_or(u16::MAX));
    if cursor_x < editor_body.right() && cursor_y < editor_body.bottom() {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn editor_line_number(line: Option<usize>) -> Line<'static> {
    line.map_or_else(
        || Line::default().style(Style::default().bg(palette().panel)),
        |line| {
            Line::styled(
                format!("{:>5}  ", line.saturating_add(1)),
                Style::default().fg(palette().faint).bg(palette().panel),
            )
        },
    )
}

fn wrapped_editor_cursor(
    source: &str,
    width: usize,
    cursor_line: usize,
    cursor_column: usize,
) -> (usize, usize) {
    let mut visual_row = 0usize;
    for (line, content) in editor_source_lines(source).enumerate() {
        let rows = text::word_wrapped_rows(content, width);
        if line == cursor_line {
            let (row, rendered_column) = rows
                .iter()
                .enumerate()
                .min_by_key(|(_, row)| {
                    row.source_column_at(row.rendered_column_at(cursor_column))
                        .abs_diff(cursor_column)
                })
                .map_or((0, 0), |(index, row)| {
                    (index, row.rendered_column_at(cursor_column))
                });
            return (visual_row.saturating_add(row), rendered_column);
        }
        visual_row = visual_row.saturating_add(rows.len());
    }
    (visual_row, 0)
}

fn wrapped_editor_view(
    source: &str,
    path: &str,
    width: usize,
    scroll: usize,
    height: usize,
) -> (
    Vec<Line<'static>>,
    Vec<Line<'static>>,
    Vec<crate::app::EditorRenderedRow>,
) {
    let mut lines = Vec::new();
    let mut line_numbers = Vec::new();
    let mut rendered_rows = Vec::new();
    let mut visual_row = 0usize;
    for (line_number, content) in editor_source_lines(source).enumerate() {
        let rows = text::word_wrapped_rows(content, width);
        let line_end = visual_row.saturating_add(rows.len());
        if line_end > scroll && visual_row < scroll.saturating_add(height) {
            let styled = text::styled_source_window(source, path, 0, line_number, 1)
                .into_iter()
                .next()
                .unwrap_or_default();
            for (index, row) in rows.iter().enumerate() {
                let absolute_row = visual_row.saturating_add(index);
                if absolute_row < scroll || absolute_row >= scroll.saturating_add(height) {
                    continue;
                }
                let rendered =
                    editor_visible_lines(vec![styled.clone()], row.source_start(), row.width())
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                lines.push(rendered);
                line_numbers.push(editor_line_number((index == 0).then_some(line_number)));
                rendered_rows.push(crate::app::EditorRenderedRow {
                    line: line_number,
                    columns: row.columns(),
                });
            }
        }
        visual_row = line_end;
        if visual_row >= scroll.saturating_add(height) {
            break;
        }
    }
    while lines.len() < height {
        lines.push(Line::default().style(Style::default().bg(palette().panel)));
        line_numbers.push(editor_line_number(None));
    }
    (lines, line_numbers, rendered_rows)
}

fn editor_source_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

fn editor_visible_lines(
    mut lines: Vec<Line<'static>>,
    start: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let end = start.saturating_add(width);
    for line in &mut lines {
        let mut column = 0usize;
        let mut visible_spans = Vec::new();
        for span in std::mem::take(&mut line.spans) {
            let mut visible = String::new();
            for grapheme in span.content.graphemes(true) {
                let grapheme_width = if grapheme == "\t" {
                    TAB_WIDTH - column % TAB_WIDTH
                } else {
                    UnicodeWidthStr::width(grapheme)
                };
                let grapheme_end = column.saturating_add(grapheme_width);
                if grapheme_width == 0 {
                    if column >= start && column < end {
                        visible.push_str(grapheme);
                    }
                } else if column >= start && grapheme_end <= end {
                    if grapheme == "\t" {
                        visible.push_str(&" ".repeat(grapheme_width));
                    } else {
                        visible.push_str(grapheme);
                    }
                } else {
                    let overlap_start = column.max(start);
                    let overlap_end = grapheme_end.min(end);
                    visible.push_str(&" ".repeat(overlap_end.saturating_sub(overlap_start)));
                }
                column = grapheme_end;
                if column >= end {
                    break;
                }
            }
            if !visible.is_empty() {
                visible_spans.push(Span::styled(visible, span.style));
            }
            if column >= end {
                break;
            }
        }
        line.spans = visible_spans;
    }
    lines
}

fn finish_selection(frame: &mut Frame<'_>, app: &mut App) {
    if app.selection.needs_capture(frame.area()) {
        app.selection.capture(frame.buffer_mut());
    }
    app.selection.render(
        frame.buffer_mut(),
        Style::default().fg(palette().canvas).bg(palette().accent),
    );
    app.selection.discard_inactive_capture();
}

fn dim(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.buffer_mut().set_style(
        area,
        Style::default()
            .bg(Color::Rgb(0, 0, 0))
            .add_modifier(Modifier::DIM),
    );
}

fn dim_except_header_controls(frame: &mut Frame<'_>, app: &App) {
    let mut preserved = Vec::new();
    for target in [
        HitTarget::HeaderRepository,
        HitTarget::HeaderWorktrees,
        HitTarget::HeaderBranch,
        HitTarget::HeaderDiff,
    ] {
        let Some(rect) = app.regions.hit_target_rect(target) else {
            continue;
        };
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                if let Some(cell) = frame.buffer_mut().cell((x, y)).cloned() {
                    preserved.push((x, y, cell));
                }
            }
        }
    }
    let area = frame.area();
    frame.buffer_mut().set_style(
        area,
        Style::default()
            .bg(Color::Rgb(0, 0, 0))
            .add_modifier(Modifier::DIM),
    );
    for (x, y, preserved) in preserved {
        if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
            *cell = preserved;
        }
    }
}

fn draw_navigation(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().surface_alt)),
        area,
    );

    let compact = area.width < 100;
    let left_pane_label = if app.changes.pane == LeftPane::Worktree {
        "Files"
    } else {
        "Changes"
    };
    let mut labels = vec![
        (
            app.settings.shortcuts.label(ShortcutAction::ToggleGraph),
            "Git Graph",
        ),
        (
            app.settings.shortcuts.label(ShortcutAction::TogglePane),
            left_pane_label,
        ),
    ];
    if app.workspace_panel_available() {
        labels.push((
            app.settings
                .shortcuts
                .label(ShortcutAction::ToggleWorkspace),
            "Workspaces",
        ));
    }
    labels.extend([
        (
            app.settings.shortcuts.label(ShortcutAction::OpenExplorer),
            "Explorer",
        ),
        (
            app.settings.shortcuts.label(ShortcutAction::OpenSettings),
            "Settings",
        ),
        (
            app.settings.shortcuts.label(ShortcutAction::OpenHelp),
            "Help",
        ),
    ]);

    let total_width = labels.iter().fold(0_u16, |width, (key, label)| {
        let label_width = if compact {
            0
        } else {
            UnicodeWidthStr::width(*label) as u16 + 1
        };
        width.saturating_add(UnicodeWidthStr::width(key.as_str()) as u16 + label_width + 2)
    });
    let mut spans = Vec::new();
    let start_x = area.right().saturating_sub(total_width).max(area.x);
    if start_x > area.x
        && let Some(path) = app
            .repository()
            .map(|repository| repository.root.display().to_string())
    {
        let width = usize::from(start_x.saturating_sub(area.x));
        frame.render_widget(
            Paragraph::new(truncate_width(&format!(" {path}"), width))
                .style(Style::default().fg(palette().soft)),
            Rect::new(area.x, area.y, start_x.saturating_sub(area.x), 1),
        );
    }
    let mut x = start_x;
    let mut rects = Vec::new();
    for (index, (key, label)) in labels.iter().enumerate() {
        let active = index == 0 && app.visible_view() == View::Graph;
        let background = active.then_some(palette().raised);
        spans.push(Span::styled(
            " ",
            Style::default().bg(background.unwrap_or(palette().surface_alt)),
        ));
        spans.push(Span::styled(
            key.as_str(),
            Style::default()
                .fg(palette().orange)
                .bg(background.unwrap_or(palette().surface_alt))
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        if !compact {
            spans.push(Span::styled(
                format!(" {label}"),
                Style::default()
                    .fg(if active {
                        palette().accent
                    } else {
                        palette().muted
                    })
                    .bg(background.unwrap_or(palette().surface_alt))
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ));
        }
        spans.push(Span::styled(
            " ",
            Style::default().bg(background.unwrap_or(palette().surface_alt)),
        ));
        let width = UnicodeWidthStr::width(key.as_str()) as u16
            + if compact {
                2
            } else {
                UnicodeWidthStr::width(*label) as u16 + 3
            };
        rects.push(Rect::new(x, area.y, width, 1));
        x = x.saturating_add(width);
    }

    app.regions.changes = None;
    app.regions.graph = rects.first().copied();
    app.regions.left_pane_toggle = rects.get(1).copied();
    if app.workspace_panel_available()
        && let Some(rect) = rects.get(2).copied()
    {
        app.regions.register_hit_target(
            HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Focus),
            rect,
        );
    }
    let offset = usize::from(app.workspace_panel_available());
    app.regions.explorer = rects.get(2 + offset).copied();
    app.regions.settings = rects.get(3 + offset).copied();
    app.regions.help = rects.get(4 + offset).copied();

    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(start_x, area.y, area.right().saturating_sub(start_x), 1),
    );
}

fn truncate_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let target = width.saturating_sub(1);
    let mut result = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > target {
            break;
        }
        result.push_str(grapheme);
        used += grapheme_width;
    }
    result.push('…');
    result
}

pub(super) fn truncate_start_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let target = width.saturating_sub(1);
    let mut suffix = Vec::new();
    let mut used = 0;
    for grapheme in value.graphemes(true).rev() {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > target {
            break;
        }
        suffix.push(grapheme);
        used += grapheme_width;
    }
    suffix.reverse();
    format!("…{}", suffix.concat())
}

fn draw_empty(frame: &mut Frame<'_>, area: Rect, message: &str) {
    fill(frame, area, palette().panel);
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(""),
            Line::styled(
                message,
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                "Press o to choose a directory",
                Style::default().fg(palette().muted),
            ),
        ])
        .alignment(Alignment::Center),
        area,
    );
}

fn fill(frame: &mut Frame<'_>, area: Rect, color: Color) {
    frame.render_widget(Block::default().style(Style::default().bg(color)), area);
}
