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
        App, ChangesHitTarget, DiffHunkRegion, HitTarget, LeftPane, Mode, PreviewOrigin,
        ScrollTarget, ShortcutAction, TextInput, View,
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

pub(super) enum ChangesPlan {
    SingleMaster {
        area: Rect,
        pane: LeftPane,
    },
    SinglePreview {
        area: Rect,
        pane: LeftPane,
    },
    SingleAgents {
        area: Rect,
    },
    SingleAgentHistory {
        area: Rect,
    },
    Columns {
        areas: [Rect; 2],
        sidebar_pane: LeftPane,
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
        ChangesPlan::SingleAgents { area } => {
            app.reset_media_presentation();
            draw_agents_panel(frame, app, area);
        }
        ChangesPlan::SingleAgentHistory { area } => {
            app.reset_media_presentation();
            draw_agent_history_pane(frame, app, area, true);
        }
        ChangesPlan::Columns {
            areas,
            sidebar_pane,
            preview_pane,
        } => {
            draw_master(frame, app, areas[0], sidebar_pane, Some(areas[1]));
            if let Some(preview_pane) = preview_pane {
                draw_detail(frame, app, areas[1], preview_pane, false);
            }
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
            area.x,
            area.y,
            detail.right().saturating_sub(area.x),
            area.height,
        )
    });
    if app.repository().is_none() {
        super::draw_empty(frame, workspace, "Open a repository to inspect its changes");
        return;
    }

    app.regions.worktree = Some(area);
    app.regions.split_bounds = detail_area.map(|_| workspace);
    app.regions.splitter = detail_area.map(|_| Rect::new(area.right(), area.y, 1, area.height));
    frame.render_widget(Clear, area);
    app.regions.clear_targets_in(area);
    app.regions.worktree_list = None;
    app.regions.explorer_list = None;
    app.regions.commit = None;
    app.regions.actions = None;
    app.regions.files_add = None;
    app.regions.files_root = None;
    fill(frame, area, palette().panel);
    if app.dragging_splitter {
        fill(
            frame,
            Rect::new(area.right(), area.y, 1, area.height),
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
    let worktree_list = layout_agents_pane(app, worktree_content, worktree_list_y, single_panel);
    app.regions.worktree_list = Some(worktree_list);
    app.regions
        .register_scroll_target(ScrollTarget::Worktree, worktree_list);
    app.regions.register_hit_target(
        HitTarget::Changes(app.changes.worktree_background_target()),
        worktree_list,
    );
    draw_sidebar_tabs(frame, app, worktree_header, pane);
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
    draw_commit_editor(
        frame,
        app,
        commit_area,
        actions_row,
        local_workspace,
        has_changes,
        details_ready,
    );
    draw_agent_history_pane(frame, app, worktree_content, single_panel);
}

fn draw_detail(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    pane: LeftPane,
    single_panel: bool,
) {
    if app.repository().is_none() {
        super::draw_empty(frame, area, "Open a repository to inspect its changes");
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
    let directory_summary = selected_directory.map(|(section, directory)| {
        let staged = section == WorktreeSection::Staged;
        let changes = repo.changes.iter().filter(|change| {
            change.staged == staged && change.path.as_path().starts_with(directory.as_path())
        });
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
        .or(section_summary.as_ref())
        .or(directory_summary.as_ref());
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
                .saturating_sub(diff_header.y.saturating_add(3)),
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
    app.regions
        .register_scroll_target(ScrollTarget::Agents, list);
    draw_sidebar_tabs(frame, app, tabs, app.sidebar_pane());
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

pub(super) fn draw_sidebar_tabs(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    pane: LeftPane,
) -> Rect {
    let agents_active = app.agents_pane_visible();
    let mut tabs = vec![
        (
            "CHANGES",
            ChangesHitTarget::WorktreeTab,
            !agents_active && pane == LeftPane::Worktree,
        ),
        (
            "FILES",
            ChangesHitTarget::FilesTab,
            !agents_active && pane == LeftPane::Files,
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

pub(super) fn draw_agent_history_pane(
    frame: &mut Frame<'_>,
    app: &mut App,
    content: Rect,
    single_panel: bool,
) {
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
    app.regions.clear_targets_in(pane);
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
    let tabs_trailing = draw_sidebar_tabs(frame, app, header, app.sidebar_pane());
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
    let repository_anchor = single_panel
        .then_some(tabs_trailing)
        .filter(|anchor| anchor.width >= 3);
    app.herdr.request_agent_latest_user_message(index);
    let (targets, scroll_max, scroll) = agents::draw_history(
        frame,
        &app.herdr,
        index,
        app.agent_preview_message(index),
        app.agent_preview_transcript_scroll(index),
        app.agent_preview_expanded_requests(index),
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
