pub(super) use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, List, ListItem, Paragraph, Wrap},
};
pub(super) use ratatui_image::{Image as TerminalImage, Resize, StatefulImage};
pub(super) use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) use crate::{
    app::{
        App, ChangesHitTarget, DiffHunkRegion, HitTarget, LeftPane, Mode, PreviewOrigin,
        ScrollTarget, ShortcutAction, TextInput, View,
    },
    git::{Change, Commit, DiffSummary},
    repo_path::{RepoPath, display_os_str},
    tree::{ExplorerRow, WorktreeRow, WorktreeSection},
};

pub(super) use super::{
    fill, palette,
    preview::{
        MediaRenderState, PreparedPreview, PreviewInput, take_inline_transmission,
        take_kitty_transmission,
    },
    text::word_wrapped_height,
    text_input_lines, truncate_width,
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

pub(super) enum ChangesPlan {
    SingleMaster {
        area: Rect,
        pane: LeftPane,
    },
    SinglePreview {
        area: Rect,
        pane: LeftPane,
    },
    Columns {
        files_area: Rect,
        editor_area: Rect,
        changes_area: Rect,
        preview_pane: Option<LeftPane>,
    },
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, plan: ChangesPlan) {
    match plan {
        ChangesPlan::SingleMaster { area, pane } => {
            app.reset_media_presentation();
            draw_master(frame, app, area, pane, None);
        }
        ChangesPlan::SinglePreview { area, pane } => {
            draw_detail(frame, app, area, pane, true);
        }
        ChangesPlan::Columns {
            files_area,
            editor_area,
            changes_area,
            preview_pane,
        } => {
            draw_explorer_master(frame, app, files_area, false);
            if let Some(preview_pane) = preview_pane {
                draw_detail(frame, app, editor_area, preview_pane, false);
            }
            draw_master(
                frame,
                app,
                changes_area,
                LeftPane::Worktree,
                Some(editor_area),
            );
        }
    }
}

fn draw_master(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    pane: LeftPane,
    detail_area: Option<Rect>,
) {
    let single_panel = detail_area.is_none();
    let workspace = detail_area.map_or(area, |detail| {
        Rect::new(
            area.x.min(detail.x),
            area.y,
            detail
                .right()
                .max(area.right())
                .saturating_sub(area.x.min(detail.x)),
            area.height,
        )
    });
    if app.repository().is_none() {
        super::draw_empty(frame, workspace, "Open a repository to inspect its changes");
        return;
    }

    app.regions.worktree = Some(area);
    app.regions.split_bounds = detail_area.map(|_| workspace);
    app.regions.splitter =
        detail_area.map(|_| Rect::new(area.x.saturating_sub(1), area.y, 1, area.height));
    frame.render_widget(Clear, area);
    app.regions.clear_targets_in(area);
    app.regions.worktree_list = None;
    app.regions.commit = None;
    app.regions.actions = None;
    fill(frame, area, palette().panel);
    if app.dragging_splitter {
        fill(
            frame,
            Rect::new(area.x.saturating_sub(1), area.y, 1, area.height),
            palette().accent,
        );
    }
    if pane == LeftPane::Files {
        draw_explorer_master(frame, app, area, single_panel);
        return;
    }

    let worktree_content = area.inner(Margin::new(1, 0));
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
    app.regions
        .register_scroll_target(ScrollTarget::Commit, commit_area);
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
    let worktree_list = Rect::new(
        worktree_content.x,
        worktree_list_y,
        worktree_content.width,
        worktree_content.bottom().saturating_sub(worktree_list_y),
    );
    app.regions.worktree_list = Some(worktree_list);
    app.regions
        .register_scroll_target(ScrollTarget::Worktree, worktree_list);
    app.regions.register_hit_target(
        HitTarget::Changes(app.changes.worktree_background_target()),
        worktree_list,
    );
    if single_panel {
        draw_sidebar_tabs(frame, app, worktree_header, pane);
    } else {
        let active = app.sidebar_pane() == LeftPane::Worktree;
        frame.render_widget(
            Paragraph::new("CHANGES").style(
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
            ),
            worktree_header,
        );
        app.regions.register_hit_target(
            HitTarget::Changes(ChangesHitTarget::WorktreeTab),
            worktree_header,
        );
    }
    let repo = app.session.data().expect("checked above");
    let local_workspace = repo.is_local();
    let details_ready = repo.details_ready;
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
                staging_row.x.saturating_add(1),
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
        .saturating_sub(UnicodeWidthStr::width(files_label.as_str()) + stage_width + 1);
    let mut staging = vec![Span::raw(" ")];
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
    staging.push(Span::raw(" ".repeat(stage_padding)));
    staging.push(Span::styled(
        files_label,
        Style::default().fg(palette().faint),
    ));
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
    draw_commit_editor(frame, app, commit_area, local_workspace, details_ready);
}

fn draw_detail(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    pane: LeftPane,
    single_panel: bool,
) {
    if app.repository().is_none() {
        if single_panel {
            super::draw_empty(frame, area, "Open a repository to inspect its changes");
        }
        return;
    }

    if single_panel {
        clear_sidebar_regions(app);
        app.regions.worktree = None;
        app.regions.split_bounds = None;
        app.regions.splitter = None;
    }
    app.regions.diff = Some(area);
    app.regions.clear_targets_in(area);
    app.regions
        .register_scroll_target(ScrollTarget::Preview, area);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);

    if pane == LeftPane::Files {
        draw_explorer_detail(frame, app, area);
        return;
    }

    let repo = app.session.data().expect("checked above");

    let selected_commit = match app.changes.preview.origin() {
        PreviewOrigin::Commit { oid } => app
            .selected_graph_commit()
            .filter(|commit| commit.oid == *oid),
        _ => None,
    };
    let branch_comparison = app.changes.branch_comparison().cloned();
    let selected_section = match app.changes.preview.origin() {
        PreviewOrigin::WorktreeSection(section) => Some(*section),
        _ => None,
    };
    let selected_directory = match app.changes.preview.origin() {
        PreviewOrigin::WorktreeDirectory { section, path } => Some((*section, path)),
        _ => None,
    };
    let selected_change = match app.changes.preview.origin() {
        PreviewOrigin::WorktreeChange { path, staged, .. } => repo
            .changes
            .iter()
            .find(|change| change.path == *path && change.staged == *staged),
        _ => None,
    };
    let selected_label = branch_comparison.as_ref().map_or_else(
        || {
            selected_commit.map_or_else(
                || {
                    selected_change.map_or_else(
                        || {
                            selected_directory.map_or_else(
                                || {
                                    selected_section.map_or_else(
                                        || "No file selected".to_owned(),
                                        |section| match section {
                                            WorktreeSection::Staged => {
                                                "All staged changes".to_owned()
                                            }
                                            WorktreeSection::Unstaged => {
                                                "All unstaged changes".to_owned()
                                            }
                                        },
                                    )
                                },
                                |(_, path)| path.display(),
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
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        1,
    );
    let state = branch_comparison.as_ref().map_or_else(
        || {
            selected_commit.map_or_else(
                || {
                    selected_section
                        .or_else(|| selected_directory.map(|(section, _)| section))
                        .map_or_else(
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
    let show_summary = inspecting_commit
        || selected_section.is_some()
        || selected_directory.is_some()
        || selected_change.is_some();
    let metadata_width = diff_header.width.saturating_sub(2);
    let message_height = selected_commit.map_or(0, |commit| {
        commit_message_height(
            &commit.message,
            metadata_width,
            area.height.saturating_sub(12),
        )
    });
    let summary = selected_commit
        .and_then(|commit| app.commit_summaries.get(&commit.oid))
        .or_else(|| app.changes.selection_summary());
    let summary_unavailable =
        selected_commit.is_some_and(|commit| app.commit_summaries.failed(&commit.oid));
    let scrolled_commit = selected_commit.cloned();
    let scrolled_commit_message = scrolled_commit
        .as_ref()
        .map(|commit| commit.message.clone());
    let scrolled_summary = summary.cloned();
    let maximum_summary_height = area
        .height
        .saturating_sub(8_u16.saturating_add(message_height))
        .min(area.height);
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
    let scrollable_metadata_height = metadata_height.saturating_add(metadata_bottom_margin);
    let diff_body = if inspecting_commit {
        Rect::new(
            diff_header.x,
            area.y.saturating_add(1),
            diff_header.width,
            area.bottom().saturating_sub(area.y.saturating_add(1)),
        )
    } else {
        Rect::new(
            diff_header.x,
            diff_header.y.saturating_add(2),
            diff_header.width,
            area.bottom()
                .saturating_sub(diff_header.y.saturating_add(2)),
        )
    };
    let wrap_label = if !app.changes.preview.wrappable() {
        String::new()
    } else if app.changes.diff_wrap {
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
    let show_hunk_actions = app.changes.preview.hunk_actions();
    let editable_diff = selected_change.map(|change| (change.path.clone(), change.code == '?'));
    let editable_combined_diff = app.changes.preview.editable() && editable_diff.is_none();
    let mut layout = prepare_preview_layout(
        app,
        area,
        diff_body,
        &syntax_path,
        false,
        scrollable_metadata_height,
    );
    let (hunk_rows, rendered_height) = if show_hunk_actions {
        app.changes
            .preview
            .document()
            .map_or((Vec::new(), 0), |document| {
                app.changes
                    .preview_presentation
                    .hunk_rows(document, layout.preview.wrapped)
            })
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
            layout = prepare_preview_layout(
                app,
                area,
                diff_body,
                &syntax_path,
                false,
                scrollable_metadata_height,
            );
        }
    }
    let visible_hunks = visible_hunks(&hunk_rows, rendered_height, &layout);
    if !layout.preview_body.is_empty() {
        if let Some((path, untracked)) = editable_diff {
            app.regions.preview_body = Some(layout.preview_body);
            app.regions.preview_path = Some(path);
            app.regions.preview_untracked = untracked;
            app.regions.preview_generation = app.changes.preview.generation();
            app.regions.preview_scroll = layout.content_scroll;
        } else if editable_combined_diff {
            app.regions.preview_body = Some(layout.preview_body);
            app.regions.preview_generation = app.changes.preview.generation();
            app.regions.preview_scroll = layout.content_scroll;
        }
    }
    if let Some(message) = scrolled_commit_message.as_deref() {
        draw_scrolled_metadata_card(
            frame,
            &layout,
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
    } else if metadata_height > 0 {
        draw_scrolled_summary_card(
            frame,
            &layout,
            metadata_height,
            scrolled_summary.as_ref(),
            summary_unavailable,
            summary_height,
        );
    }
    render_scrollable_content(frame, app, &mut layout);
    draw_hunk_actions(frame, app, &layout, visible_hunks);
}

fn clear_sidebar_regions(app: &mut App) {
    app.regions.worktree = None;
    app.regions.worktree_list = None;
    app.regions.explorer_list = None;
    app.regions.commit = None;
    app.regions.actions = None;
    app.regions.files_add = None;
    app.regions.files_root = None;
}

pub(super) fn draw_sidebar_tabs(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    pane: LeftPane,
) -> Rect {
    let tabs = vec![
        (
            "CHANGES",
            ChangesHitTarget::WorktreeTab,
            pane == LeftPane::Worktree,
        ),
        ("FILES", ChangesHitTarget::FilesTab, pane == LeftPane::Files),
    ];
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

#[cfg(test)]
mod tests;
