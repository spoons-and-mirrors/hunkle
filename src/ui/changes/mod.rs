pub(super) use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph, Wrap},
};
pub(super) use ratatui_image::{Resize, StatefulImage};
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::{
    app::{
        App, ChangesHitTarget, DiffHunkRegion, HitTarget, LeftPane, Mode, ShortcutAction,
        TextInput, View, WorkspacePanelHitTarget,
    },
    git::{Change, Commit, DiffSummary},
    repo_path::{RepoPath, display_os_str},
    tree::{ExplorerRow, WorktreeRow, WorktreeSection},
};

pub(super) use super::{
    fill, palette,
    preview::{PreparedPreview, PreviewInput, take_inline_transmission, take_kitty_transmission},
    text::word_wrapped_height,
    truncate_width, workspace_panel,
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
    if app.repository().is_none() {
        super::draw_empty(frame, area, "Open a repository to inspect its changes");
        return;
    }

    let left_width = app
        .settings
        .worktree_width
        .clamp(24, area.width.saturating_sub(25));
    let columns = [
        Rect::new(area.x, area.y, left_width, area.height),
        Rect::new(
            area.x.saturating_add(left_width).saturating_add(1),
            area.y,
            area.width.saturating_sub(left_width).saturating_sub(1),
            area.height,
        ),
    ];
    app.regions.worktree = Some(columns[0]);
    app.regions.diff = Some(columns[1]);
    app.regions.split_bounds = Some(area);
    app.regions.splitter = Some(Rect::new(columns[0].right(), area.y, 1, area.height));
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
    let worktree_list_y = actions_row.bottom();
    let worktree_list = layout_agents_pane(app, worktree_content, worktree_list_y);
    app.regions.worktree_list = Some(worktree_list);
    app.regions.register_hit_target(
        HitTarget::Changes(app.changes.worktree_background_target()),
        worktree_list,
    );
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
    let stage_label = details_ready.then(|| {
        if worktree_header.width >= 36 {
            format!("Stage all  {} files", repo.changes.len())
        } else {
            "All".to_owned()
        }
    });
    let worktree_title = if !details_ready {
        "CHANGES  …".to_owned()
    } else if worktree_header.width >= 36 {
        format!("CHANGES  {}", repo.changes.len())
    } else {
        "CHANGES".to_owned()
    };
    let files_title = "FILES";
    let worktree_title_width = UnicodeWidthStr::width(worktree_title.as_str());
    let title_width = worktree_title_width + 2 + files_title.len();
    let stage_width = stage_label
        .as_deref()
        .map_or(0, |label| UnicodeWidthStr::width(label) + 3);
    let stage_target_width = worktree_header.width.min(stage_width as u16);
    if details_ready {
        app.regions.register_hit_target(
            HitTarget::Changes(ChangesHitTarget::StageAll),
            Rect::new(
                worktree_header.right().saturating_sub(stage_target_width),
                worktree_header.y,
                stage_target_width,
                1,
            ),
        );
    }
    let stage_padding =
        usize::from(worktree_header.width).saturating_sub(title_width + stage_width);
    let mut header = vec![
        Span::styled(
            worktree_title,
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(files_title, Style::default().fg(palette().faint)),
        Span::raw(" ".repeat(stage_padding)),
    ];
    if let Some(stage_label) = stage_label {
        header.extend([
            Span::styled(
                format!("{stage_label} "),
                Style::default().fg(palette().muted),
            ),
            Span::styled(
                format!("{checkbox} "),
                Style::default()
                    .fg(checkbox_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }
    frame.render_widget(Paragraph::new(Line::from(header)), worktree_header);
    app.regions.register_hit_target(
        HitTarget::Changes(ChangesHitTarget::WorktreeTab),
        Rect::new(
            worktree_header.x,
            worktree_header.y,
            worktree_title_width as u16,
            1,
        ),
    );
    app.regions.register_hit_target(
        HitTarget::Changes(ChangesHitTarget::FilesTab),
        Rect::new(
            worktree_header
                .x
                .saturating_add(worktree_title_width as u16 + 2),
            worktree_header.y,
            files_title.len() as u16,
            1,
        ),
    );
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
        return;
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
    draw_commit_editor(
        frame,
        app,
        commit_area,
        actions_row,
        local_workspace,
        has_changes,
        details_ready,
    );
}

#[cfg(test)]
#[cfg(test)]
mod tests;
