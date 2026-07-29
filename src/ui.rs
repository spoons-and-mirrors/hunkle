mod changes;
mod history;
mod overlays;
pub(crate) mod preview;
mod sqlite;
mod text;
mod workspace_panel;

#[cfg(test)]
mod tests;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{
        App, FileDialogKind, GraphHitTarget, HitTarget, LeftPane, MINIMUM_WORKSPACE_PANEL_WIDTH,
        Mode, Regions, View, WorkspacePanelHitTarget, WorkspacePanelPlacement,
    },
    theme::{Palette, load_theme},
};

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
        frame.render_widget(
            Paragraph::new("hunkle needs at least 60 columns and 16 rows\n\nq  quit")
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
    const MINIMUM_MAIN_WIDTH: u16 = 60;
    let panel_available = app.workspace_panel_enabled()
        && content.width
            >= MINIMUM_WORKSPACE_PANEL_WIDTH
                .saturating_add(1)
                .saturating_add(MINIMUM_MAIN_WIDTH);
    app.workspace_panel.set_layout_available(panel_available);
    if !panel_available && app.mode == Mode::WorkspacePanel {
        app.mode = Mode::Normal;
    }
    let main_content = if app.workspace_panel.is_visible() && panel_available {
        let panel_width = app.settings.workspace_panel_width.clamp(
            MINIMUM_WORKSPACE_PANEL_WIDTH,
            content
                .width
                .saturating_sub(1)
                .saturating_sub(MINIMUM_MAIN_WIDTH),
        );
        let main_width = content.width.saturating_sub(panel_width).saturating_sub(1);
        let (panel_area, divider, main) = match app.workspace_panel.placement {
            WorkspacePanelPlacement::Left => {
                let panel = Rect::new(content.x, content.y, panel_width, content.height);
                let divider = Rect::new(panel.right(), content.y, 1, content.height);
                let main = Rect::new(divider.right(), content.y, main_width, content.height);
                (panel, divider, main)
            }
            WorkspacePanelPlacement::Right => {
                let main = Rect::new(content.x, content.y, main_width, content.height);
                let divider = Rect::new(main.right(), content.y, 1, content.height);
                let panel = Rect::new(divider.right(), content.y, panel_width, content.height);
                (panel, divider, main)
            }
            WorkspacePanelPlacement::Off => unreachable!(),
        };
        app.regions.workspace_panel = Some(panel_area);
        let (workspace_section, agent_section) = workspace_panel::section_areas(panel_area);
        app.regions.workspace_panel_workspaces = Some(workspace_section);
        app.regions.workspace_panel_agents = Some(agent_section);
        app.regions.workspace_panel_splitter = Some(divider);
        app.regions.workspace_panel_bounds = Some(content);
        let workspace_panel_hover = match app.hovered_hit_target {
            Some(HitTarget::WorkspacePanel(target)) => Some(target),
            _ => None,
        };
        let show_agent_harness = app.settings.show_agent_harness;
        let loaded_workspace_path = app.repository().map(|repository| repository.root.clone());
        for (target, rect) in workspace_panel::draw(
            frame,
            &mut app.workspace_panel,
            panel_area,
            app.mode == Mode::WorkspacePanel,
            workspace_panel_hover,
            show_agent_harness,
            loaded_workspace_path.as_deref(),
        ) {
            app.regions.register_hit_target(target, rect);
        }
        fill(
            frame,
            divider,
            if app.dragging_workspace_panel_splitter {
                palette().accent
            } else {
                palette().canvas
            },
        );
        main
    } else {
        content
    };
    changes::draw(
        frame,
        app,
        main_content,
        app.view != View::Graph || app.graph_commit_open,
    );
    if app.view == View::Graph && !app.graph_commit_open {
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
            app.session.data(),
            &app.commit_summaries,
            &app.author_filter,
            &mut app.graph_state,
            &mut app.graph_scroll_to_selection,
            graph_area,
        );
        app.regions.graph_table = graph_regions.table;
        for (target, rect) in graph_regions.targets {
            app.regions.register_hit_target(target, rect);
        }
    }
    draw_navigation(frame, app, layout[2]);
    match app.mode {
        Mode::FileSearch => {
            dim(frame);
            let files = app
                .session
                .data()
                .map_or(&[][..], |repo| repo.files.as_slice());
            let regions = overlays::draw_file_search(frame, &mut app.file_search, files);
            app.regions.file_search_overlay = Some(regions.overlay);
            app.regions.file_search_list = Some(regions.list);
        }
        Mode::Explorer => {
            dim(frame);
            for (target, rect) in overlays::draw_explorer(frame, &mut app.workspace_explorer) {
                app.regions.register_hit_target(target, rect);
            }
        }
        Mode::Settings => {
            dim(frame);
            let regions = overlays::draw_settings(
                frame,
                &app.settings,
                app.settings_selection,
                app.fetch_running(),
            );
            app.regions.settings_overlay = Some(regions.overlay);
            app.regions.auto_fetch = Some(regions.auto_fetch);
            app.regions.fetch_interval = Some(regions.fetch_interval);
            app.regions.fetch_interval_down = Some(regions.fetch_interval_down);
            app.regions.fetch_interval_up = Some(regions.fetch_interval_up);
            app.regions.workspace_panel_setting = Some(regions.workspace_panel);
            app.regions.agent_harness_setting = Some(regions.agent_harness);
            app.regions.media_preview_setting = Some(regions.media_preview);
            app.regions.editor_setting = Some(regions.editor);
        }
        Mode::RepositoryBrowser => {
            dim(frame);
            for (target, rect) in
                overlays::draw_repository_browser(frame, &mut app.repository_browser)
            {
                app.regions.register_hit_target(target, rect);
            }
            if let Some(dialog) = &app.repository_browser.branch_delete {
                dim(frame);
                overlays::draw_branch_delete_dialog(frame, dialog);
            }
        }
        Mode::WorktreeManager => {
            dim(frame);
            for (target, rect) in overlays::draw_worktree_manager(frame, &mut app.worktree_manager)
            {
                app.regions.register_hit_target(target, rect);
            }
            if let Some(dialog) = &app.worktree_manager.create_dialog {
                dim(frame);
                overlays::draw_worktree_create_dialog(frame, dialog);
            } else if let Some(dialog) = &app.worktree_manager.remove_dialog {
                dim(frame);
                overlays::draw_worktree_remove_dialog(frame, dialog);
            }
        }
        Mode::AuthorFilter => {
            let anchor = app
                .regions
                .hit_target_rect(HitTarget::Graph(GraphHitTarget::AuthorHeader))
                .unwrap_or(Rect::new(main_content.x, main_content.y, 1, 1));
            for (target, rect) in history::draw_author_filter(frame, anchor, &mut app.author_filter)
            {
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
            app.regions.herdr_prompt_overlay =
                Some(overlays::draw_herdr_prompt(frame, &app.herdr_prompt));
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
            overlays::draw_help(frame);
        }
        Mode::WorkspacePanel => {
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
                let (overlay, targets) =
                    overlays::draw_workspace_presets(frame, &app.workspace_panel);
                app.regions.workspace_presets_overlay = Some(overlay);
                for (target, rect) in targets {
                    app.regions.register_hit_target(target, rect);
                }
            }
        }
        Mode::Normal | Mode::Commit => {}
    }
    finish_selection(frame, app);
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

fn draw_header(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let (path, branch) = app.repository().map_or_else(
        || ("No repository selected".to_owned(), "offline".to_owned()),
        |repo| {
            let branch = if !repo.is_local() && !repo.details_ready {
                "loading".to_owned()
            } else {
                repo.branch.clone()
            };
            (repo.root.display().to_string(), branch)
        },
    );
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().surface_alt)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let repository = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("hunkle");
    let branch_label = format!(" {branch} ");
    let branch_width = UnicodeWidthStr::width(branch_label.as_str())
        .min(usize::from(area.width.saturating_sub(12)));
    let notice_label = app
        .notice
        .as_ref()
        .map_or_else(String::new, |notice| format!("  {notice}"));
    let fixed_width = UnicodeWidthStr::width(repository)
        .saturating_add(UnicodeWidthStr::width(notice_label.as_str()))
        .saturating_add(4);
    let left_width = usize::from(area.width).saturating_sub(branch_width);
    let display_path = truncate_width(&path, left_width.saturating_sub(fixed_width));
    let mut title = vec![
        Span::styled(
            format!("  {repository}"),
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {display_path}"),
            Style::default().fg(palette().faint),
        ),
    ];
    if !notice_label.is_empty() {
        title.push(Span::styled(
            notice_label,
            Style::default().fg(palette().yellow),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(title)),
        Rect::new(area.x, area.y, left_width as u16, 1),
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            truncate_width(&branch_label, branch_width),
            Style::default()
                .fg(palette().accent)
                .bg(palette().surface_alt)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Right),
        Rect::new(
            area.right().saturating_sub(branch_width as u16),
            area.y,
            branch_width as u16,
            1,
        ),
    );
}

fn draw_navigation(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().surface_alt)),
        area,
    );

    let compact = area.width < 100;
    let left_pane_label = if app.view == View::Graph || app.changes.pane == LeftPane::Worktree {
        "Files"
    } else {
        "Changes"
    };
    let mut labels = vec![("Tab", "Git Graph"), ("f", left_pane_label)];
    let show_edit = app.can_edit_selected_file();
    if app.workspace_panel_available() {
        labels.push(("w", "Workspaces"));
    }
    labels.extend([
        ("o", "Explorer"),
        ("W", "Worktrees"),
        ("b", "Branches"),
        ("s", "Settings"),
        ("?", "Help"),
    ]);

    let total_width = labels.iter().fold(0_u16, |width, (key, label)| {
        let label_width = if compact {
            0
        } else {
            UnicodeWidthStr::width(*label) as u16 + 1
        };
        width.saturating_add(UnicodeWidthStr::width(*key) as u16 + label_width + 2)
    });
    let mut spans = Vec::new();
    let start_x = area.right().saturating_sub(total_width).max(area.x);
    if show_edit && start_x > area.x {
        let mut edit = vec![
            Span::raw(" "),
            Span::styled("e", Style::default().fg(palette().orange)),
        ];
        if !compact {
            edit.push(Span::styled(" Edit", Style::default().fg(palette().muted)));
        }
        frame.render_widget(
            Paragraph::new(Line::from(edit)),
            Rect::new(area.x, area.y, start_x.saturating_sub(area.x), 1),
        );
    }
    let mut x = start_x;
    let mut rects = Vec::new();
    for (index, (key, label)) in labels.iter().enumerate() {
        let active = index == 0 && app.view == View::Graph;
        let background = active.then_some(palette().raised);
        spans.push(Span::styled(
            " ",
            Style::default().bg(background.unwrap_or(palette().surface_alt)),
        ));
        spans.push(Span::styled(
            *key,
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
        let width = UnicodeWidthStr::width(*key) as u16
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
    app.regions.worktree_manager = rects.get(3 + offset).copied();
    app.regions.repository_browser = rects.get(4 + offset).copied();
    app.regions.settings = rects.get(5 + offset).copied();
    app.regions.help = rects.get(6 + offset).copied();

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
