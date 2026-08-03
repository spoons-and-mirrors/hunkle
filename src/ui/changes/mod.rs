pub(super) use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, List, ListItem, Paragraph, Wrap},
};
pub(super) use ratatui_image::{Resize, StatefulImage};
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::{
    app::{
        App, ChangesHitTarget, DiffHunkRegion, HitTarget, LeftPane, Mode, ShortcutAction,
        TextInput, View,
    },
    git::{Change, Commit, DiffSummary},
    repo_path::{RepoPath, display_os_str},
    tree::{ExplorerRow, WorktreeRow, WorktreeSection},
};

pub(super) use super::{
    agents, fill, palette,
    preview::{PreparedPreview, PreviewInput, take_inline_transmission, take_kitty_transmission},
    text::word_wrapped_height,
    truncate_width,
};

mod commit_editor;
use commit_editor::*;
mod diff_summary;
use diff_summary::*;
mod explorer;
use explorer::*;
mod hunks;
use hunks::*;
mod layout;
use layout::*;
mod metadata;
use metadata::*;

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect, draw_details: bool) {
    let sidebar_pane = app.changes.pane;
    let preview_pane = app.changes.preview_pane;
    if app.single_panel_layout() {
        if app.agents_pane_visible() {
            app.reset_media_presentation();
            if app.single_panel_detail_visible() {
                draw_agent_history_pane(frame, app, area);
            } else {
                draw_agents_panel(frame, app, area);
            }
        } else if ((app.single_panel_detail_visible() || app.graph_commit_open) && draw_details)
            || app.mode == Mode::FileEdit
        {
            app.changes.pane = preview_pane;
            draw_pane(frame, app, area, true);
            app.changes.pane = sidebar_pane;
        } else {
            app.reset_media_presentation();
            draw_pane(frame, app, area, false);
        }
        return;
    }
    if draw_details && sidebar_pane != preview_pane {
        app.changes.pane = preview_pane;
        draw_pane(frame, app, area, true);
        app.changes.pane = sidebar_pane;
        draw_pane(frame, app, area, false);
        return;
    }
    draw_pane(frame, app, area, draw_details);
}

fn draw_pane(frame: &mut Frame<'_>, app: &mut App, area: Rect, draw_details: bool) {
    if app.repository().is_none() {
        super::draw_empty(frame, area, "Open a repository to inspect its changes");
        return;
    }

    let single_panel = app.single_panel_layout();
    let columns = if single_panel {
        [area, area]
    } else {
        let left_width = app
            .settings
            .worktree_width
            .clamp(24, area.width.saturating_sub(25));
        [
            Rect::new(area.x, area.y, left_width, area.height),
            Rect::new(
                area.x.saturating_add(left_width).saturating_add(1),
                area.y,
                area.width.saturating_sub(left_width).saturating_sub(1),
                area.height,
            ),
        ]
    };
    app.regions.worktree = (!single_panel || !draw_details).then_some(columns[0]);
    app.regions.diff = (!single_panel || draw_details).then_some(columns[1]);
    app.regions.split_bounds = (!single_panel).then_some(area);
    app.regions.splitter =
        (!single_panel).then(|| Rect::new(columns[0].right(), area.y, 1, area.height));
    frame.render_widget(Clear, columns[0]);
    app.regions.clear_hit_targets_in(columns[0]);
    app.regions.worktree_list = None;
    app.regions.explorer_list = None;
    app.regions.commit = None;
    app.regions.actions = None;
    app.regions.files_add = None;
    app.regions.files_root = None;
    fill(frame, columns[0], palette().panel);
    if draw_details {
        fill(frame, columns[1], palette().panel);
    }
    if app.dragging_splitter {
        fill(
            frame,
            Rect::new(columns[0].right(), area.y, 1, area.height),
            palette().accent,
        );
    }
    if app.changes.pane == LeftPane::Files {
        draw_explorer_changes(frame, app, columns, draw_details);
        return;
    }

    let worktree_content = columns[0].inner(Margin::new(1, 0));
    let worktree_header = Rect::new(
        worktree_content.x,
        worktree_content.y.saturating_add(1),
        worktree_content.width,
        1,
    );
    let commit_area = Rect::new(
        worktree_content.x,
        worktree_header.y.saturating_add(2),
        worktree_content.width,
        5,
    );
    app.regions.commit = Some(commit_area);
    let actions_row = Rect::new(
        worktree_content.x,
        commit_area.bottom(),
        worktree_content.width,
        1,
    );
    let staging_row = Rect::new(
        worktree_content.x,
        actions_row.bottom(),
        worktree_content.width,
        1,
    );
    let worktree_list_y = staging_row.bottom();
    let worktree_list = layout_agents_pane(app, worktree_content, worktree_list_y);
    app.regions.worktree_list = Some(worktree_list);
    app.regions.register_hit_target(
        HitTarget::Changes(app.changes.worktree_background_target()),
        worktree_list,
    );
    draw_sidebar_tabs(frame, app, worktree_header);
    let repo = app.session.data().expect("checked above");
    let local_workspace = repo.is_local();
    let details_ready = repo.details_ready;
    let has_changes = !repo.changes.is_empty();
    let staged_count = repo.change_counts.0;
    let checkbox = if !repo.changes.is_empty() && staged_count == repo.changes.len() {
        "◉"
    } else if staged_count > 0 {
        "◐"
    } else {
        "○"
    };
    let checkbox_color = if staged_count == repo.changes.len() && staged_count > 0 {
        palette().green
    } else if staged_count > 0 {
        palette().yellow
    } else {
        palette().muted
    };
    let worktree_len = app.changes.worktree_rows(repo).len();
    let worktree_viewport = usize::from(worktree_list.height);
    app.changes.worktree_scroll = app
        .changes
        .worktree_scroll
        .min(worktree_len.saturating_sub(worktree_viewport));
    if app.changes.worktree_scroll_to_selection
        && worktree_viewport > 0
        && let Some(selected) = app.changes.worktree_state.selected()
    {
        if selected < app.changes.worktree_scroll {
            app.changes.worktree_scroll = selected;
        } else if selected
            >= app
                .changes
                .worktree_scroll
                .saturating_add(worktree_viewport)
        {
            app.changes.worktree_scroll =
                selected.saturating_add(1).saturating_sub(worktree_viewport);
        }
    }
    app.changes.worktree_scroll_to_selection = false;
    let selected_style = Style::default().bg(if app.mode == Mode::Commit {
        palette().inactive_selected
    } else {
        palette().selected
    });
    let items: Vec<ListItem<'_>> = app
        .changes
        .worktree_rows(repo)
        .iter()
        .enumerate()
        .skip(app.changes.worktree_scroll)
        .take(worktree_viewport)
        .map(|(index, row)| {
            let item = worktree_item(row, &repo.changes, worktree_list.width as usize);
            if app.changes.worktree_state.selected() == Some(index) {
                item.style(selected_style)
            } else {
                item
            }
        })
        .collect();
    for (index, row) in app
        .changes
        .worktree_rows(repo)
        .iter()
        .enumerate()
        .skip(app.changes.worktree_scroll)
        .take(worktree_viewport)
    {
        let row_area = Rect::new(
            worktree_list.x,
            worktree_list
                .y
                .saturating_add((index - app.changes.worktree_scroll) as u16),
            worktree_list.width,
            1,
        );
        app.regions.register_hit_target(
            HitTarget::Changes(app.changes.worktree_row_target(index)),
            row_area,
        );
        if row.change_index.is_some() {
            app.regions.register_hit_target(
                HitTarget::Changes(app.changes.worktree_stage_target(index)),
                Rect::new(row_area.right().saturating_sub(2), row_area.y, 2, 1),
            );
        }
    }
    let list = List::new(items);
    let stage_label = details_ready.then_some("STAGE ALL  ");
    let stage_width = stage_label.map_or(0, |label| UnicodeWidthStr::width(label) + 1);
    let stage_target_width = staging_row.width.min(stage_width as u16);
    if details_ready {
        app.regions.register_hit_target(
            HitTarget::Changes(ChangesHitTarget::StageAll),
            Rect::new(
                staging_row.right().saturating_sub(stage_target_width),
                staging_row.y,
                stage_target_width,
                1,
            ),
        );
    }
    let files_label = if details_ready {
        format!("{} FILES", repo.changes.len())
    } else {
        "LOADING CHANGES…".to_owned()
    };
    let stage_padding = usize::from(staging_row.width)
        .saturating_sub(UnicodeWidthStr::width(files_label.as_str()) + stage_width);
    let mut staging = vec![
        Span::styled(files_label, Style::default().fg(palette().faint)),
        Span::raw(" ".repeat(stage_padding)),
    ];
    if let Some(stage_label) = stage_label {
        staging.push(Span::styled(
            stage_label,
            Style::default().fg(palette().muted),
        ));
        staging.push(Span::styled(
            checkbox,
            Style::default()
                .fg(checkbox_color)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(staging)), staging_row);
    frame.render_widget(list, worktree_list);

    app.regions.actions = if local_workspace {
        frame.render_widget(
            Paragraph::new(Line::styled(
                "LOCAL WORKSPACE",
                Style::default().fg(palette().faint),
            )),
            actions_row,
        );
        None
    } else {
        Some(draw_actions(frame, actions_row, app.mode))
    };
    draw_agents_section(frame, app);
    if !draw_details {
        draw_commit_editor(
            frame,
            app,
            commit_area,
            actions_row,
            local_workspace,
            has_changes,
            details_ready,
        );
        draw_agent_history_pane(frame, app, worktree_content);
        return;
    }
    if single_panel {
        clear_sidebar_regions(app);
        app.regions.clear_hit_targets_in(columns[1]);
        frame.render_widget(Clear, columns[1]);
        fill(frame, columns[1], palette().panel);
    }
    let repo = app.session.data().expect("checked above");

    let selected_graph_commit = (app.visible_view() == View::Graph && app.graph_commit_open)
        .then(|| app.selected_graph_commit())
        .flatten();
    let selected_commit = selected_graph_commit;
    let branch_comparison = selected_commit
        .is_none()
        .then(|| app.changes.branch_comparison())
        .flatten()
        .cloned();
    let selected_section = (selected_commit.is_none() && branch_comparison.is_none())
        .then(|| app.changes.selected_diff_section())
        .flatten();
    let selected_change = if selected_commit.is_none() && branch_comparison.is_none() {
        app.changes
            .worktree_state
            .selected()
            .and_then(|index| app.changes.worktree_rows(repo).get(index))
            .and_then(|row| row.change_index)
            .and_then(|index| repo.changes.get(index))
    } else {
        None
    };
    let selected_label = branch_comparison.as_ref().map_or_else(
        || {
            selected_commit.map_or_else(
                || {
                    selected_change.map_or_else(
                        || {
                            selected_section.map_or_else(
                                || "No file selected".to_owned(),
                                |section| match section {
                                    WorktreeSection::Staged => "All staged changes".to_owned(),
                                    WorktreeSection::Unstaged => "All unstaged changes".to_owned(),
                                },
                            )
                        },
                        |change| change.path.display(),
                    )
                },
                |commit| commit.oid.chars().take(7).collect(),
            )
        },
        |_| String::new(),
    );
    let syntax_path = selected_change.map_or_else(String::new, |change| change.path.display());
    let diff_header = Rect::new(
        columns[1].x.saturating_add(1),
        columns[1].y.saturating_add(1),
        columns[1].width.saturating_sub(2),
        1,
    );
    let state = branch_comparison.as_ref().map_or_else(
        || {
            selected_commit.map_or_else(
                || {
                    selected_section.map_or_else(
                        || {
                            selected_change.map_or("", |change| {
                                if change.staged { "staged" } else { "unstaged" }
                            })
                        },
                        |section| match section {
                            WorktreeSection::Staged => "staged",
                            WorktreeSection::Unstaged => "unstaged",
                        },
                    )
                },
                |_| "commit",
            )
        },
        |_| "branch",
    );
    let inspecting_commit = selected_commit.is_some();
    let show_file_headers =
        inspecting_commit || selected_section.is_some() || branch_comparison.is_some();
    let show_summary = inspecting_commit || selected_section.is_some() || selected_change.is_some();
    let metadata_width = diff_header.width.saturating_sub(2);
    let message_height = selected_commit.map_or(0, |commit| {
        commit_message_height(
            &commit.message,
            metadata_width,
            columns[1].height.saturating_sub(12),
        )
    });
    let live_summary = selected_change.map(|change| DiffSummary {
        files: vec![change.path.clone()],
        files_truncated: false,
        additions: change.additions,
        deletions: change.deletions,
    });
    let section_summary = selected_section.map(|section| {
        let staged = section == WorktreeSection::Staged;
        let changes = repo.changes.iter().filter(|change| change.staged == staged);
        DiffSummary {
            files: changes.clone().map(|change| change.path.clone()).collect(),
            files_truncated: false,
            additions: changes.clone().map(|change| change.additions).sum(),
            deletions: changes.map(|change| change.deletions).sum(),
        }
    });
    let summary = selected_commit
        .and_then(|commit| app.commit_summaries.get(&commit.oid))
        .or(live_summary.as_ref())
        .or(section_summary.as_ref());
    let summary_unavailable =
        selected_commit.is_some_and(|commit| app.commit_summaries.failed(&commit.oid));
    let scrolled_commit = selected_commit.cloned();
    let scrolled_commit_message = scrolled_commit
        .as_ref()
        .map(|commit| commit.message.clone());
    let scrolled_summary = summary.cloned();
    let maximum_summary_height = columns[1]
        .height
        .saturating_sub(8_u16.saturating_add(message_height))
        .min(columns[1].height);
    let summary_height = if show_summary {
        diff_summary_height(summary, metadata_width, true, maximum_summary_height)
    } else {
        0
    };
    let metadata_height = if message_height > 0 || summary_height > 0 {
        message_height
            .saturating_add(summary_height)
            .saturating_add(if inspecting_commit { 2 } else { 1 })
    } else {
        0
    };
    let metadata_bottom_margin = u16::from(metadata_height > 0);
    let scrollable_metadata_height = if inspecting_commit {
        metadata_height.saturating_add(metadata_bottom_margin)
    } else {
        0
    };
    let fixed_metadata_height = if inspecting_commit {
        0
    } else {
        metadata_height.saturating_add(metadata_bottom_margin)
    };
    let diff_body = if inspecting_commit {
        Rect::new(
            diff_header.x,
            columns[1].y.saturating_add(1),
            diff_header.width,
            columns[1]
                .bottom()
                .saturating_sub(columns[1].y.saturating_add(1)),
        )
    } else {
        Rect::new(
            diff_header.x,
            diff_header
                .y
                .saturating_add(2)
                .saturating_add(fixed_metadata_height),
            diff_header.width,
            columns[1].bottom().saturating_sub(
                diff_header
                    .y
                    .saturating_add(3)
                    .saturating_add(fixed_metadata_height),
            ),
        )
    };
    let wrap_label = if app.changes.diff_wrap {
        format!(
            "  {}:on",
            app.settings.shortcuts.label(ShortcutAction::ToggleWrap)
        )
    } else {
        format!(
            "  {}:off",
            app.settings.shortcuts.label(ShortcutAction::ToggleWrap)
        )
    };
    let display_path = truncate_width(
        &selected_label,
        usize::from(diff_header.width).saturating_sub(
            8 + UnicodeWidthStr::width(state) + UnicodeWidthStr::width(wrap_label.as_str()),
        ),
    );
    if !inspecting_commit {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if branch_comparison.is_some() {
                        "DIFF"
                    } else {
                        "DIFF  "
                    },
                    Style::default()
                        .fg(palette().muted)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    display_path,
                    Style::default()
                        .fg(palette().ink)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {state}"),
                    Style::default().fg(match state {
                        "staged" => palette().green,
                        "branch" => palette().purple,
                        _ => palette().yellow,
                    }),
                ),
                Span::styled(
                    wrap_label,
                    Style::default().fg(if app.changes.diff_wrap {
                        palette().accent
                    } else {
                        palette().faint
                    }),
                ),
            ])),
            diff_header,
        );
    }
    if !inspecting_commit {
        draw_metadata_card(
            frame,
            Rect::new(
                diff_header.x,
                diff_header.y.saturating_add(2),
                diff_header.width,
                metadata_height,
            ),
            summary,
            summary_unavailable,
            summary_height,
        );
    }
    let show_hunk_actions =
        !inspecting_commit && selected_change.is_some_and(|change| !change.staged);
    let editable_diff = selected_change.map(|change| (change.path.clone(), change.code == '?'));
    let editable_combined_diff = selected_section.is_some() || branch_comparison.is_some();
    let mut preview = prepare_preview_lines(
        app,
        diff_body,
        &syntax_path,
        true,
        show_file_headers,
        false,
        scrollable_metadata_height,
    );
    if let Some((path, untracked)) = editable_diff {
        app.regions.preview_body = Some(diff_body);
        app.regions.preview_path = Some(path);
        app.regions.preview_untracked = untracked;
        app.regions.preview_generation = app.changes.preview_content_generation;
        app.regions.preview_scroll = app.changes.diff_scroll;
    } else if editable_combined_diff {
        app.regions.preview_body = Some(diff_body);
        app.regions.preview_generation = app.changes.preview_content_generation;
        app.regions.preview_scroll = app.changes.diff_scroll;
    }
    let (hunk_rows, rendered_height) = if show_hunk_actions {
        app.changes
            .preview_presentation
            .hunk_rows(&app.changes.diff, preview.wrapped)
    } else {
        (Vec::new(), 0)
    };
    let pin_hunk = app.changes.take_hunk_pin_request();
    if pin_hunk
        && let Some(selected) = app.changes.hunk_selection
        && let Some((_, row)) = hunk_rows.iter().find(|(index, _)| *index == selected)
    {
        let old_scroll = app.changes.diff_scroll;
        app.changes.diff_scroll = scroll_to_row(*row, rendered_height);
        if app.changes.diff_scroll != old_scroll {
            preview = prepare_preview_lines(
                app,
                diff_body,
                &syntax_path,
                true,
                show_file_headers,
                false,
                scrollable_metadata_height,
            );
        }
    }
    let visible_hunks = visible_hunks(
        &hunk_rows,
        rendered_height,
        diff_body,
        app.changes.diff_scroll,
    );
    render_scrollable_content(
        frame,
        app,
        columns[1],
        diff_body,
        preview,
        scrollable_metadata_height,
    );
    if let Some(message) = scrolled_commit_message.as_deref() {
        draw_scrolled_metadata_card(
            frame,
            diff_body,
            app.changes.diff_scroll,
            CommitMetadata {
                height: metadata_height,
                commit: scrolled_commit
                    .as_ref()
                    .expect("commit metadata requires a selected commit"),
                message,
                message_height,
                summary: scrolled_summary.as_ref(),
                summary_unavailable,
                summary_height,
            },
        );
    }
    draw_hunk_actions(frame, app, diff_body, visible_hunks);
    if !single_panel {
        draw_commit_editor(
            frame,
            app,
            commit_area,
            actions_row,
            local_workspace,
            has_changes,
            details_ready,
        );
        draw_agent_history_pane(frame, app, worktree_content);
    }
}

fn draw_agents_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    fill(frame, area, palette().panel);
    let content = area.inner(Margin::new(1, 0));
    let tabs = Rect::new(content.x, content.y.saturating_add(1), content.width, 1);
    let header = Rect::new(content.x, tabs.bottom().saturating_add(1), content.width, 1);
    let list = Rect::new(
        content.x,
        header.bottom(),
        content.width,
        content.bottom().saturating_sub(header.bottom()),
    );
    app.regions.worktree = Some(area);
    app.regions.agents_splitter = Some(header);
    app.regions.agents_bounds = Some(list);
    app.regions.agents_list = Some(list);
    draw_sidebar_tabs(frame, app, tabs);
    draw_agents_section(frame, app);
}

fn clear_sidebar_regions(app: &mut App) {
    app.regions.worktree = None;
    app.regions.worktree_list = None;
    app.regions.explorer_list = None;
    app.regions.agents_list = None;
    app.regions.agents_splitter = None;
    app.regions.agents_bounds = None;
    app.regions.commit = None;
    app.regions.actions = None;
    app.regions.files_add = None;
    app.regions.files_root = None;
}

pub(super) fn draw_sidebar_tabs(frame: &mut Frame<'_>, app: &mut App, area: Rect) -> Rect {
    let agents_active = app.agents_pane_visible();
    let mut tabs = vec![
        (
            "CHANGES",
            ChangesHitTarget::WorktreeTab,
            !agents_active && app.changes.pane == LeftPane::Worktree,
        ),
        (
            "FILES",
            ChangesHitTarget::FilesTab,
            !agents_active && app.changes.pane == LeftPane::Files,
        ),
    ];
    if app.herdr_available() {
        tabs.push(("AGENTS", ChangesHitTarget::AgentsTab, agents_active));
    }
    let mut spans = Vec::new();
    let mut x = area.x;
    for (index, (label, target, active)) in tabs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
            x = x.saturating_add(2);
        }
        let width = UnicodeWidthStr::width(label) as u16;
        spans.push(Span::styled(
            label,
            Style::default()
                .fg(if active {
                    palette().muted
                } else {
                    palette().faint
                })
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        app.regions.register_hit_target(
            HitTarget::Changes(target),
            Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1),
        );
        x = x.saturating_add(width);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    let trailing_x = x.saturating_add(2).min(area.right());
    Rect::new(
        trailing_x,
        area.y,
        area.right().saturating_sub(trailing_x),
        area.height,
    )
}

pub(super) fn draw_agent_history_pane(frame: &mut Frame<'_>, app: &mut App, content: Rect) {
    if !app.agents_pane_visible() {
        return;
    }
    let bottom = app
        .regions
        .agents_splitter
        .map_or(content.bottom(), |splitter| splitter.y);
    let pane = Rect::new(
        content.x,
        content.y,
        content.width,
        bottom.saturating_sub(content.y),
    );
    frame.render_widget(Clear, pane);
    fill(frame, pane, palette().panel);
    app.regions.clear_hit_targets_in(pane);
    app.regions.worktree_list = None;
    app.regions.explorer_list = None;
    app.regions.commit = None;
    app.regions.actions = None;
    app.regions.files_add = None;
    app.regions.files_root = None;

    let header = Rect::new(
        pane.x,
        pane.y.saturating_add(1),
        pane.width,
        u16::from(pane.height > 1),
    );
    let tabs_trailing = draw_sidebar_tabs(frame, app, header);
    let history = Rect::new(
        pane.x,
        header.bottom().saturating_add(1),
        pane.width,
        pane.bottom()
            .saturating_sub(header.bottom().saturating_add(1)),
    );
    let Some(index) = app.agents_pane_index() else {
        frame.render_widget(
            Paragraph::new("NO AGENT SELECTED")
                .style(Style::default().fg(palette().faint))
                .alignment(Alignment::Center),
            history,
        );
        return;
    };
    let repository_anchor = app
        .single_panel_layout()
        .then_some(tabs_trailing)
        .filter(|anchor| anchor.width >= 3);
    app.herdr.request_agent_latest_user_message(index);
    let (targets, scroll_max, scroll) = agents::draw_history(
        frame,
        &app.herdr,
        index,
        app.agent_preview_request(index),
        app.agent_preview_transcript_scroll(index),
        app.agent_preview_button_flash(),
        app.agent_preview_picker_open(),
        app.hovered_hit_target.clone(),
        repository_anchor,
        history,
    );
    app.regions.agent_preview_scroll = scroll;
    app.regions.agent_preview_scroll_max = scroll_max;
    for (target, rect) in targets {
        app.regions.register_hit_target(target, rect);
    }
    if let Some((offset, neighbor)) = app.agent_preview_swipe(index) {
        let repository = app.herdr.agent_repository_name(neighbor).unwrap_or("agent");
        slide_agent_preview(frame, history, offset, repository);
    }
}

fn slide_agent_preview(frame: &mut Frame<'_>, area: Rect, offset: i32, neighbor: &str) {
    let maximum = i32::from(area.width / 2);
    let offset = offset.clamp(-maximum, maximum);
    if offset == 0 || area.is_empty() {
        return;
    }
    let width = usize::from(area.width);
    let mut page = Vec::with_capacity(width.saturating_mul(usize::from(area.height)));
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            page.push(frame.buffer_mut().cell((x, y)).cloned().unwrap());
        }
    }

    frame.render_widget(Clear, area);
    fill(frame, area, palette().surface_alt);
    let reveal_width = u16::try_from(offset.unsigned_abs())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let reveal = if offset > 0 {
        Rect::new(area.x, area.y, reveal_width, area.height)
    } else {
        Rect::new(
            area.right().saturating_sub(reveal_width),
            area.y,
            reveal_width,
            area.height,
        )
    };
    let direction = if offset > 0 { "‹" } else { "›" };
    let label = if offset > 0 {
        format!("{direction} {neighbor}")
    } else {
        format!("{neighbor} {direction}")
    };
    frame.render_widget(
        Paragraph::new(truncate_width(&label, usize::from(reveal.width)))
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(palette().accent)
                    .bg(palette().surface_alt)
                    .add_modifier(Modifier::BOLD),
            ),
        Rect::new(
            reveal.x,
            reveal.y.saturating_add(reveal.height / 2),
            reveal.width,
            u16::from(reveal.height > 0),
        ),
    );

    for source_y in 0..usize::from(area.height) {
        for source_x in 0..width {
            let destination_x = i32::try_from(source_x).unwrap_or(i32::MAX) + offset;
            let Ok(destination_x) = usize::try_from(destination_x) else {
                continue;
            };
            if destination_x >= width {
                continue;
            }
            let source = page[source_y * width + source_x].clone();
            if let Some(cell) = frame.buffer_mut().cell_mut((
                area.x
                    .saturating_add(u16::try_from(destination_x).unwrap_or(u16::MAX)),
                area.y
                    .saturating_add(u16::try_from(source_y).unwrap_or(u16::MAX)),
            )) {
                *cell = source;
            }
        }
    }

    let edge_x = if offset > 0 {
        area.x.saturating_add(reveal_width)
    } else {
        area.right().saturating_sub(reveal_width).saturating_sub(1)
    };
    let edge = if offset > 0 { "▌" } else { "▐" };
    for y in area.y..area.bottom() {
        if let Some(cell) = frame.buffer_mut().cell_mut((edge_x, y)) {
            cell.set_symbol(edge).set_fg(palette().accent);
        }
    }
}

#[cfg(test)]
mod tests;
