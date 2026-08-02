mod agents;
mod changes;
mod history;
mod overlays;
pub(crate) mod preview;
mod sqlite;
mod text;

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
        AgentDestinationKind, App, BranchPickerStep, CloneField, FileDialogKind, GraphHitTarget,
        HeaderPickerItem, HeaderPickerKind, HitTarget, LeftPane, Mode, Regions,
        RepositoryPickerStep, ShortcutAction, TAB_WIDTH, TextInput, View, WorktreePickerField,
        WorktreePickerStep,
    },
    theme::{Palette, load_theme},
};

mod editor;
use editor::draw_file_editor;
#[cfg(test)]
use editor::{selected_display_range, wrapped_editor_cursor};
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
        draw_agent_pane_picker_overlay(frame, app);
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
    let main_content = content;
    if app.workspace_loading_initial_state() {
        app.reset_media_presentation();
        draw_empty(frame, main_content, "Loading workspace…");
        draw_navigation(frame, app, layout[2]);
        draw_agent_pane_picker_overlay(frame, app);
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
    let hovered_agent_message = match app.hovered_hit_target {
        Some(HitTarget::Agent(index)) => Some((index, None)),
        Some(
            HitTarget::AgentTooltip { agent, message } | HitTarget::AgentMessage { agent, message },
        ) => Some((agent, Some(message))),
        _ => None,
    };
    if let Some((index, message)) = hovered_agent_message
        && let Some(anchor) = app.regions.hit_target_rect(HitTarget::Agent(index))
        && app
            .herdr
            .agent_user_messages(index)
            .is_some_and(|messages| !messages.is_empty())
    {
        if let Some(viewer) = app.regions.diff {
            dim_area(frame, viewer);
        }
        for (target, rect) in agents::draw_tooltip(frame, &app.herdr, index, message, anchor) {
            app.regions.register_hit_target(target, rect);
        }
    }
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
            let targets = overlays::draw_explorer(
                frame,
                &mut app.workspace_explorer,
                &app.settings.shortcuts,
            );
            for (target, rect) in targets {
                app.regions.register_hit_target(target, rect);
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
            app.regions.cross_workspace_agents_setting = regions.cross_workspace_agents;
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
        Mode::Normal | Mode::Commit => {}
    }
    if app.header_picker.is_open() {
        dim_except_header_controls(frame, app);
        draw_header_picker(frame, app);
    }
    draw_agent_pane_picker_overlay(frame, app);
    finish_selection(frame, app);
}

fn draw_agent_pane_picker_overlay(frame: &mut Frame<'_>, app: &mut App) {
    if app.herdr_prompt.agent_pane_picker_open() {
        dim(frame);
        for (target, rect) in
            overlays::draw_agent_pane_picker(frame, &app.herdr_prompt, app.hovered_hit_target)
        {
            app.regions.register_hit_target(target, rect);
        }
    }
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

fn dim_area(frame: &mut Frame<'_>, area: Rect) {
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
        HitTarget::HeaderAgent,
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
        && let Some(path) = app.repository().map(|repository| {
            let path = repository.root.display().to_string();
            if repository.is_local() || repository.branch.is_empty() {
                path
            } else {
                format!("{path}:{}", repository.branch)
            }
        })
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
    app.regions.explorer = rects.get(2).copied();
    app.regions.settings = rects.get(3).copied();
    app.regions.help = rects.get(4).copied();

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
