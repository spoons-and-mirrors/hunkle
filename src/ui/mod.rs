mod agents;
mod changes;
mod header_card;
mod history;
mod location_picker;
mod overlays;
pub(crate) mod preview;
mod sqlite;
mod text;
mod workspace;

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

use std::{env, path::Path};

pub(super) use crate::{
    app::{
        APP_MIN_WIDTH, App, BranchPickerStep, CloneField, FOOTER_MARQUEE_PAUSE,
        FOOTER_MARQUEE_STEP, FileDialogKind, GraphHitTarget, HeaderPickerItem, HeaderPickerKind,
        HitTarget, LayoutProfile, LeftPane, Mode, RepositoryPickerStep, ScrollTarget,
        ShortcutAction, TAB_WIDTH, TextInput, View, WorktreePickerStep,
    },
    theme::{Palette, load_theme},
};

mod editor;
use editor::draw_file_editor;
#[cfg(test)]
use editor::{selected_display_range, wrapped_editor_cursor};
mod header;
use header::*;
use header_card::*;
use location_picker::*;

fn palette() -> &'static Palette {
    static THEME: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
    THEME.get_or_init(|| load_theme().palette)
}

fn text_input_lines(input: &TextInput, active: bool, inactive: Color) -> Vec<Line<'static>> {
    let selection = active.then(|| input.selection()).flatten();
    let mut line_start = 0;
    input
        .text()
        .split('\n')
        .map(|line| {
            if !active {
                line_start += line.len() + 1;
                return Line::styled(line.to_owned(), Style::default().fg(inactive));
            }
            let mut spans = line
                .char_indices()
                .map(|(offset, character)| {
                    let index = line_start + offset;
                    let style = if input.cursor_visible() && input.cursor() == index {
                        Style::default().fg(palette().canvas).bg(palette().accent)
                    } else if selection.is_some_and(|(start, end)| start <= index && index < end) {
                        Style::default().fg(palette().ink).bg(palette().selected)
                    } else {
                        Style::default().fg(palette().ink)
                    };
                    Span::styled(character.to_string(), style)
                })
                .collect::<Vec<_>>();
            if input.cursor() == line_start + line.len() {
                spans.push(Span::styled(
                    " ",
                    if input.cursor_visible() {
                        Style::default().bg(palette().accent)
                    } else {
                        Style::default()
                    },
                ));
            }
            line_start += line.len() + 1;
            Line::from(spans)
        })
        .collect()
}

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let profile = app.begin_render_frame(frame.area());
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().canvas).fg(palette().ink)),
        frame.area(),
    );

    if frame.area().width < APP_MIN_WIDTH || frame.area().height < 16 {
        app.reset_media_presentation();
        let message = if app.mode == Mode::FileEdit {
            format!(
                "hunkle editor needs at least {APP_MIN_WIDTH} columns and 16 rows\n\n{}  save + close    esc  close",
                app.settings.shortcuts.label(ShortcutAction::SaveOrFormat)
            )
        } else {
            format!(
                "hunkle needs at least {APP_MIN_WIDTH} columns and 16 rows\n\n{}  quit",
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

    let hide_navigation = profile.is_single()
        && app.workspace_detail_open()
        && app.agents_pane_visible()
        && !app.notice.as_deref().is_some_and(notice_is_error);
    let layout = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(6),
        Constraint::Length(u16::from(!hide_navigation)),
    ])
    .split(frame.area());

    draw_header(frame, app, layout[0], profile);
    let content = layout[1];
    let main_content = content;
    if app.workspace_loading_initial_state() {
        app.reset_media_presentation();
        draw_empty(frame, main_content, "Loading workspace…");
        draw_navigation(frame, app, layout[2], profile);
        draw_agent_pane_picker_overlay(frame, app);
        finish_selection(frame, app);
        return;
    }
    workspace::draw(frame, app, main_content, profile);
    draw_main_top_padding(frame, app, layout[1], profile);
    draw_header_card_bottom_padding(frame, app);
    draw_navigation(frame, app, layout[2], profile);
    if matches!(
        app.mode,
        Mode::Explorer
            | Mode::Settings
            | Mode::AuthorFilter
            | Mode::ActionMenu
            | Mode::Command
            | Mode::HerdrPrompt
            | Mode::Editor
            | Mode::Files
            | Mode::Help
            | Mode::Scheduler
    ) {
        app.regions.capture_scroll_boundary();
    }
    match app.mode {
        Mode::Explorer => {
            dim(frame);
            let targets = overlays::draw_explorer(
                frame,
                &mut app.workspace_explorer,
                &app.settings.shortcuts,
            );
            for (target, rect) in targets {
                if matches!(
                    target,
                    HitTarget::Explorer(
                        crate::app::ExplorerHitTarget::SurroundingsPane
                            | crate::app::ExplorerHitTarget::Surrounding { .. }
                    )
                ) {
                    app.regions
                        .register_scroll_target(ScrollTarget::WorkspaceExplorerSurroundings, rect);
                }
                if target == HitTarget::Explorer(crate::app::ExplorerHitTarget::Overlay) {
                    app.regions
                        .register_scroll_target(ScrollTarget::WorkspaceExplorer, rect);
                }
                app.regions.register_hit_target(target, rect);
            }
        }
        Mode::Settings => {
            dim(frame);
            let targets = overlays::draw_settings(
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
                    herdr_available: app.herdr_available(),
                },
                app.fetch_running(),
            );
            for (target, rect) in targets {
                if app.settings_page == crate::app::SettingsPage::Shortcuts
                    && target == HitTarget::Settings(crate::app::SettingsHitTarget::Overlay)
                {
                    app.regions
                        .register_scroll_target(ScrollTarget::SettingsShortcuts, rect);
                }
                app.regions.register_hit_target(target, rect);
            }
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
                if target == HitTarget::Graph(GraphHitTarget::FilterOverlay) {
                    app.regions
                        .register_scroll_target(ScrollTarget::AuthorFilter, rect);
                }
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
            app.regions
                .register_scroll_target(ScrollTarget::ActionMenu, regions.list);
        }
        Mode::Command => {
            dim(frame);
            let regions = overlays::draw_command(frame, &mut app.actions);
            app.regions.command_overlay = Some(regions.overlay);
            app.regions.command_output = Some(regions.output);
            app.regions
                .register_scroll_target(ScrollTarget::CommandOutput, regions.output);
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
            draw_file_editor(frame, app, profile);
            draw_main_top_padding(frame, app, layout[1], profile);
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
            overlays::draw_help(frame, &app.settings.shortcuts, app.herdr_available());
        }
        Mode::Scheduler => {
            dim(frame);
            let regions = overlays::draw_scheduler(frame, app, profile);
            for (target, rect) in regions.targets {
                app.regions.register_hit_target(target, rect);
            }
            for (target, rect) in regions.scrolls {
                app.regions.register_scroll_target(target, rect);
            }
        }
        Mode::Normal | Mode::Commit => {}
    }
    draw_agent_pane_picker_overlay(frame, app);
    if app.header_picker.is_open() {
        dim_except_header_controls(frame, app);
        draw_header_picker(frame, app, profile);
        app.regions
            .capture_scroll_target(ScrollTarget::HeaderPicker);
    }
    finish_selection(frame, app);
}

fn draw_agent_pane_picker_overlay(frame: &mut Frame<'_>, app: &mut App) {
    if app.herdr_prompt.agent_pane_picker_open() {
        app.regions.capture_scroll_boundary();
        dim_except_header_controls(frame, app);
        for (target, rect) in overlays::draw_agent_pane_picker(
            frame,
            &app.herdr_prompt,
            app.hovered_hit_target.clone(),
        ) {
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

fn dim_except_header_controls(frame: &mut Frame<'_>, app: &App) {
    let mut preserved = Vec::new();
    for target in [
        HitTarget::HeaderRepository,
        HitTarget::HeaderWorktrees,
        HitTarget::HeaderBranch,
        HitTarget::HeaderDiff,
        HitTarget::HeaderIssue,
        HitTarget::HeaderAgent,
        HitTarget::HeaderLocalBuild,
        HitTarget::HeaderFullscreen,
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

fn draw_navigation(frame: &mut Frame<'_>, app: &mut App, area: Rect, profile: LayoutProfile) {
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().canvas)),
        area,
    );
    if let Some(error) = app
        .notice
        .as_deref()
        .filter(|notice| notice_is_error(notice))
    {
        let error = truncate_width(&format!(" ERROR  {error}"), usize::from(area.width));
        app.clear_footer_marquee();
        frame.render_widget(
            Paragraph::new(error).style(
                Style::default()
                    .fg(palette().red)
                    .bg(palette().canvas)
                    .add_modifier(Modifier::BOLD),
            ),
            area,
        );
        return;
    }
    if profile.is_single() {
        app.regions.changes = None;
        app.regions.graph = None;
        app.regions.left_pane_toggle = None;
        app.regions.explorer = None;
        app.regions.settings = None;
        app.regions.help = None;
        if let Some(path) = app.repository().map(|repository| {
            let path = display_path(&repository.root);
            if repository.is_local() || repository.branch.is_empty() {
                path
            } else {
                format!("{path}:{}", repository.branch)
            }
        }) {
            let path = format!(" {path}");
            let width = usize::from(area.width);
            let path = if UnicodeWidthStr::width(path.as_str()) > width {
                let frame = app.footer_marquee_elapsed(&path, width).as_millis()
                    / FOOTER_MARQUEE_STEP.as_millis();
                marquee_window(&path, width, frame as usize)
            } else {
                app.clear_footer_marquee();
                path
            };
            frame.render_widget(
                Paragraph::new(path).style(Style::default().fg(palette().soft)),
                area,
            );
        } else {
            app.clear_footer_marquee();
        }
        return;
    }
    app.clear_footer_marquee();

    let compact = area.width < 100;
    let (left_pane_action, left_pane_label) = if app.agents_pane_visible() {
        (ShortcutAction::ShowChanges, "Changes")
    } else if app.sidebar_pane() == LeftPane::Worktree {
        (ShortcutAction::ShowFiles, "Files")
    } else if app.herdr_available() {
        (ShortcutAction::ShowAgents, "Agents")
    } else {
        (ShortcutAction::ShowChanges, "Changes")
    };
    let search_active = app.visible_view() == View::RepositorySearch;
    let key_label = |action| {
        if search_active {
            String::new()
        } else {
            app.settings.shortcuts.label(action)
        }
    };
    let show_back = (profile.is_single() && app.workspace_detail_open()) || app.graph_commit_open();
    let mut labels = Vec::new();
    if show_back {
        labels.push(("esc".to_owned(), "Back"));
    }
    labels.extend([
        (key_label(ShortcutAction::ToggleGraph), "Git Graph"),
        (key_label(left_pane_action), left_pane_label),
    ]);
    labels.extend([
        (key_label(ShortcutAction::OpenExplorer), "Explorer"),
        (key_label(ShortcutAction::OpenSettings), "Settings"),
        (key_label(ShortcutAction::OpenHelp), "Help"),
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
            let path = display_path(&repository.root);
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
        let active = index == usize::from(show_back) && app.visible_view() == View::Graph;
        let background = active.then_some(palette().raised);
        spans.push(Span::styled(
            " ",
            Style::default().bg(background.unwrap_or(palette().canvas)),
        ));
        spans.push(Span::styled(
            key.as_str(),
            Style::default()
                .fg(palette().orange)
                .bg(background.unwrap_or(palette().canvas))
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
                    .bg(background.unwrap_or(palette().canvas))
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ));
        }
        spans.push(Span::styled(
            " ",
            Style::default().bg(background.unwrap_or(palette().canvas)),
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

    let offset = usize::from(show_back);
    app.regions.changes = show_back.then(|| rects[0]);
    app.regions.graph = rects.get(offset).copied();
    app.regions.left_pane_toggle = rects.get(offset + 1).copied();
    app.regions.explorer = rects.get(offset + 2).copied();
    app.regions.settings = rects.get(offset + 3).copied();
    app.regions.help = rects.get(offset + 4).copied();

    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(start_x, area.y, area.right().saturating_sub(start_x), 1),
    );
}

fn notice_is_error(notice: &str) -> bool {
    let notice = notice.to_ascii_lowercase();
    [
        "could not",
        "cannot",
        "can't",
        "failed",
        "fatal:",
        "error:",
        "not a git",
    ]
    .iter()
    .any(|prefix| notice.starts_with(prefix))
        || [" failed", " error", " unavailable", " disconnected"]
            .iter()
            .any(|marker| notice.contains(marker))
}

fn display_path(path: &Path) -> String {
    let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) else {
        return path.display().to_string();
    };
    let Some(relative) = path.strip_prefix(Path::new(&home)).ok() else {
        return path.display().to_string();
    };
    if relative.as_os_str().is_empty() {
        "~".to_owned()
    } else {
        format!("~/{}", relative.display())
    }
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

fn marquee_window(value: &str, width: usize, frame: usize) -> String {
    let total_width = UnicodeWidthStr::width(value);
    let travel = total_width.saturating_sub(width);
    if travel == 0 || width == 0 {
        return truncate_width(value, width);
    }
    let pause_frames =
        (FOOTER_MARQUEE_PAUSE.as_millis() / FOOTER_MARQUEE_STEP.as_millis()) as usize;
    let cycle = travel * 2 + pause_frames * 2 + 1;
    let frame = frame % cycle;
    let offset = if frame <= travel {
        frame
    } else if frame <= travel + pause_frames {
        travel
    } else if frame <= travel * 2 + pause_frames {
        travel * 2 + pause_frames - frame
    } else {
        0
    };

    let mut skipped = 0;
    let mut used = 0;
    let mut result = String::new();
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if skipped < offset {
            skipped += grapheme_width;
            continue;
        }
        if used + grapheme_width > width {
            break;
        }
        result.push_str(grapheme);
        used += grapheme_width;
    }
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
