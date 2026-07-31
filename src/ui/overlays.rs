use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{git::Branch, repo_path::RepoPath};

use crate::app::{
    ACTION_ITEMS, ActionsState, BranchDeleteDialog, BrowserTab, CommandRecord, CommandStatus,
    Explorer, ExplorerHitTarget, ExplorerTab, FileDialog, FileDialogKind, FileNameAction,
    FileSearch, HerdrPrompt, HitTarget, Issue, PickerAction, PickerEntry, PullRequest, RemoteItems,
    RepositoryBrowser, RepositoryBrowserHitTarget, Settings, SnapshotLoadDialog, SurroundingEntry,
    WorkspaceDeleteDialog, WorkspaceDeleteKind, WorkspacePanel, WorkspacePanelHitTarget,
    WorkspaceRenameDialog, WorkspaceRenameTarget, WorktreeCreateDialog, WorktreeCreateField,
    WorktreeManager, WorktreeManagerHitTarget, WorktreeManagerRow, WorktreeRemoveDialog,
    short_head, worktree_label,
};

use super::{fill, palette, text::word_wrapped_height, truncate_width};

pub(super) struct FileSearchRegions {
    pub(super) overlay: Rect,
    pub(super) list: Rect,
}

pub(super) struct SettingsRegions {
    pub(super) overlay: Rect,
    pub(super) auto_fetch: Rect,
    pub(super) fetch_interval: Rect,
    pub(super) fetch_interval_down: Rect,
    pub(super) fetch_interval_up: Rect,
    pub(super) workspace_panel: Rect,
    pub(super) agent_harness: Rect,
    pub(super) agent_time: Rect,
    pub(super) clear_agent_timings: Rect,
    pub(super) media_preview: Rect,
    pub(super) editor: Rect,
}

pub(super) struct ActionMenuRegions {
    pub(super) overlay: Rect,
    pub(super) list: Rect,
}

pub(super) struct CommandRegions {
    pub(super) overlay: Rect,
    pub(super) output: Rect,
}

pub(super) struct FileDialogRegions {
    pub(super) overlay: Rect,
    pub(super) primary: Rect,
    pub(super) secondary: Rect,
}

pub(super) fn draw_repository_browser(
    frame: &mut Frame<'_>,
    browser: &mut RepositoryBrowser,
) -> Vec<(HitTarget, Rect)> {
    let area = centered_min(frame.area(), 88, 78, 68, 20);
    let mut hit_targets = vec![(
        HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::Overlay),
        area,
    )];
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    hit_targets.extend(draw_explorer_tabs(frame, area, ExplorerTab::Branches));
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    let repository_summary = format!("{} REFS", browser.branches.len());
    let title_width = "BRANCHES  Navigate repository work".len();
    let title_padding = usize::from(inner_width)
        .saturating_sub(title_width + UnicodeWidthStr::width(repository_summary.as_str()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "BRANCHES",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Navigate repository work",
                Style::default().fg(palette().faint),
            ),
            Span::raw(" ".repeat(title_padding)),
            Span::styled(
                repository_summary,
                Style::default()
                    .fg(palette().green)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );

    let cards_width = inner_width.saturating_sub(2);
    let first_width = cards_width / 3;
    let second_width = cards_width / 3;
    let tabs = [
        Rect::new(inner_x, area.y.saturating_add(3), first_width, 3),
        Rect::new(
            inner_x.saturating_add(first_width).saturating_add(1),
            area.y.saturating_add(3),
            second_width,
            3,
        ),
        Rect::new(
            inner_x
                .saturating_add(first_width)
                .saturating_add(second_width)
                .saturating_add(2),
            area.y.saturating_add(3),
            cards_width
                .saturating_sub(first_width)
                .saturating_sub(second_width),
            3,
        ),
    ];
    for (index, rect) in tabs.iter().copied().enumerate() {
        let tab = BrowserTab::ALL[index];
        hit_targets.push((
            HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::Tab(tab)),
            rect,
        ));
        let active = tab == browser.tab;
        fill(
            frame,
            rect,
            if active {
                palette().raised
            } else {
                palette().surface_alt
            },
        );
        if active {
            fill(
                frame,
                Rect::new(rect.x, rect.y, 1, rect.height),
                palette().accent,
            );
        }
        let (label, status, status_color) = match tab {
            BrowserTab::Branches => (
                "BRANCHES",
                format!("{} local and remote", browser.branches.len()),
                palette().green,
            ),
            BrowserTab::PullRequests => {
                let (status, color) = remote_card_status(&browser.pull_requests);
                ("PULL REQUESTS", status, color)
            }
            BrowserTab::Issues => {
                let (status, color) = remote_card_status(&browser.issues);
                ("ISSUES", status, color)
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    label,
                    Style::default()
                        .fg(if active {
                            palette().ink
                        } else {
                            palette().muted
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if active { "  ACTIVE" } else { "" },
                    Style::default()
                        .fg(palette().orange)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            Rect::new(
                rect.x.saturating_add(2),
                rect.y,
                rect.width.saturating_sub(3),
                1,
            ),
        );
        frame.render_widget(
            Paragraph::new(truncate_width(
                &status,
                usize::from(rect.width.saturating_sub(3)),
            ))
            .style(Style::default().fg(status_color)),
            Rect::new(
                rect.x.saturating_add(2),
                rect.y.saturating_add(1),
                rect.width.saturating_sub(3),
                1,
            ),
        );
    }

    let filter_area = Rect::new(inner_x, area.y.saturating_add(7), inner_width, 3);
    fill(frame, filter_area, palette().raised);
    fill(
        frame,
        Rect::new(filter_area.x, filter_area.y, 1, filter_area.height),
        palette().accent,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "FILTER",
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if browser.query.is_empty() {
                    "  TYPE TO SEARCH"
                } else {
                    "  FILTERING"
                },
                Style::default()
                    .fg(palette().orange)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(
            filter_area.x.saturating_add(2),
            filter_area.y,
            filter_area.width.saturating_sub(4),
            1,
        ),
    );
    let filter_placeholder = match browser.tab {
        BrowserTab::Branches => "branch, upstream, commit or subject",
        BrowserTab::PullRequests => "title, branch, author or number",
        BrowserTab::Issues => "title, label, author or number",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if browser.query.is_empty() {
                    filter_placeholder
                } else {
                    browser.query.as_str()
                },
                Style::default().fg(if browser.query.is_empty() {
                    palette().faint
                } else {
                    palette().ink
                }),
            ),
            Span::styled("▌", Style::default().fg(palette().accent)),
        ])),
        Rect::new(
            filter_area.x.saturating_add(2),
            filter_area.y.saturating_add(1),
            filter_area.width.saturating_sub(4),
            1,
        ),
    );

    let result_indices = browser.result_indices();
    let result_count = result_indices.len();
    let section_label = match browser.tab {
        BrowserTab::Branches => "LOCAL & REMOTE",
        BrowserTab::PullRequests => "OPEN PULL REQUESTS",
        BrowserTab::Issues => "OPEN ISSUES",
    };
    let (result_summary, result_color) = match browser.tab {
        BrowserTab::Branches => (format!("{result_count} shown"), palette().faint),
        BrowserTab::PullRequests => remote_result_summary(&browser.pull_requests, result_count),
        BrowserTab::Issues => remote_result_summary(&browser.issues, result_count),
    };
    let panes = Layout::horizontal([
        Constraint::Percentage(58),
        Constraint::Length(2),
        Constraint::Min(24),
    ])
    .split(Rect::new(
        inner_x,
        area.y.saturating_add(11),
        inner_width,
        area.bottom().saturating_sub(1).saturating_sub(area.y + 11),
    ));
    let list_title = Rect::new(panes[0].x, panes[0].y, panes[0].width, 1);
    let details_title = Rect::new(panes[2].x, panes[2].y, panes[2].width, 1);
    let list = Rect::new(
        panes[0].x,
        panes[0].y.saturating_add(2),
        panes[0].width,
        panes[0].height.saturating_sub(2),
    );
    let details = Rect::new(
        panes[2].x,
        panes[2].y.saturating_add(2),
        panes[2].width,
        panes[2].height.saturating_sub(2),
    );
    let divider = Rect::new(
        panes[1].x.saturating_add(panes[1].width / 2),
        panes[1].y,
        1,
        panes[1].height,
    );
    frame.render_widget(
        Paragraph::new("│\n".repeat(usize::from(divider.height)))
            .style(Style::default().fg(palette().surface_alt)),
        divider,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                section_label,
                Style::default()
                    .fg(palette().orange)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {result_summary}"),
                Style::default().fg(result_color),
            ),
        ])),
        list_title,
    );
    let details_label = match browser.tab {
        BrowserTab::Branches => "BRANCH DETAILS",
        BrowserTab::PullRequests => "PULL REQUEST",
        BrowserTab::Issues => "ISSUE DETAILS",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                details_label,
                Style::default()
                    .fg(if browser.state.selected().is_some() {
                        palette().orange
                    } else {
                        palette().muted
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  selected", Style::default().fg(palette().faint)),
        ])),
        details_title,
    );
    let selected = browser.state.selected();
    let items: Vec<ListItem<'_>> = match browser.tab {
        BrowserTab::Branches => {
            if result_indices.is_empty() {
                vec![status_row(
                    if browser.query.is_empty() {
                        "No branches found"
                    } else {
                        "No branches match this filter"
                    },
                    palette().muted,
                )]
            } else {
                result_indices
                    .iter()
                    .filter_map(|index| browser.branches.get(*index))
                    .enumerate()
                    .map(|(row, branch)| {
                        branch_browser_row(branch, usize::from(list.width), selected == Some(row))
                    })
                    .collect()
            }
        }
        BrowserTab::PullRequests => {
            if let Some(pull_requests) = browser.pull_requests.items() {
                if result_indices.is_empty() {
                    vec![status_row(
                        if browser.query.is_empty() {
                            "No open pull requests"
                        } else {
                            "No pull requests match this filter"
                        },
                        palette().muted,
                    )]
                } else {
                    result_indices
                        .iter()
                        .filter_map(|index| pull_requests.get(*index))
                        .enumerate()
                        .map(|(row, pull_request)| {
                            pull_request_row(pull_request, selected == Some(row))
                        })
                        .collect()
                }
            } else if browser.pull_requests.is_loading() {
                vec![status_row("Loading pull requests…", palette().muted)]
            } else if let Some(error) = browser.pull_requests.error() {
                vec![status_row(error, palette().red)]
            } else {
                Vec::new()
            }
        }
        BrowserTab::Issues => {
            if let Some(issues) = browser.issues.items() {
                if result_indices.is_empty() {
                    vec![status_row(
                        if browser.query.is_empty() {
                            "No open issues"
                        } else {
                            "No issues match this filter"
                        },
                        palette().muted,
                    )]
                } else {
                    result_indices
                        .iter()
                        .filter_map(|index| issues.get(*index))
                        .enumerate()
                        .map(|(row, issue)| issue_row(issue, selected == Some(row)))
                        .collect()
                }
            } else if browser.issues.is_loading() {
                vec![status_row("Loading issues…", palette().muted)]
            } else if let Some(error) = browser.issues.error() {
                vec![status_row(error, palette().red)]
            } else {
                Vec::new()
            }
        }
    };
    frame.render_stateful_widget(
        List::new(items).highlight_style(Style::default().bg(palette().selected)),
        list,
        &mut browser.state,
    );
    let selected_source_index = selected.and_then(|row| result_indices.get(row)).copied();
    draw_repository_browser_details(frame, browser, selected_source_index, details);
    hit_targets.push((
        HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::List),
        list,
    ));
    let row_height = match browser.tab {
        BrowserTab::Branches => 1,
        BrowserTab::PullRequests | BrowserTab::Issues => 2,
    };
    let mut row_y = list.y;
    for index in browser.state.offset()..result_count {
        let height = row_height.min(list.bottom().saturating_sub(row_y));
        if height == 0 {
            break;
        }
        hit_targets.push((
            HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::Item(index)),
            Rect::new(list.x, row_y, list.width, height),
        ));
        row_y = row_y.saturating_add(row_height);
    }

    let footer = if browser.tab == BrowserTab::Branches {
        key_hint_line(
            &[
                ("Enter", "graph"),
                ("Del", "delete"),
                ("Tab/←→", "section"),
                ("↑↓", "select"),
                ("Ctrl-U", "clear"),
                ("Esc", ""),
            ],
            usize::from(inner_width),
        )
    } else {
        key_hint_line(
            &[
                ("Tab/←→", "section"),
                ("↑↓", "select"),
                ("type", "filter"),
                ("Ctrl-U", "clear"),
                ("Esc", ""),
            ],
            usize::from(inner_width),
        )
    };
    frame.render_widget(
        Paragraph::new(footer).alignment(Alignment::Right),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );

    hit_targets
}

pub(super) fn draw_worktree_manager(
    frame: &mut Frame<'_>,
    manager: &mut WorktreeManager,
) -> Vec<(HitTarget, Rect)> {
    let area = centered_min(frame.area(), 88, 78, 68, 20);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );
    let mut hit_targets = vec![(
        HitTarget::WorktreeManager(WorktreeManagerHitTarget::Overlay),
        area,
    )];
    hit_targets.extend(draw_explorer_tabs(frame, area, ExplorerTab::Worktrees));

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    let summary = format!(
        "  Manage linked checkouts  {} repositories · {} worktrees{}",
        manager.repositories.len(),
        manager.worktree_count(),
        if manager.loading {
            " · refreshing"
        } else {
            ""
        }
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "WORKTREES",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(summary, Style::default().fg(palette().faint)),
        ])),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );
    let filter_area = Rect::new(inner_x, area.y.saturating_add(4), inner_width, 3);
    fill(frame, filter_area, palette().raised);
    fill(
        frame,
        Rect::new(filter_area.x, filter_area.y, 1, filter_area.height),
        palette().accent,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "FILTER",
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if manager.query.is_empty() {
                    "  TYPE TO SEARCH"
                } else {
                    "  FILTERING"
                },
                Style::default()
                    .fg(palette().orange)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(
            filter_area.x.saturating_add(2),
            filter_area.y,
            filter_area.width.saturating_sub(4),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if manager.query.is_empty() {
                    "branch, repository, path or commit"
                } else {
                    manager.query.as_str()
                },
                Style::default().fg(if manager.query.is_empty() {
                    palette().faint
                } else {
                    palette().ink
                }),
            ),
            Span::styled("▌", Style::default().fg(palette().accent)),
        ])),
        Rect::new(
            filter_area.x.saturating_add(2),
            filter_area.y.saturating_add(1),
            filter_area.width.saturating_sub(4),
            1,
        ),
    );

    let panes = Layout::horizontal([
        Constraint::Percentage(58),
        Constraint::Length(2),
        Constraint::Min(24),
    ])
    .split(Rect::new(
        inner_x,
        area.y.saturating_add(8),
        inner_width,
        area.bottom().saturating_sub(1).saturating_sub(area.y + 8),
    ));
    let list_title = Rect::new(panes[0].x, panes[0].y, panes[0].width, 1);
    let details_title = Rect::new(panes[2].x, panes[2].y, panes[2].width, 1);
    let list = Rect::new(
        panes[0].x,
        panes[0].y.saturating_add(2),
        panes[0].width,
        panes[0].height.saturating_sub(2),
    );
    let details = Rect::new(
        panes[2].x,
        panes[2].y.saturating_add(2),
        panes[2].width,
        panes[2].height.saturating_sub(2),
    );
    let divider = Rect::new(
        panes[1].x.saturating_add(panes[1].width / 2),
        panes[1].y,
        1,
        panes[1].height,
    );
    frame.render_widget(
        Paragraph::new("│\n".repeat(usize::from(divider.height)))
            .style(Style::default().fg(palette().surface_alt)),
        divider,
    );
    let rows = manager.rows();
    let shown = rows
        .iter()
        .filter(|row| matches!(row, WorktreeManagerRow::Worktree { .. }))
        .count();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "LINKED CHECKOUTS",
                Style::default()
                    .fg(palette().orange)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {shown} shown"),
                Style::default().fg(palette().faint),
            ),
        ])),
        list_title,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "WORKTREE DETAILS",
                Style::default()
                    .fg(if manager.selected_worktree().is_some() {
                        palette().orange
                    } else {
                        palette().muted
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  selected", Style::default().fg(palette().faint)),
        ])),
        details_title,
    );
    let repository_column = manager
        .repositories
        .iter()
        .map(|repository| UnicodeWidthStr::width(repository.label.as_str()))
        .max()
        .unwrap_or(0)
        .min(usize::from(list.width.saturating_sub(2)) / 3);
    let branch_column = manager
        .repositories
        .iter()
        .flat_map(|repository| repository.worktrees.iter())
        .map(|worktree| UnicodeWidthStr::width(worktree_label(worktree).as_str()))
        .max()
        .unwrap_or(0);
    let branch_budget = branch_column
        .min(usize::from(list.width.saturating_sub(3)).saturating_sub(repository_column) / 2);
    let items = if rows.is_empty() {
        vec![status_row(
            if manager.loading {
                "Loading known repositories…"
            } else if manager.query.is_empty() {
                "No linked Git worktrees found"
            } else {
                "No worktrees match this filter"
            },
            palette().muted,
        )]
    } else {
        rows.iter()
            .enumerate()
            .map(|(row_index, row)| match *row {
                WorktreeManagerRow::Group(repository_index) => {
                    let group = manager.repositories[repository_index].group.as_deref();
                    let label = Line::from(Span::styled(
                        group.unwrap_or("Ungrouped").to_uppercase(),
                        Style::default()
                            .fg(palette().ink)
                            .add_modifier(Modifier::BOLD),
                    ));
                    if row_index == 0 {
                        ListItem::new(label)
                    } else {
                        ListItem::new(vec![Line::raw(""), label])
                    }
                }
                WorktreeManagerRow::Status(repository_index) => {
                    let repository = &manager.repositories[repository_index];
                    let label = truncate_width(&repository.label, repository_column);
                    let label = format!(
                        "{label}{}",
                        " ".repeat(
                            repository_column
                                .saturating_sub(UnicodeWidthStr::width(label.as_str())),
                        )
                    );
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{label}  "),
                            Style::default()
                                .fg(palette().ink)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("UNAVAILABLE  ", Style::default().fg(palette().red)),
                        Span::styled(
                            truncate_width(
                                repository.error.as_deref().unwrap_or_default(),
                                usize::from(list.width).saturating_sub(repository_column + 15),
                            ),
                            Style::default().fg(palette().faint),
                        ),
                    ]))
                    .style(Style::default().bg(palette().surface_alt))
                }
                WorktreeManagerRow::Worktree {
                    repository: repository_index,
                    worktree,
                } => {
                    let repository = &manager.repositories[repository_index];
                    let worktree = &repository.worktrees[worktree];
                    let current = manager.is_current(&worktree.path);
                    let selected = manager
                        .state
                        .selected()
                        .is_some_and(|selected| rows.get(selected) == Some(row));
                    let mut badges = Vec::new();
                    if worktree.locked {
                        badges.push("LOCKED");
                    }
                    if worktree.prunable {
                        badges.push("MISSING");
                    }
                    let badge = if badges.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", badges.join(" · "))
                    };
                    let marker = if current || selected { "▌ " } else { "  " };
                    let badge = truncate_width(&badge, usize::from(list.width / 3));
                    let first_in_repository = row_index == 0
                        || !matches!(
                            rows[row_index - 1],
                            WorktreeManagerRow::Worktree {
                                repository: previous,
                                ..
                            } if previous == repository_index
                        );
                    let repository_label = truncate_width(&repository.label, repository_column);
                    let repository_label_width = UnicodeWidthStr::width(repository_label.as_str());
                    let identity_width = usize::from(list.width.saturating_sub(3))
                        .saturating_sub(repository_column)
                        .saturating_sub(UnicodeWidthStr::width(badge.as_str()));
                    let branch_label = worktree_label(worktree);
                    let branch_label = truncate_width(&branch_label, branch_budget);
                    let branch_padding =
                        branch_budget.saturating_sub(UnicodeWidthStr::width(branch_label.as_str()));
                    let path = worktree.path.display().to_string();
                    let path_budget = identity_width.saturating_sub(branch_budget + 2);
                    let path = truncate_start_width(&path, path_budget);
                    let mut line = vec![
                        Span::styled(
                            marker,
                            Style::default().fg(if current {
                                palette().green
                            } else {
                                palette().accent
                            }),
                        ),
                        Span::styled(
                            if first_in_repository {
                                format!(
                                    "{repository_label}{}",
                                    " ".repeat(
                                        repository_column.saturating_sub(repository_label_width),
                                    )
                                )
                            } else {
                                " ".repeat(repository_column)
                            },
                            Style::default().fg(palette().ink).add_modifier(
                                if first_in_repository {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                },
                            ),
                        ),
                        Span::raw(" "),
                        Span::styled(branch_label, Style::default().fg(palette().accent)),
                        Span::raw(format!("{}  ", " ".repeat(branch_padding))),
                        Span::styled(
                            path,
                            Style::default().fg(if selected || current {
                                palette().soft
                            } else {
                                palette().muted
                            }),
                        ),
                    ];
                    line.push(Span::styled(
                        badge,
                        Style::default()
                            .fg(if current {
                                palette().green
                            } else {
                                palette().purple
                            })
                            .add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ));
                    ListItem::new(Line::from(line)).style(Style::default().bg(if selected {
                        palette().raised
                    } else if current {
                        palette().add_bg
                    } else {
                        palette().surface_alt
                    }))
                }
            })
            .collect::<Vec<_>>()
    };
    frame.render_stateful_widget(List::new(items), list, &mut manager.state);
    draw_worktree_details(frame, manager, details);
    hit_targets.push((
        HitTarget::WorktreeManager(WorktreeManagerHitTarget::List),
        list,
    ));
    let mut row_y = list.y;
    for (index, row) in rows.iter().enumerate().skip(manager.state.offset()) {
        let row_height = if matches!(row, WorktreeManagerRow::Group(_)) && index != 0 {
            2
        } else {
            1
        };
        let remaining = list.bottom().saturating_sub(row_y);
        if remaining < row_height {
            break;
        }
        if matches!(row, WorktreeManagerRow::Worktree { .. }) {
            hit_targets.push((
                HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                    generation: manager.content_generation(),
                    row: index,
                }),
                Rect::new(list.x, row_y, list.width, row_height),
            ));
        }
        row_y = row_y.saturating_add(row_height);
    }

    frame.render_widget(
        Paragraph::new(key_hint_line(
            &[
                ("Enter", "open"),
                ("N", "new"),
                ("Del", "remove"),
                ("↑↓", "select"),
                ("Ctrl-R", "refresh"),
                ("Ctrl-U", "clear"),
                ("Esc", ""),
            ],
            usize::from(inner_width),
        ))
        .alignment(Alignment::Right),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );

    hit_targets
}

fn draw_worktree_details(frame: &mut Frame<'_>, manager: &WorktreeManager, area: Rect) {
    let Some((repository, worktree)) = manager.selected_worktree() else {
        frame.render_widget(explorer_empty_list("Select a worktree to inspect it"), area);
        return;
    };
    let current = manager.is_current(&worktree.path);
    let state = if worktree.prunable {
        Some(("MISSING", palette().red))
    } else if worktree.locked {
        Some(("LOCKED", palette().yellow))
    } else if current {
        None
    } else {
        Some(("AVAILABLE", palette().green))
    };
    let ownership = if manager.is_herdr(&worktree.path) {
        "Managed by Herdr"
    } else {
        "Native Git worktree"
    };
    let revision = worktree
        .head
        .as_deref()
        .map_or_else(|| "unknown".to_owned(), short_head);
    let branch_kind = if worktree.is_bare {
        "Bare repository"
    } else if worktree.is_detached {
        "Detached HEAD"
    } else {
        "Local branch"
    };
    let open = manager.open_protection(worktree);
    let create = manager.create_protection(worktree);
    let remove = manager.remove_protection(worktree);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        if current {
            palette().add_bg
        } else {
            palette().surface_alt
        },
    );
    let mut heading = vec![Span::styled(
        worktree_label(worktree),
        Style::default()
            .fg(palette().ink)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some((label, color)) = state {
        heading.push(Span::styled(
            format!("  {label}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    let mut lines = vec![Line::from(heading)];
    lines.extend([
        Line::from(""),
        detail_label("LOCATION"),
        Line::styled(
            worktree.path.display().to_string(),
            Style::default().fg(palette().ink),
        ),
    ]);
    if area.height >= 15 {
        lines.extend([
            Line::from(""),
            detail_label("REVISION"),
            Line::from(vec![
                Span::styled(revision, Style::default().fg(palette().purple)),
                Span::styled(
                    format!("  {branch_kind}"),
                    Style::default().fg(palette().faint),
                ),
            ]),
            Line::from(""),
            detail_label("REPOSITORY"),
            Line::from(vec![
                Span::styled(
                    repository.label.as_str(),
                    Style::default().fg(palette().ink),
                ),
                Span::styled(
                    format!("  {ownership}"),
                    Style::default().fg(palette().faint),
                ),
            ]),
        ]);
    }
    lines.extend([Line::from(""), detail_label("AVAILABLE ACTIONS")]);
    for (label, protection) in [("OPEN", open), ("NEW", create), ("REMOVE", remove)] {
        let (marker, color, detail) = protection.map_or_else(
            || ("✓", palette().green, "Ready".to_owned()),
            |reason| ("×", palette().faint, reason),
        );
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} {label:<6} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_width(&detail, usize::from(area.width.saturating_sub(9))),
                Style::default().fg(if marker == "✓" {
                    palette().muted
                } else {
                    palette().faint
                }),
            ),
        ]));
    }
    if let Some(reason) = worktree
        .locked_reason
        .as_deref()
        .or(worktree.prunable_reason.as_deref())
    {
        lines.extend([
            Line::from(""),
            Line::styled(
                truncate_width(reason, usize::from(area.width)),
                Style::default().fg(palette().yellow),
            ),
        ]);
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn detail_label(label: &'static str) -> Line<'static> {
    Line::styled(
        label,
        Style::default()
            .fg(palette().muted)
            .add_modifier(Modifier::BOLD),
    )
}

pub(super) fn draw_worktree_create_dialog(frame: &mut Frame<'_>, dialog: &WorktreeCreateDialog) {
    let area = centered_min(frame.area(), 72, 0, 60, 18);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new("CREATE WORKTREE").style(
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("STARTING POINT  ", Style::default().fg(palette().muted)),
            Span::styled(
                format!("{} / {}", dialog.repository_label, dialog.start_label),
                Style::default().fg(palette().ink),
            ),
        ])),
        Rect::new(inner.x, inner.y.saturating_add(2), inner.width, 1),
    );

    let branch_selected = dialog.field == WorktreeCreateField::Branch;
    let path_selected = dialog.field == WorktreeCreateField::Path;
    let branch_area = Rect::new(inner.x, inner.y.saturating_add(4), inner.width, 3);
    let path_area = Rect::new(inner.x, inner.y.saturating_add(8), inner.width, 3);
    fill(
        frame,
        branch_area,
        if branch_selected {
            palette().selected
        } else {
            palette().surface_alt
        },
    );
    if branch_selected {
        fill(
            frame,
            Rect::new(branch_area.x, branch_area.y, 1, branch_area.height),
            palette().accent,
        );
    }
    if path_selected {
        fill(
            frame,
            Rect::new(path_area.x, path_area.y, 1, path_area.height),
            palette().accent,
        );
    }
    fill(
        frame,
        path_area,
        if path_selected {
            palette().selected
        } else {
            palette().surface_alt
        },
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "  BRANCH NAME",
                Style::default().fg(palette().muted),
            )),
            Line::from(Span::styled(
                format!(
                    "  {}{}",
                    dialog.branch,
                    if branch_selected { "▌" } else { "" }
                ),
                Style::default().fg(palette().ink),
            )),
        ]),
        branch_area,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "  DESTINATION",
                Style::default().fg(palette().muted),
            )),
            Line::from(Span::styled(
                truncate_start_width(
                    &format!("  {}{}", dialog.path, if path_selected { "▌" } else { "" }),
                    usize::from(path_area.width),
                ),
                Style::default().fg(palette().ink),
            )),
        ]),
        path_area,
    );
    let (message, style) = dialog.error.as_ref().map_or(
        ("Managed by Herdr", Style::default().fg(palette().purple)),
        |error| (error.as_str(), Style::default().fg(palette().red)),
    );
    frame.render_widget(
        Paragraph::new(message).style(style),
        Rect::new(inner.x, inner.y.saturating_add(12), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(key_hint_line(
            &[
                ("Tab", "field"),
                ("Enter", "continue / create"),
                ("Esc", "cancel"),
            ],
            usize::from(inner.width),
        ))
        .alignment(Alignment::Right),
        Rect::new(inner.x, area.bottom().saturating_sub(2), inner.width, 1),
    );
}

pub(super) fn draw_worktree_remove_dialog(frame: &mut Frame<'_>, dialog: &WorktreeRemoveDialog) {
    let area = centered_min(frame.area(), 68, 0, 56, 15);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new("REMOVE WORKTREE").style(
            Style::default()
                .fg(palette().red)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "This removes the linked checkout from disk.",
                Style::default().fg(palette().muted),
            )),
            Line::from(""),
            detail_label("WORKTREE"),
            Line::from(Span::styled(
                dialog.label.as_str(),
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                dialog.path.display().to_string(),
                Style::default().fg(palette().faint),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Git will refuse if the worktree contains changes.",
                Style::default().fg(palette().yellow),
            )),
        ])
        .wrap(Wrap { trim: false }),
        Rect::new(inner.x, inner.y.saturating_add(3), inner.width, 9),
    );
    frame.render_widget(
        Paragraph::new(key_hint_line(
            &[("Enter / y", "remove"), ("n / Esc", "cancel")],
            usize::from(inner.width),
        ))
        .alignment(Alignment::Right),
        Rect::new(inner.x, area.bottom().saturating_sub(2), inner.width, 1),
    );
}

pub(super) fn draw_branch_delete_dialog(frame: &mut Frame<'_>, dialog: &BranchDeleteDialog) {
    let area = centered_min(frame.area(), 66, 0, 54, 13);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new("DELETE BRANCH").style(
            Style::default()
                .fg(palette().red)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(format!("Delete local branch {}?", dialog.branch))
            .style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    let detail = dialog.remote.as_ref().map_or_else(
        || "This branch has no tracked remote branch.".to_owned(),
        |(remote, branch)| format!("Choose whether to keep or delete {remote}/{branch}."),
    );
    frame.render_widget(
        Paragraph::new(detail).style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new("Force permanently discards unmerged work.")
            .style(Style::default().fg(palette().red)),
        Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1),
    );

    let labels = dialog.remote.as_ref().map_or_else(
        || vec!["Local only".to_owned(), "Force local".to_owned()],
        |(remote, _)| {
            vec![
                "Local only".to_owned(),
                format!("Local + {remote}"),
                format!("Force + {remote}"),
            ]
        },
    );
    let gaps = labels.len().saturating_sub(1) as u16;
    let button_width = 18_u16.min(inner.width.saturating_sub(gaps) / labels.len() as u16);
    let total_width = button_width
        .saturating_mul(labels.len() as u16)
        .saturating_add(gaps);
    let start_x = inner.right().saturating_sub(total_width);
    for (index, label) in labels.into_iter().enumerate() {
        let button = Rect::new(
            start_x.saturating_add(index as u16 * button_width.saturating_add(1)),
            area.y.saturating_add(9),
            button_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(palette().red)
                    .bg(if dialog.choice == index {
                        palette().selected
                    } else {
                        palette().raised
                    }),
            ),
            button,
        );
    }
    frame.render_widget(
        Paragraph::new("←/→ choose   Enter confirm   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(super) fn draw_workspace_delete_dialog(frame: &mut Frame<'_>, dialog: &WorkspaceDeleteDialog) {
    let area = centered_min(frame.area(), 66, 0, 54, 12);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    let (title, prompt, detail, warning, action) = match &dialog.kind {
        WorkspaceDeleteKind::Workspace { pane_count } => {
            let noun = if *pane_count == 1 { "pane" } else { "panes" };
            (
                "CLOSE WORKSPACE",
                format!("Close workspace {}?", dialog.label),
                format!("This closes the workspace and all {pane_count} {noun} inside it."),
                "Processes running in those panes will stop.".to_owned(),
                "Close workspace",
            )
        }
        WorkspaceDeleteKind::Worktree { path, .. } => {
            let path = path.as_deref().map_or_else(
                || "its checkout directory".to_owned(),
                |path| path.display().to_string(),
            );
            (
                "DELETE WORKTREE",
                format!("Delete worktree {}?", dialog.label),
                format!("This removes the linked checkout at {path}."),
                "Uncommitted work will block safe deletion.".to_owned(),
                "Delete worktree",
            )
        }
    };
    frame.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(palette().red)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(&prompt, usize::from(inner.width)))
            .style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(&detail, usize::from(inner.width)))
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(warning).style(Style::default().fg(palette().red)),
        Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1),
    );
    let button = Rect::new(
        inner.right().saturating_sub(18),
        area.y.saturating_add(9),
        18,
        1,
    );
    frame.render_widget(
        Paragraph::new(action)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().red).bg(palette().selected)),
        button,
    );
    frame.render_widget(
        Paragraph::new("Enter confirm   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(super) fn draw_workspace_rename_dialog(frame: &mut Frame<'_>, dialog: &WorkspaceRenameDialog) {
    let area = centered_min(frame.area(), 62, 0, 48, 12);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    let (title, subject) = match &dialog.target {
        WorkspaceRenameTarget::Workspace { .. } => ("RENAME WORKSPACE", "workspace"),
        WorkspaceRenameTarget::Agent { .. } => ("RENAME AGENT", "agent"),
    };
    frame.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(
            &format!("Rename {subject} {}", dialog.original_label),
            usize::from(inner.width),
        ))
        .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    let mut input = dialog.input.text().to_owned();
    if dialog.input.cursor_visible() {
        input.insert(dialog.input.cursor(), '▌');
    }
    frame.render_widget(
        Paragraph::new(truncate_start_width(&input, usize::from(inner.width)))
            .style(Style::default().fg(palette().ink).bg(palette().selected)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    if let Some(error) = &dialog.error {
        frame.render_widget(
            Paragraph::new(truncate_width(error, usize::from(inner.width)))
                .style(Style::default().fg(palette().red)),
            Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1),
        );
    }
    frame.render_widget(
        Paragraph::new("Enter rename   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(super) fn draw_snapshot_load_dialog(frame: &mut Frame<'_>, dialog: &SnapshotLoadDialog) {
    let area = centered_min(frame.area(), 68, 0, 56, 13);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new("LOAD WORKSPACE PRESET").style(
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(
            &format!("Load workspace preset {}?", dialog.name),
            usize::from(inner.width),
        ))
        .style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    let workspace_noun = if dialog.close_count == 1 {
        "workspace"
    } else {
        "workspaces"
    };
    let pane_noun = if dialog.close_pane_count == 1 {
        "pane"
    } else {
        "panes"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Open {}  |  Close {} {} ({} {})  |  Restore {} groups",
            dialog.open_count,
            dialog.close_count,
            workspace_noun,
            dialog.close_pane_count,
            pane_noun,
            dialog.group_count,
        ))
        .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    let warning = if dialog.close_count == 0 {
        "Existing workspaces are reused by directory."
    } else {
        "Processes in closed workspace panes will stop."
    };
    frame.render_widget(
        Paragraph::new(warning).style(Style::default().fg(if dialog.close_count == 0 {
            palette().accent
        } else {
            palette().red
        })),
        Rect::new(inner.x, area.y.saturating_add(8), inner.width, 1),
    );
    let button = Rect::new(
        inner.right().saturating_sub(18),
        area.y.saturating_add(10),
        18,
        1,
    );
    frame.render_widget(
        Paragraph::new("Load preset")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().accent).bg(palette().selected)),
        button,
    );
    frame.render_widget(
        Paragraph::new("Enter confirm   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(super) fn draw_workspace_presets(
    frame: &mut Frame<'_>,
    panel: &WorkspacePanel,
) -> (Rect, Vec<(HitTarget, Rect)>) {
    let item_count = panel.snapshots.len() + 1;
    let desired_height = if panel.snapshot_editing {
        10
    } else {
        u16::try_from(item_count).unwrap_or(u16::MAX).min(7) + 7
    };
    let area = centered_min(frame.area(), 0, 0, 50, desired_height);
    let mut targets = vec![(
        HitTarget::WorkspacePanel(WorkspacePanelHitTarget::PresetOverlay),
        area,
    )];
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "WORKSPACE PRESETS",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {} workspaces · {} groups",
                    panel.workspaces.len(),
                    panel.groups.len()
                ),
                Style::default().fg(palette().faint),
            ),
        ])),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    let section_y = area.y.saturating_add(3);

    if panel.snapshot_editing {
        frame.render_widget(
            Paragraph::new("PRESET NAME").style(
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(inner.x, section_y, inner.width, 1),
        );
        let mut input = panel.snapshot_input.text().to_owned();
        if panel.snapshot_input.cursor_visible() {
            input.insert(panel.snapshot_input.cursor(), '▌');
        }
        frame.render_widget(
            Paragraph::new(format!("  {input}"))
                .style(Style::default().fg(palette().ink).bg(palette().selected)),
            Rect::new(inner.x, section_y.saturating_add(2), inner.width, 1),
        );
        if section_y.saturating_add(4) < area.bottom().saturating_sub(1) {
            frame.render_widget(
                Paragraph::new(panel.snapshot_error.as_deref().unwrap_or(
                    "Using an existing name updates that preset with the current setup.",
                ))
                .style(Style::default().fg(if panel.snapshot_error.is_some() {
                    palette().red
                } else {
                    palette().faint
                })),
                Rect::new(inner.x, section_y.saturating_add(4), inner.width, 1),
            );
        }
        frame.render_widget(
            Paragraph::new("Enter save   Esc back")
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().muted)),
            Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
        );
        return (area, targets);
    }

    frame.render_widget(
        Paragraph::new(format!("SAVED PRESETS  {}", panel.snapshots.len())).style(
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, section_y, inner.width, 1),
    );
    let list_y = section_y.saturating_add(2);
    let list = Rect::new(
        inner.x,
        list_y,
        inner.width,
        area.bottom().saturating_sub(2).saturating_sub(list_y),
    );
    let visible = usize::from(list.height).min(item_count);
    let start = panel
        .snapshot_menu_choice
        .saturating_add(1)
        .saturating_sub(visible)
        .min(item_count.saturating_sub(visible));
    for index in start..start + visible {
        let row = Rect::new(
            list.x,
            list.y + u16::try_from(index - start).unwrap_or(0),
            list.width,
            1,
        );
        let selected = panel.snapshot_menu_choice == index;
        let (label, detail, color, target) = if index == 0 {
            (
                "+  Create preset from current setup".to_owned(),
                String::new(),
                palette().accent,
                WorkspacePanelHitTarget::SaveSnapshot,
            )
        } else {
            let preset = &panel.snapshots[index - 1];
            (
                format!("   {}", preset.name),
                format!(
                    "{} workspaces  ·  {} groups",
                    preset.workspace_count(),
                    preset.group_count()
                ),
                palette().ink,
                WorkspacePanelHitTarget::Snapshot(index - 1),
            )
        };
        let detail_width = UnicodeWidthStr::width(detail.as_str());
        let label = truncate_width(
            &label,
            usize::from(row.width).saturating_sub(detail_width.saturating_add(2)),
        );
        let padding = usize::from(row.width)
            .saturating_sub(UnicodeWidthStr::width(label.as_str()) + detail_width);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, Style::default().fg(color)),
                Span::raw(" ".repeat(padding)),
                Span::styled(detail, Style::default().fg(palette().faint)),
            ]))
            .style(Style::default().bg(if selected {
                palette().selected
            } else {
                palette().panel
            })),
            row,
        );
        targets.push((HitTarget::WorkspacePanel(target), row));
    }
    let status = panel
        .snapshot_error
        .as_deref()
        .unwrap_or("Enter load  n new  u update  Del delete  Esc");
    frame.render_widget(
        Paragraph::new(status)
            .alignment(Alignment::Right)
            .style(Style::default().fg(if panel.snapshot_error.is_some() {
                palette().accent
            } else {
                palette().muted
            })),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
    (area, targets)
}

fn remote_card_status<T>(items: &RemoteItems<T>) -> (String, Color) {
    match (items.count(), items.is_loading(), items.error()) {
        (Some(count), true, _) => (format!("{count} open · refreshing"), palette().muted),
        (Some(count), false, Some(_)) => {
            (format!("{count} cached · refresh failed"), palette().red)
        }
        (Some(count), false, None) => (format!("{count} open"), palette().green),
        (None, true, _) => ("loading from GitHub".to_owned(), palette().muted),
        (None, false, Some(_)) => ("GitHub unavailable".to_owned(), palette().red),
        (None, false, None) => ("not loaded".to_owned(), palette().faint),
    }
}

fn remote_result_summary<T>(items: &RemoteItems<T>, shown: usize) -> (String, Color) {
    if items.count().is_some() {
        if items.is_loading() {
            (format!("{shown} shown · refreshing…"), palette().muted)
        } else if items.error().is_some() {
            (format!("{shown} shown · refresh failed"), palette().red)
        } else {
            (format!("{shown} shown"), palette().faint)
        }
    } else if items.is_loading() {
        ("loading…".to_owned(), palette().muted)
    } else if items.error().is_some() {
        ("unavailable".to_owned(), palette().red)
    } else {
        ("not loaded".to_owned(), palette().faint)
    }
}

fn branch_browser_row(branch: &Branch, width: usize, selected: bool) -> ListItem<'static> {
    let (badge, badge_color) = if branch.current {
        ("CURRENT", palette().green)
    } else if branch.default {
        ("DEFAULT", palette().yellow)
    } else if branch.remote {
        ("REMOTE", palette().purple)
    } else {
        ("LOCAL", palette().green)
    };
    let marker = if branch.current { "● " } else { "  " };
    let label = truncate_width(
        &branch.name,
        width.saturating_sub(UnicodeWidthStr::width(marker) + badge.len() + 2),
    );
    let padding = width.saturating_sub(
        UnicodeWidthStr::width(marker) + UnicodeWidthStr::width(label.as_str()) + badge.len(),
    );
    let color = |default| if selected { palette().ink } else { default };
    ListItem::new(Line::from(vec![
        Span::styled(marker, Style::default().fg(color(palette().green))),
        Span::styled(
            label,
            Style::default()
                .fg(color(palette().ink))
                .add_modifier(if branch.current {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::raw(" ".repeat(padding)),
        Span::styled(
            badge,
            Style::default()
                .fg(color(badge_color))
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .style(Style::default().bg(if branch.current && !selected {
        palette().add_bg
    } else {
        palette().panel
    }))
}

fn issue_row(issue: &Issue, selected: bool) -> ListItem<'static> {
    let color = |default| if selected { palette().ink } else { default };
    let metadata = if issue.labels.is_empty() {
        issue.author.clone()
    } else {
        format!("{}  ·  {}", issue.author, issue.labels)
    };
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                format!("#{:<4}", issue.number),
                Style::default()
                    .fg(color(palette().accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(issue.title.clone(), Style::default().fg(palette().ink)),
        ]),
        Line::from(vec![
            Span::raw("     "),
            Span::styled(metadata, Style::default().fg(color(palette().purple))),
        ]),
    ])
}

fn pull_request_row(pull_request: &PullRequest, selected: bool) -> ListItem<'static> {
    let color = |default| if selected { palette().ink } else { default };
    let mut metadata = vec![
        Span::raw("    "),
        Span::styled(
            pull_request.branch.clone(),
            Style::default().fg(color(palette().cyan)),
        ),
        Span::styled("  by  ", Style::default().fg(color(palette().faint))),
        Span::styled(
            pull_request.author.clone(),
            Style::default().fg(color(palette().purple)),
        ),
    ];
    if pull_request.draft {
        metadata.push(Span::styled(
            "  DRAFT",
            Style::default()
                .fg(color(palette().yellow))
                .add_modifier(Modifier::BOLD),
        ));
    }
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                format!("#{:<4}", pull_request.number),
                Style::default()
                    .fg(color(palette().accent))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                pull_request.title.clone(),
                Style::default().fg(palette().ink),
            ),
        ]),
        Line::from(metadata),
    ])
}

fn draw_repository_browser_details(
    frame: &mut Frame<'_>,
    browser: &RepositoryBrowser,
    selected_source_index: Option<usize>,
    area: Rect,
) {
    let lines = match browser.tab {
        BrowserTab::Branches => {
            let Some(branch) = selected_source_index.and_then(|index| browser.branches.get(index))
            else {
                frame.render_widget(explorer_empty_list("Select a branch to inspect it"), area);
                return;
            };
            let (badge, badge_color) = if branch.current {
                ("CURRENT", palette().green)
            } else if branch.default {
                ("DEFAULT", palette().yellow)
            } else if branch.remote {
                ("REMOTE", palette().purple)
            } else {
                ("LOCAL", palette().green)
            };
            let tracking = if branch.upstream.is_empty() {
                if branch.remote {
                    "Remote reference".to_owned()
                } else {
                    "No upstream configured".to_owned()
                }
            } else {
                branch.upstream.clone()
            };
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        branch.name.clone(),
                        Style::default()
                            .fg(palette().ink)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {badge}"),
                        Style::default()
                            .fg(badge_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                detail_label("LATEST COMMIT"),
                Line::from(vec![
                    Span::styled(
                        short_head(&branch.oid),
                        Style::default().fg(palette().purple),
                    ),
                    Span::styled(
                        format!("  {}", branch.date),
                        Style::default().fg(palette().faint),
                    ),
                ]),
                Line::styled(branch.subject.clone(), Style::default().fg(palette().ink)),
                Line::from(""),
                detail_label("TRACKING"),
                Line::styled(tracking, Style::default().fg(palette().cyan)),
            ];
            if area.height >= 12 {
                lines.extend([
                    Line::from(""),
                    detail_label("ACTION"),
                    Line::styled(
                        if branch.remote {
                            "Enter opens this commit in the graph"
                        } else if branch.current {
                            "Checked out now · deletion protected"
                        } else {
                            "Enter opens graph · Del deletes branch"
                        },
                        Style::default().fg(palette().muted),
                    ),
                ]);
            }
            lines
        }
        BrowserTab::PullRequests => {
            let Some(pull_request) =
                selected_source_index.and_then(|index| browser.pull_requests.items()?.get(index))
            else {
                frame.render_widget(
                    explorer_empty_list("Select a pull request to inspect it"),
                    area,
                );
                return;
            };
            vec![
                Line::from(vec![
                    Span::styled(
                        format!("#{}", pull_request.number),
                        Style::default()
                            .fg(palette().accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if pull_request.draft {
                            "  DRAFT"
                        } else {
                            "  OPEN"
                        },
                        Style::default()
                            .fg(if pull_request.draft {
                                palette().yellow
                            } else {
                                palette().green
                            })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                detail_label("TITLE"),
                Line::styled(
                    pull_request.title.clone(),
                    Style::default().fg(palette().ink),
                ),
                Line::from(""),
                detail_label("HEAD BRANCH"),
                Line::styled(
                    pull_request.branch.clone(),
                    Style::default().fg(palette().cyan),
                ),
                Line::from(""),
                detail_label("AUTHOR"),
                Line::styled(
                    pull_request.author.clone(),
                    Style::default().fg(palette().purple),
                ),
            ]
        }
        BrowserTab::Issues => {
            let Some(issue) =
                selected_source_index.and_then(|index| browser.issues.items()?.get(index))
            else {
                frame.render_widget(explorer_empty_list("Select an issue to inspect it"), area);
                return;
            };
            let labels = if issue.labels.is_empty() {
                "No labels".to_owned()
            } else {
                issue.labels.clone()
            };
            vec![
                Line::from(vec![
                    Span::styled(
                        format!("#{}", issue.number),
                        Style::default()
                            .fg(palette().accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "  OPEN",
                        Style::default()
                            .fg(palette().green)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                detail_label("TITLE"),
                Line::styled(issue.title.clone(), Style::default().fg(palette().ink)),
                Line::from(""),
                detail_label("LABELS"),
                Line::styled(labels, Style::default().fg(palette().purple)),
                Line::from(""),
                detail_label("AUTHOR"),
                Line::styled(issue.author.clone(), Style::default().fg(palette().cyan)),
            ]
        }
    };
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        palette().surface_alt,
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn status_row(message: &str, color: Color) -> ListItem<'_> {
    ListItem::new(Line::styled(message, Style::default().fg(color)))
}

pub(super) fn draw_file_add_popover(
    frame: &mut Frame<'_>,
    anchor: Rect,
    selection: usize,
) -> FileDialogRegions {
    let width = 18.min(frame.area().width.saturating_sub(2));
    let height = 2;
    let minimum_x = frame.area().x.saturating_add(1);
    let maximum_x = frame
        .area()
        .right()
        .saturating_sub(width.saturating_add(1))
        .max(minimum_x);
    let x = anchor
        .right()
        .saturating_sub(width)
        .clamp(minimum_x, maximum_x);
    let below = anchor.bottom();
    let y = if below.saturating_add(height) <= frame.area().bottom() {
        below
    } else {
        anchor.y.saturating_sub(height)
    };
    let overlay = Rect::new(x, y, width, height);
    let primary = Rect::new(x, y, width, 1);
    let secondary = Rect::new(x, y.saturating_add(1), width, 1);
    frame.render_widget(Clear, overlay);
    fill(frame, overlay, palette().raised);
    for (index, (label, area)) in [("New file", primary), ("New folder", secondary)]
        .into_iter()
        .enumerate()
    {
        frame.render_widget(
            Paragraph::new(format!("  {label}")).style(Style::default().fg(palette().ink).bg(
                if selection == index {
                    palette().selected
                } else {
                    palette().raised
                },
            )),
            area,
        );
    }
    FileDialogRegions {
        overlay,
        primary,
        secondary,
    }
}

pub(super) fn draw_file_dialog(frame: &mut Frame<'_>, dialog: &FileDialog) -> FileDialogRegions {
    let area = centered_min(frame.area(), 62, 0, 48, 13);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    let (title, prompt, primary_label, secondary_label, destructive) = match &dialog.kind {
        FileDialogKind::Add { parent } => (
            "ADD TO FILES",
            if parent.is_empty() {
                "Create in the repository root".to_owned()
            } else {
                format!("Create inside {parent}")
            },
            "File",
            "Folder",
            false,
        ),
        FileDialogKind::Name {
            action,
            parent,
            source,
        } => {
            let (title, verb) = match action {
                FileNameAction::CreateFile => ("NEW FILE", "Create"),
                FileNameAction::CreateDirectory => ("NEW FOLDER", "Create"),
                FileNameAction::Rename => ("RENAME", "Rename"),
            };
            let prompt = source.as_ref().map_or_else(
                || {
                    if parent.is_empty() {
                        "Name in repository root".to_owned()
                    } else {
                        format!("Name inside {parent}")
                    }
                },
                |source| format!("Rename {source}"),
            );
            (title, prompt, verb, "Cancel", false)
        }
        FileDialogKind::Delete { path, is_directory } => (
            "CONFIRM DELETE",
            if *is_directory {
                format!(
                    "Permanently delete folder {path} and everything inside it, including ignored files?"
                )
            } else {
                format!("Permanently delete file {path}?")
            },
            "Delete",
            "Cancel",
            true,
        ),
        FileDialogKind::DiscardUnstaged { change } => (
            "DISCARD UNSTAGED CHANGES",
            match change.code {
                '?' => format!("Permanently delete untracked file {}?", change.path),
                'R' => format!(
                    "Discard rename {} → {} and restore the original file?",
                    change
                        .original_path
                        .as_ref()
                        .map_or_else(|| "unknown".to_owned(), |path| path.display()),
                    change.path
                ),
                'C' => format!("Permanently delete untracked copy {}?", change.path),
                _ => format!(
                    "Restore {} from the index? Any staged changes will be preserved.",
                    change.path
                ),
            },
            "Discard",
            "Cancel",
            true,
        ),
    };
    frame.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(if destructive {
                    palette().red
                } else {
                    palette().ink
                })
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(prompt).style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 2),
    );
    if matches!(dialog.kind, FileDialogKind::Name { .. }) {
        let mut input = dialog.input.text().to_owned();
        if dialog.input.cursor_visible() {
            input.insert(dialog.input.cursor(), '▌');
        }
        frame.render_widget(
            Paragraph::new(truncate_start_width(&input, usize::from(inner.width)))
                .style(Style::default().fg(palette().ink).bg(palette().selected)),
            Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1),
        );
        if let Some(error) = &dialog.error {
            frame.render_widget(
                Paragraph::new(truncate_width(error, usize::from(inner.width)))
                    .style(Style::default().fg(palette().red)),
                Rect::new(inner.x, area.y.saturating_add(8), inner.width, 1),
            );
        }
    }
    let button_width = 12_u16.min(inner.width.saturating_sub(1) / 2);
    let secondary = Rect::new(
        inner.right().saturating_sub(button_width),
        area.bottom().saturating_sub(2),
        button_width,
        1,
    );
    let primary = Rect::new(
        secondary.x.saturating_sub(button_width.saturating_add(1)),
        secondary.y,
        button_width,
        1,
    );
    let primary_selected = !matches!(dialog.kind, FileDialogKind::Add { .. }) || dialog.choice == 0;
    frame.render_widget(
        Paragraph::new(primary_label)
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(if destructive {
                        palette().red
                    } else {
                        palette().ink
                    })
                    .bg(if primary_selected {
                        palette().selected
                    } else {
                        palette().raised
                    }),
            ),
        primary,
    );
    frame.render_widget(
        Paragraph::new(secondary_label)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().ink).bg(if !primary_selected {
                palette().selected
            } else {
                palette().raised
            })),
        secondary,
    );
    FileDialogRegions {
        overlay: area,
        primary,
        secondary,
    }
}

pub(super) fn draw_action_menu(
    frame: &mut Frame<'_>,
    anchor: Rect,
    selection: usize,
) -> ActionMenuRegions {
    let width = 38.min(frame.area().width.saturating_sub(2));
    let height = ACTION_ITEMS.len() as u16 + 1;
    let minimum_x = frame.area().x.saturating_add(1);
    let maximum_x = frame
        .area()
        .right()
        .saturating_sub(width.saturating_add(1))
        .max(minimum_x);
    let x = anchor
        .right()
        .saturating_sub(width)
        .clamp(minimum_x, maximum_x);
    let below = anchor.y.saturating_add(1);
    let y = if below.saturating_add(height) <= frame.area().bottom() {
        below
    } else {
        anchor.y.saturating_sub(height)
    };
    let area = Rect::new(x, y, width, height);
    let list = Rect::new(area.x, area.y, area.width, ACTION_ITEMS.len() as u16);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().raised);

    let items = ACTION_ITEMS.iter().enumerate().map(|(index, action)| {
        let detail_width = UnicodeWidthStr::width(action.detail);
        let label = truncate_width(
            action.label,
            usize::from(list.width).saturating_sub(detail_width + 4),
        );
        let padding = usize::from(list.width)
            .saturating_sub(UnicodeWidthStr::width(label.as_str()) + detail_width + 3);
        let item = ListItem::new(Line::from(vec![
            Span::styled(
                if index == selection { " › " } else { "   " },
                Style::default().fg(palette().accent),
            ),
            Span::styled(
                label,
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(padding)),
            Span::styled(action.detail, Style::default().fg(palette().faint)),
        ]));
        if index == selection {
            item.style(Style::default().bg(palette().selected))
        } else {
            item
        }
    });
    frame.render_widget(List::new(items), list);
    frame.render_widget(
        Paragraph::new("Enter run   Esc close")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );

    ActionMenuRegions {
        overlay: area,
        list,
    }
}

pub(super) fn draw_command(frame: &mut Frame<'_>, actions: &mut ActionsState) -> CommandRegions {
    let area = centered_min(frame.area(), 82, 68, 64, 18);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    let (title, status, status_color) = match actions.status {
        CommandStatus::Input => ("GIT COMMAND", "NON-INTERACTIVE".to_owned(), palette().muted),
        CommandStatus::Running => ("COMMAND OUTPUT", "RUNNING".to_owned(), palette().yellow),
        CommandStatus::Complete {
            success: true,
            exit_code,
        } => (
            "COMMAND OUTPUT",
            format!("SUCCESS · exit {}", exit_code.unwrap_or(0)),
            palette().green,
        ),
        CommandStatus::Complete {
            success: false,
            exit_code,
        } => (
            "COMMAND OUTPUT",
            exit_code.map_or_else(
                || "FAILED".to_owned(),
                |code| format!("FAILED · exit {code}"),
            ),
            palette().red,
        ),
    };
    let title_padding = usize::from(inner_width)
        .saturating_sub(UnicodeWidthStr::width(title) + UnicodeWidthStr::width(status.as_str()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                title,
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(title_padding)),
            Span::styled(
                status,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );

    let command_area = Rect::new(inner_x, area.bottom().saturating_sub(5), inner_width, 3);
    let command_editable = actions.status != CommandStatus::Running;
    fill(
        frame,
        command_area,
        if command_editable {
            palette().selected
        } else {
            palette().raised
        },
    );
    if command_editable {
        fill(
            frame,
            Rect::new(command_area.x, command_area.y, 1, command_area.height),
            palette().accent,
        );
    }
    frame.render_widget(
        Paragraph::new(Line::styled(
            "COMMAND",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(
            command_area.x.saturating_add(2),
            command_area.y,
            command_area.width.saturating_sub(4),
            1,
        ),
    );
    let command = if command_editable {
        format!("git {}▌", actions.input)
    } else {
        actions.command.clone()
    };
    frame.render_widget(
        Paragraph::new(truncate_start_width(
            &command,
            usize::from(command_area.width.saturating_sub(4)),
        ))
        .style(Style::default().fg(palette().ink)),
        Rect::new(
            command_area.x.saturating_add(2),
            command_area.y.saturating_add(1),
            command_area.width.saturating_sub(4),
            1,
        ),
    );

    let output = Rect::new(
        inner_x,
        area.y.saturating_add(4),
        inner_width,
        command_area
            .y
            .saturating_sub(area.y.saturating_add(4))
            .saturating_sub(1),
    );
    let lines = command_lines(actions.status, &actions.transcript, &actions.stderr);
    let rendered_height = rendered_height(&lines, usize::from(output.width));
    actions.scroll_max = rendered_height
        .saturating_sub(usize::from(output.height))
        .min(usize::from(u16::MAX)) as u16;
    actions.scroll = actions.scroll.min(actions.scroll_max);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((actions.scroll, 0))
            .style(Style::default().fg(palette().ink)),
        output,
    );

    let footer = match actions.status {
        CommandStatus::Input => "Enter run   Ctrl+U clear   Esc close",
        CommandStatus::Running => "Running in background   Esc close",
        CommandStatus::Complete { .. } => {
            "Type next command   Enter run/re-run   ↑↓ scroll   Esc close"
        }
    };
    frame.render_widget(
        Paragraph::new(footer)
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );

    CommandRegions {
        overlay: area,
        output,
    }
}

pub(super) fn draw_herdr_prompt(frame: &mut Frame<'_>, prompt: &HerdrPrompt) -> Rect {
    let area = centered_min(frame.area(), 70, 0, 56, 12);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    let status = if prompt.sending { "SENDING" } else { "READY" };
    let title_padding = usize::from(inner_width)
        .saturating_sub(UnicodeWidthStr::width("HERDR COMMAND") + UnicodeWidthStr::width(status));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "HERDR COMMAND",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(title_padding)),
            Span::styled(
                status,
                Style::default()
                    .fg(if prompt.sending {
                        palette().yellow
                    } else {
                        palette().green
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );
    frame.render_widget(
        Paragraph::new("Send to the pane directly below Hunkle. A pane is created when needed.")
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner_x, area.y.saturating_add(4), inner_width, 1),
    );

    let input_area = Rect::new(inner_x, area.y.saturating_add(6), inner_width, 3);
    fill(
        frame,
        input_area,
        if prompt.sending {
            palette().raised
        } else {
            palette().selected
        },
    );
    if !prompt.sending {
        fill(
            frame,
            Rect::new(input_area.x, input_area.y, 1, input_area.height),
            palette().accent,
        );
    }
    frame.render_widget(
        Paragraph::new("COMMAND OR PROMPT").style(
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            input_area.x.saturating_add(2),
            input_area.y,
            input_area.width.saturating_sub(4),
            1,
        ),
    );
    let mut input = prompt.input.text().to_owned();
    if !prompt.sending && prompt.input.cursor_visible() {
        input.insert(prompt.input.cursor(), '▌');
    }
    let input = format!("> {input}");
    frame.render_widget(
        Paragraph::new(truncate_start_width(
            &input,
            usize::from(input_area.width.saturating_sub(4)),
        ))
        .style(Style::default().fg(palette().ink)),
        Rect::new(
            input_area.x.saturating_add(2),
            input_area.y.saturating_add(1),
            input_area.width.saturating_sub(4),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new(
            prompt
                .error
                .as_deref()
                .unwrap_or("Herdr sends the text and Return without moving focus from Hunkle."),
        )
        .style(Style::default().fg(if prompt.error.is_some() {
            palette().red
        } else {
            palette().faint
        })),
        Rect::new(inner_x, area.y.saturating_add(9), inner_width, 1),
    );
    frame.render_widget(
        Paragraph::new(if prompt.sending {
            "Sending…   Esc close"
        } else {
            "Enter send   Ctrl+U clear   F1 / Esc close"
        })
        .alignment(Alignment::Right)
        .style(Style::default().fg(palette().muted)),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );
    area
}

fn command_lines<'a>(
    status: CommandStatus,
    transcript: &'a [CommandRecord],
    stderr: &'a str,
) -> Vec<Line<'a>> {
    if status == CommandStatus::Input && transcript.is_empty() {
        return if stderr.is_empty() {
            vec![
                Line::styled(
                    "Run any non-interactive Git command from this repository.",
                    Style::default().fg(palette().ink),
                ),
                Line::raw(""),
                Line::styled(
                    "Examples: status --short · log --oneline -10 · remote -v",
                    Style::default().fg(palette().faint),
                ),
                Line::styled(
                    "Shell pipes and redirects are not interpreted.",
                    Style::default().fg(palette().faint),
                ),
            ]
        } else {
            vec![Line::styled(stderr, Style::default().fg(palette().red))]
        };
    }
    let mut lines = Vec::new();
    for (index, record) in transcript.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        let status = if record.success {
            format!("exit {}", record.exit_code.unwrap_or(0))
        } else {
            record
                .exit_code
                .map_or_else(|| "failed".to_owned(), |code| format!("exit {code}"))
        };
        lines.push(Line::from(vec![
            Span::styled(
                record.command.as_str(),
                Style::default()
                    .fg(palette().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {status}"), Style::default().fg(palette().muted)),
        ]));
        if !record.stdout.is_empty() {
            lines.extend(
                record
                    .stdout
                    .lines()
                    .map(|line| Line::styled(line, Style::default().fg(palette().ink))),
            );
        }
        if !record.stderr.is_empty() {
            lines.extend(
                record
                    .stderr
                    .lines()
                    .map(|line| Line::styled(line, Style::default().fg(palette().red))),
            );
        }
        if record.stdout.is_empty() && record.stderr.is_empty() {
            lines.push(Line::styled(
                "Completed without output.",
                Style::default().fg(palette().faint),
            ));
        }
    }
    if status == CommandStatus::Input && !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(stderr, Style::default().fg(palette().red)));
    }
    if status == CommandStatus::Running {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            "Waiting for Git...",
            Style::default().fg(palette().yellow),
        ));
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "Command completed without output.",
            Style::default().fg(palette().faint),
        ));
    }
    lines
}

fn rendered_height(lines: &[Line<'_>], width: usize) -> usize {
    let width = width.max(1);
    lines
        .iter()
        .map(|line| {
            let content = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            word_wrapped_height(&content, width)
        })
        .sum()
}

pub(super) fn draw_explorer(
    frame: &mut Frame<'_>,
    explorer: &mut Explorer,
) -> Vec<(HitTarget, Rect)> {
    let area = centered_min(frame.area(), 88, 78, 68, 20);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let tab_targets = draw_explorer_tabs(frame, area, ExplorerTab::Explorer);
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    let current_is_repo = explorer.entries.first().is_some_and(|entry| entry.is_repo);
    let location_kind = if current_is_repo {
        "GIT REPOSITORY"
    } else {
        "DIRECTORY"
    };
    let title_width = "EXPLORER  Switch working directory".len();
    let title_padding = usize::from(inner_width)
        .saturating_sub(title_width + UnicodeWidthStr::width(location_kind));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "EXPLORER",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Switch working directory",
                Style::default().fg(palette().faint),
            ),
            Span::raw(" ".repeat(title_padding)),
            Span::styled(
                location_kind,
                Style::default()
                    .fg(if current_is_repo {
                        palette().green
                    } else {
                        palette().muted
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );

    let favorites_row = Rect::new(inner_x, area.y.saturating_add(2), inner_width, 1);
    let mut favorite_targets = Vec::new();
    if explorer.naming_favorite {
        fill(frame, favorites_row, palette().selected);
        let name_width = usize::from(favorites_row.width).saturating_sub(17);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "FAVORITE NAME  ",
                    Style::default()
                        .fg(palette().orange)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_width(&explorer.favorite_name, name_width),
                    Style::default().fg(palette().ink),
                ),
                Span::styled("▌", Style::default().fg(palette().accent)),
            ])),
            favorites_row,
        );
    } else if explorer.favorites.is_empty() {
        frame.render_widget(
            Paragraph::new("Ctrl+F  favorite this directory")
                .style(Style::default().fg(palette().faint)),
            favorites_row,
        );
    } else {
        let mut x = favorites_row.x;
        for (index, favorite) in explorer.favorites.iter().enumerate() {
            let remaining = favorites_row.right().saturating_sub(x);
            if remaining < 5 {
                break;
            }
            let name = truncate_width(
                &favorite.name,
                usize::from(remaining.saturating_sub(4)).min(18),
            );
            let label = format!(" ★ {name} ");
            let width = u16::try_from(UnicodeWidthStr::width(label.as_str()))
                .unwrap_or(u16::MAX)
                .min(remaining);
            let card = Rect::new(x, favorites_row.y, width, 1);
            fill(
                frame,
                card,
                if explorer.favorite_is_current(index) {
                    palette().selected
                } else {
                    palette().raised
                },
            );
            frame.render_widget(
                Paragraph::new(label).style(Style::default().fg(
                    if explorer.favorite_is_current(index) {
                        palette().orange
                    } else {
                        palette().ink
                    },
                )),
                card,
            );
            favorite_targets.push((HitTarget::Explorer(explorer.favorite_target(index)), card));
            x = card.right().saturating_add(1);
        }
    }

    let path_area = Rect::new(inner_x, area.y.saturating_add(4), inner_width, 3);
    fill(
        frame,
        path_area,
        if explorer.editing_path {
            palette().selected
        } else {
            palette().raised
        },
    );
    if explorer.editing_path {
        fill(
            frame,
            Rect::new(path_area.x, path_area.y, 1, path_area.height),
            palette().accent,
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "PATH",
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if explorer.editing_path {
                    "  EDITING"
                } else {
                    ""
                },
                Style::default()
                    .fg(palette().orange)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(
            path_area.x.saturating_add(2),
            path_area.y,
            path_area.width.saturating_sub(4),
            1,
        ),
    );
    let input_area = Rect::new(
        path_area.x.saturating_add(2),
        path_area.y.saturating_add(1),
        path_area.width.saturating_sub(4),
        1,
    );
    if explorer.editing_path {
        let cursor = explorer.path_cursor.min(explorer.path_input.len());
        let (before_cursor, after_cursor) = explorer.path_input.split_at(cursor);
        let cursor_column = UnicodeWidthStr::width(before_cursor);
        let scroll = cursor_column
            .saturating_add(1)
            .saturating_sub(usize::from(input_area.width));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(before_cursor.to_owned(), Style::default().fg(palette().ink)),
                Span::styled("▌", Style::default().fg(palette().accent)),
                Span::styled(after_cursor.to_owned(), Style::default().fg(palette().ink)),
            ]))
            .scroll((0, u16::try_from(scroll).unwrap_or(u16::MAX))),
            input_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(truncate_start_width(
                &explorer.path_input,
                usize::from(input_area.width),
            ))
            .style(Style::default().fg(palette().muted)),
            input_area,
        );
    }

    let list_y = area.y.saturating_add(10);
    let panes_area = Rect::new(
        inner_x,
        area.y.saturating_add(8),
        inner_width,
        area.bottom().saturating_sub(1).saturating_sub(area.y + 8),
    );
    let left_width = explorer.pane_width(inner_width);
    let left_pane = Rect::new(panes_area.x, panes_area.y, left_width, panes_area.height);
    let gutter = Rect::new(left_pane.right(), panes_area.y, 2, panes_area.height);
    let right_pane = Rect::new(
        gutter.right(),
        panes_area.y,
        panes_area
            .width
            .saturating_sub(left_width)
            .saturating_sub(2),
        panes_area.height,
    );
    let left_title = Rect::new(left_pane.x, left_pane.y, left_pane.width, 1);
    let right_title = Rect::new(right_pane.x, right_pane.y, right_pane.width, 1);
    let left_list = Rect::new(
        left_pane.x,
        list_y,
        left_pane.width,
        area.bottom().saturating_sub(1).saturating_sub(list_y),
    );
    let right_list = Rect::new(
        right_pane.x,
        list_y,
        right_pane.width,
        area.bottom().saturating_sub(1).saturating_sub(list_y),
    );
    let divider = Rect::new(gutter.x.saturating_add(1), gutter.y, 1, gutter.height);
    frame.render_widget(
        Paragraph::new("│\n".repeat(usize::from(divider.height))).style(Style::default().fg(
            if explorer.dragging_splitter {
                palette().accent
            } else {
                palette().surface_alt
            },
        )),
        divider,
    );

    let (left_label, left_count, right_label, right_count) = if explorer.editing_path {
        (
            "PATH MATCHES".to_owned(),
            if explorer.searching {
                "indexing…".to_owned()
            } else {
                format!("{} found", explorer.matches.len())
            },
            "LIVE PREVIEW".to_owned(),
            format!("{} inside", explorer.preview_entries.len()),
        )
    } else {
        (
            "AROUND HERE".to_owned(),
            format!("{} places", explorer.surroundings.len()),
            "CONTENTS".to_owned(),
            if explorer.loading {
                "loading…".to_owned()
            } else {
                format!("{} entries", explorer.entries.len())
            },
        )
    };
    for (title_area, label, count, active) in [
        (
            left_title,
            left_label,
            left_count,
            explorer.editing_path || explorer.surroundings_focused,
        ),
        (
            right_title,
            right_label,
            right_count,
            !explorer.editing_path && !explorer.surroundings_focused,
        ),
    ] {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    label,
                    Style::default()
                        .fg(if active {
                            palette().orange
                        } else {
                            palette().muted
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {count}"), Style::default().fg(palette().faint)),
            ])),
            title_area,
        );
    }

    if explorer.editing_path {
        if explorer.matches.is_empty() {
            let message = if explorer.searching {
                "Indexing folders…"
            } else if explorer.path_input.trim().is_empty() {
                "Type a folder or path"
            } else {
                "No matching entries"
            };
            frame.render_widget(explorer_empty_list(message), left_list);
        } else {
            let items = explorer
                .matches
                .iter()
                .map(|entry| explorer_item(entry, usize::from(left_list.width)));
            frame.render_stateful_widget(
                List::new(items).highlight_style(Style::default().bg(palette().selected)),
                left_list,
                &mut explorer.match_state,
            );
        }
        if explorer.preview_entries.is_empty() {
            let message = if explorer.matches.is_empty() {
                "Select a match to inspect it"
            } else {
                "No child entries"
            };
            frame.render_widget(explorer_empty_list(message), right_list);
        } else {
            let preview = explorer
                .preview_entries
                .iter()
                .map(|entry| explorer_item(entry, usize::from(right_list.width)));
            frame.render_widget(List::new(preview), right_list);
        }
    } else {
        if explorer.surroundings.is_empty() {
            let message = if explorer.loading {
                "Reading nearby folders…"
            } else {
                "No surrounding folders"
            };
            frame.render_widget(explorer_empty_list(message), left_list);
        } else {
            let surroundings = explorer
                .surroundings
                .iter()
                .map(|entry| surrounding_item(entry, usize::from(left_list.width)));
            frame.render_stateful_widget(
                List::new(surroundings).highlight_style(Style::default().bg(
                    if explorer.surroundings_focused {
                        palette().selected
                    } else {
                        palette().surface_alt
                    },
                )),
                left_list,
                &mut explorer.surroundings_state,
            );
        }
        if explorer.entries.is_empty() {
            let message = if explorer.loading {
                "Reading directory…"
            } else {
                "No directory entries"
            };
            frame.render_widget(explorer_empty_list(message), right_list);
        } else {
            let items = explorer
                .entries
                .iter()
                .map(|entry| explorer_item(entry, usize::from(right_list.width)));
            frame.render_stateful_widget(
                List::new(items).highlight_style(Style::default().bg(
                    if explorer.surroundings_focused {
                        palette().surface_alt
                    } else {
                        palette().selected
                    },
                )),
                right_list,
                &mut explorer.state,
            );
        }
    }

    let footer = Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1);
    if let Some(error) = &explorer.error {
        frame.render_widget(
            Paragraph::new(truncate_width(error, usize::from(footer.width)))
                .style(Style::default().fg(palette().red)),
            footer,
        );
    } else {
        let hint = if explorer.naming_favorite {
            key_hint_line(
                &[("Enter", "save"), ("Ctrl+U", "clear"), ("Esc", "cancel")],
                usize::from(inner_width),
            )
        } else if explorer.editing_path {
            key_hint_line(
                &[
                    ("Tab", "complete"),
                    ("↑↓", "choose"),
                    ("Ctrl/Alt+BS", "segment"),
                    ("Enter", "open"),
                    ("Esc", ""),
                ],
                usize::from(inner_width),
            )
        } else {
            key_hint_line(
                &[
                    ("Tab", "pane"),
                    ("↑↓", "select"),
                    ("Enter", "open"),
                    ("Ctrl+F", "favorite"),
                    ("type", "path"),
                    ("Esc", ""),
                ],
                usize::from(inner_width),
            )
        };
        frame.render_widget(Paragraph::new(hint).alignment(Alignment::Right), footer);
    }

    let mut targets = vec![
        (HitTarget::Explorer(ExplorerHitTarget::Overlay), area),
        (HitTarget::Explorer(ExplorerHitTarget::Path), path_area),
        (HitTarget::Explorer(ExplorerHitTarget::Splitter), divider),
    ];
    targets.extend(tab_targets);
    targets.extend(favorite_targets);
    if explorer.editing_path {
        targets.push((
            HitTarget::Explorer(ExplorerHitTarget::MatchesPane),
            left_list,
        ));
        targets.push((
            HitTarget::Explorer(ExplorerHitTarget::PreviewPane),
            right_list,
        ));
        let offset = explorer.match_state.offset();
        for index in offset..(offset + usize::from(left_list.height)).min(explorer.matches.len()) {
            targets.push((
                HitTarget::Explorer(explorer.match_target(index)),
                Rect::new(
                    left_list.x,
                    left_list.y + u16::try_from(index - offset).unwrap_or(u16::MAX),
                    left_list.width,
                    1,
                ),
            ));
        }
        for index in 0..usize::from(right_list.height).min(explorer.preview_entries.len()) {
            targets.push((
                HitTarget::Explorer(explorer.preview_target(index)),
                Rect::new(
                    right_list.x,
                    right_list.y + u16::try_from(index).unwrap_or(u16::MAX),
                    right_list.width,
                    1,
                ),
            ));
        }
    } else {
        targets.push((
            HitTarget::Explorer(ExplorerHitTarget::SurroundingsPane),
            left_list,
        ));
        targets.push((
            HitTarget::Explorer(ExplorerHitTarget::EntriesPane),
            right_list,
        ));
        let offset = explorer.surroundings_state.offset();
        for index in
            offset..(offset + usize::from(left_list.height)).min(explorer.surroundings.len())
        {
            targets.push((
                HitTarget::Explorer(explorer.surrounding_target(index)),
                Rect::new(
                    left_list.x,
                    left_list.y + u16::try_from(index - offset).unwrap_or(u16::MAX),
                    left_list.width,
                    1,
                ),
            ));
        }
        let offset = explorer.state.offset();
        for index in offset..(offset + usize::from(right_list.height)).min(explorer.entries.len()) {
            targets.push((
                HitTarget::Explorer(explorer.entry_target(index)),
                Rect::new(
                    right_list.x,
                    right_list.y + u16::try_from(index - offset).unwrap_or(u16::MAX),
                    right_list.width,
                    1,
                ),
            ));
        }
    }
    targets
}

fn draw_explorer_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    active: ExplorerTab,
) -> Vec<(HitTarget, Rect)> {
    let mut x = area.x.saturating_add(2);
    ExplorerTab::ALL
        .into_iter()
        .enumerate()
        .map(|(index, tab)| {
            let label = match tab {
                ExplorerTab::Explorer => "F1  EXPLORER",
                ExplorerTab::Worktrees => "F2  WORKTREES",
                ExplorerTab::Branches => "F3  BRANCHES",
            };
            let width = u16::try_from(UnicodeWidthStr::width(label) + 2).unwrap_or(u16::MAX);
            let rect = Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1);
            let selected = tab == active;
            frame.render_widget(
                Paragraph::new(format!(" {label} ")).style(
                    Style::default()
                        .fg(if selected {
                            palette().accent
                        } else {
                            palette().muted
                        })
                        .bg(if selected {
                            palette().raised
                        } else {
                            palette().surface_alt
                        })
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                rect,
            );
            x = rect.right().saturating_add(u16::from(index == 0));
            (HitTarget::ExplorerTab(tab), rect)
        })
        .collect()
}

pub(super) fn draw_file_search(
    frame: &mut Frame<'_>,
    search: &mut FileSearch,
    files: &[RepoPath],
) -> FileSearchRegions {
    let desired_height = (11 + search.results.len().clamp(1, 13) as u16).clamp(15, 24);
    let area = centered_min(frame.area(), 78, 0, 56, desired_height);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    let count = format!("{} FILES", files.len());
    let title_width = "FIND FILE  Search this repository".len();
    let title_padding = usize::from(inner_width)
        .saturating_sub(title_width + UnicodeWidthStr::width(count.as_str()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "FIND FILE",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Search this repository",
                Style::default().fg(palette().faint),
            ),
            Span::raw(" ".repeat(title_padding)),
            Span::styled(
                count,
                Style::default()
                    .fg(palette().accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );

    let input = Rect::new(inner_x, area.y.saturating_add(4), inner_width, 3);
    fill(frame, input, palette().selected);
    fill(
        frame,
        Rect::new(input.x, input.y, 1, input.height),
        palette().accent,
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "QUERY",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(
            input.x.saturating_add(2),
            input.y,
            input.width.saturating_sub(4),
            1,
        ),
    );
    let query_width = usize::from(input.width.saturating_sub(5));
    let query = truncate_start_width(&search.query, query_width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if query.is_empty() {
                    "Type a filename or path…".to_owned()
                } else {
                    query
                },
                Style::default().fg(if search.query.is_empty() {
                    palette().faint
                } else {
                    palette().ink
                }),
            ),
            Span::styled("▌", Style::default().fg(palette().accent)),
        ])),
        Rect::new(
            input.x.saturating_add(2),
            input.y.saturating_add(1),
            input.width.saturating_sub(4),
            1,
        ),
    );

    let detail = if search.query.trim().is_empty() {
        "start typing".to_owned()
    } else if search.match_count > search.results.len() {
        format!(
            "showing {} of {} matches",
            search.results.len(),
            search.match_count
        )
    } else {
        format!("{} matches", search.match_count)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "RESULTS",
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {detail}"), Style::default().fg(palette().faint)),
        ])),
        Rect::new(inner_x, area.y.saturating_add(8), inner_width, 1),
    );

    let list_y = area.y.saturating_add(10);
    let list = Rect::new(
        inner_x,
        list_y,
        inner_width,
        area.bottom().saturating_sub(1).saturating_sub(list_y),
    );
    if search.results.is_empty() {
        let message = if search.query.trim().is_empty() {
            "Search by filename, path, or multiple words"
        } else {
            "No repository files match that query"
        };
        frame.render_widget(
            List::new([ListItem::new(Line::styled(
                message,
                Style::default().fg(palette().faint),
            ))]),
            list,
        );
    } else {
        let items = search.results.iter().filter_map(|result| {
            files
                .get(result.file_index)
                .map(|path| file_search_item(&path.display(), usize::from(list.width)))
        });
        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().bg(palette().selected)),
            list,
            &mut search.state,
        );
    }

    frame.render_widget(
        Paragraph::new("Enter open   ↑↓ select   Ctrl+U clear   F3 / Esc close")
            .style(Style::default().fg(palette().muted))
            .alignment(Alignment::Right),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );

    FileSearchRegions {
        overlay: area,
        list,
    }
}

fn file_search_item(path: &str, width: usize) -> ListItem<'static> {
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    let available = width.saturating_sub(2);
    let name = truncate_width(name, available);
    let name_width = UnicodeWidthStr::width(name.as_str());
    let parent_width = available.saturating_sub(name_width + 2);
    let parent = truncate_start_width(parent, parent_width);
    let mut spans = vec![
        Span::styled("› ", Style::default().fg(palette().accent)),
        Span::styled(
            name,
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if !parent.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(parent, Style::default().fg(palette().faint)));
    }
    ListItem::new(Line::from(spans))
}

fn explorer_item(entry: &PickerEntry, width: usize) -> ListItem<'static> {
    let (marker, label, detail, color) = match entry.action {
        PickerAction::Open if entry.is_repo => ("● ", entry.label.clone(), "open", palette().green),
        PickerAction::Open => ("○ ", entry.label.clone(), "check", palette().muted),
        PickerAction::OpenFile => ("· ", entry.label.clone(), "file", palette().muted),
        PickerAction::Navigate if entry.label == ".." => {
            ("↑ ", "Parent directory".to_owned(), "", palette().muted)
        }
        PickerAction::Navigate if entry.is_repo => {
            ("◆ ", entry.label.clone(), "repository", palette().green)
        }
        PickerAction::Navigate => ("› ", entry.label.clone(), "", palette().faint),
    };
    let detail_width = usize::from(!detail.is_empty()) + UnicodeWidthStr::width(detail);
    let label_width = width.saturating_sub(2 + detail_width);
    let label = truncate_width(&label, label_width);
    let padding = width.saturating_sub(2 + UnicodeWidthStr::width(label.as_str()) + detail_width);
    let mut spans = vec![
        Span::styled(marker, Style::default().fg(color)),
        Span::styled(label, Style::default().fg(palette().ink)),
        Span::raw(" ".repeat(padding)),
    ];
    if !detail.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            detail.to_owned(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    ListItem::new(Line::from(spans))
}

fn surrounding_item(entry: &SurroundingEntry, width: usize) -> ListItem<'static> {
    let indent = " ".repeat(entry.depth.min(4));
    let marker = if entry.current { "● " } else { "├ " };
    let label_width = width.saturating_sub(UnicodeWidthStr::width(indent.as_str()) + 2);
    let label = truncate_width(&entry.label, label_width);
    let padding = width.saturating_sub(
        UnicodeWidthStr::width(indent.as_str()) + 2 + UnicodeWidthStr::width(label.as_str()),
    );
    ListItem::new(Line::from(vec![
        Span::raw(indent),
        Span::styled(
            marker,
            Style::default().fg(if entry.current {
                palette().orange
            } else {
                palette().faint
            }),
        ),
        Span::styled(label, Style::default().fg(palette().ink)),
        Span::raw(" ".repeat(padding)),
    ]))
}

fn explorer_empty_list(message: &'static str) -> List<'static> {
    List::new([ListItem::new(Line::styled(
        format!("  {message}"),
        Style::default().fg(palette().faint),
    ))])
}

fn key_hint_line(items: &[(&'static str, &'static str)], maximum_width: usize) -> Line<'static> {
    let mut spans = Vec::with_capacity(items.len() * 3);
    let mut width = 0;
    for (index, (key, description)) in items.iter().enumerate() {
        let separator_width = usize::from(index > 0) * 2;
        let item_width = UnicodeWidthStr::width(*key)
            + usize::from(!description.is_empty())
            + UnicodeWidthStr::width(*description);
        if width + separator_width + item_width > maximum_width {
            break;
        }
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            *key,
            Style::default()
                .fg(palette().orange)
                .add_modifier(Modifier::BOLD),
        ));
        if !description.is_empty() {
            spans.push(Span::styled(
                format!(" {description}"),
                Style::default().fg(palette().muted),
            ));
        }
        width += separator_width + item_width;
    }
    Line::from(spans)
}

pub(super) fn draw_settings(
    frame: &mut Frame<'_>,
    settings: &Settings,
    selection: usize,
    fetch_running: bool,
) -> SettingsRegions {
    let area = centered_min(frame.area(), 58, 0, 48, 28);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "SETTINGS",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Application preferences",
                Style::default().fg(palette().faint),
            ),
        ])),
        Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new("Space toggle   ←/→ interval   Enter edit   Esc close")
            .style(Style::default().fg(palette().muted))
            .alignment(Alignment::Right),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            area.width.saturating_sub(4),
            1,
        ),
    );

    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );
    let compact = area.height < 26;
    let automation_header_y = if compact { 3 } else { 4 };
    let auto_y = if compact { 4 } else { 7 };
    let interval_y = if compact { 5 } else { 9 };
    let interface_header_y = if compact { 7 } else { 13 };
    let workspace_y = if compact { 8 } else { 14 };
    let agent_y = if compact { 9 } else { 16 };
    let agent_time_y = if compact { 10 } else { 18 };
    let clear_timings_y = if compact { 11 } else { 20 };
    let media_y = if compact { 12 } else { 22 };
    let editor_y = if compact { 13 } else { 24 };
    let auto_row = Rect::new(inner.x, area.y.saturating_add(auto_y), inner.width, 1);
    let interval_row = Rect::new(inner.x, area.y.saturating_add(interval_y), inner.width, 1);
    let workspace_panel_row =
        Rect::new(inner.x, area.y.saturating_add(workspace_y), inner.width, 1);
    let agent_harness_row = Rect::new(inner.x, area.y.saturating_add(agent_y), inner.width, 1);
    let agent_time_row = Rect::new(inner.x, area.y.saturating_add(agent_time_y), inner.width, 1);
    let clear_agent_timings_row = Rect::new(
        inner.x,
        area.y.saturating_add(clear_timings_y),
        inner.width,
        1,
    );
    let media_preview_row = Rect::new(inner.x, area.y.saturating_add(media_y), inner.width, 1);
    let editor_row = Rect::new(inner.x, area.y.saturating_add(editor_y), inner.width, 1);
    let interval_down = Rect::new(
        interval_row.right().saturating_sub(15),
        interval_row.y,
        3,
        1,
    );
    let interval_up = Rect::new(interval_row.right().saturating_sub(3), interval_row.y, 3, 1);

    let media_protocol_label = media_preview_protocol_label(settings.media_preview_protocol);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Media protocol", Style::default().fg(palette().ink)),
            Span::raw(
                " ".repeat(
                    usize::from(media_preview_row.width)
                        .saturating_sub("Media protocol".len() + media_protocol_label.len()),
                ),
            ),
            Span::styled(media_protocol_label, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selection == 6 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        media_preview_row,
    );

    frame.render_widget(
        Paragraph::new(Line::styled(
            "AUTOMATION",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(
            inner.x,
            area.y.saturating_add(automation_header_y),
            inner.width,
            1,
        ),
    );
    if !compact {
        let description = truncate_width(
            "Fetch updated remote refs in the background",
            usize::from(inner.width),
        );
        frame.render_widget(
            Paragraph::new(description).style(Style::default().fg(palette().faint)),
            Rect::new(inner.x, area.y.saturating_add(5), inner.width, 1),
        );
    }

    let (auto_switch, auto_switch_color) = settings_toggle(settings.auto_fetch);
    let auto_padding =
        usize::from(auto_row.width).saturating_sub(19 + UnicodeWidthStr::width(auto_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Auto-fetch remotes", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(auto_padding)),
            Span::styled(
                auto_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(auto_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 0 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        auto_row,
    );

    let interval_control = format!("[-] {:>4} min [+]", settings.fetch_interval_minutes);
    let interval_padding = usize::from(interval_row.width)
        .saturating_sub("Fetch interval".len() + interval_control.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Fetch interval", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(interval_padding)),
            Span::styled(interval_control, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selection == 1 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        interval_row,
    );

    let status = if fetch_running {
        "Fetching remotes now...".to_owned()
    } else if settings.auto_fetch {
        format!(
            "Enabled · every {} minute{}",
            settings.fetch_interval_minutes,
            if settings.fetch_interval_minutes == 1 {
                ""
            } else {
                "s"
            }
        )
    } else {
        "Disabled".to_owned()
    };
    if !compact {
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(if settings.auto_fetch {
                palette().green
            } else {
                palette().faint
            })),
            Rect::new(inner.x, area.y.saturating_add(11), inner.width, 1),
        );
    }

    frame.render_widget(
        Paragraph::new(Line::styled(
            "INTERFACE",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(
            inner.x,
            area.y.saturating_add(interface_header_y),
            inner.width,
            1,
        ),
    );
    let (workspace_switch, workspace_switch_color) =
        settings_toggle(settings.workspace_panel_enabled);
    let workspace_padding = usize::from(workspace_panel_row.width)
        .saturating_sub(18 + UnicodeWidthStr::width(workspace_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Workspace manager", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(workspace_padding)),
            Span::styled(
                workspace_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(workspace_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 2 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        workspace_panel_row,
    );

    let (agent_harness_switch, agent_harness_switch_color) =
        settings_toggle(settings.show_agent_harness);
    let agent_harness_padding = usize::from(agent_harness_row.width)
        .saturating_sub(14 + UnicodeWidthStr::width(agent_harness_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agent harness", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(agent_harness_padding)),
            Span::styled(
                agent_harness_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(agent_harness_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 3 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        agent_harness_row,
    );

    let agent_time_label = settings.agent_time_display.label();
    let agent_time_padding = usize::from(agent_time_row.width)
        .saturating_sub("Agent time".len() + agent_time_label.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agent time", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(agent_time_padding)),
            Span::styled(agent_time_label, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selection == 4 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        agent_time_row,
    );

    let clear_label = "Clear";
    let clear_padding = usize::from(clear_agent_timings_row.width)
        .saturating_sub("Agent timing history".len() + clear_label.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agent timing history", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(clear_padding)),
            Span::styled(clear_label, Style::default().fg(palette().orange)),
        ]))
        .style(Style::default().bg(if selection == 5 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        clear_agent_timings_row,
    );

    let editor = settings
        .editor_command
        .as_deref()
        .unwrap_or("Not configured");
    let editor = truncate_width(editor, usize::from(editor_row.width).saturating_sub(17));
    let editor_padding =
        usize::from(editor_row.width).saturating_sub("Editor command".len() + editor.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Editor command", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(editor_padding)),
            Span::styled(
                editor,
                Style::default().fg(if settings.editor_command.is_some() {
                    palette().accent
                } else {
                    palette().muted
                }),
            ),
        ]))
        .style(Style::default().bg(if selection == 7 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        editor_row,
    );

    SettingsRegions {
        overlay: area,
        auto_fetch: auto_row,
        fetch_interval: interval_row,
        fetch_interval_down: interval_down,
        fetch_interval_up: interval_up,
        workspace_panel: workspace_panel_row,
        agent_harness: agent_harness_row,
        agent_time: agent_time_row,
        clear_agent_timings: clear_agent_timings_row,
        media_preview: media_preview_row,
        editor: editor_row,
    }
}

fn media_preview_protocol_label(protocol: crate::media::MediaPreviewProtocol) -> &'static str {
    match protocol {
        crate::media::MediaPreviewProtocol::Auto => "Auto",
        crate::media::MediaPreviewProtocol::Halfblocks => "Unicode",
        crate::media::MediaPreviewProtocol::Kitty => "Kitty (Ghostty)",
        crate::media::MediaPreviewProtocol::Iterm2 => "iTerm2 (WezTerm)",
        crate::media::MediaPreviewProtocol::Sixel => "Sixel (Windows Terminal)",
    }
}

fn settings_toggle(enabled: bool) -> (&'static str, Color) {
    if enabled {
        ("   ◼ ", palette().green)
    } else {
        (" ◼   ", palette().faint)
    }
}

pub(super) fn draw_editor(
    frame: &mut Frame<'_>,
    input: &str,
    error: Option<&str>,
    configure_only: bool,
) -> Rect {
    let area = centered_min(frame.area(), 64, 0, 52, 12);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "EDITOR COMMAND",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Saved for next time",
                Style::default().fg(palette().faint),
            ),
        ])),
        Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(4),
        area.width.saturating_sub(4),
        area.height.saturating_sub(5),
    );
    frame.render_widget(
        Paragraph::new("Choose the interactive editor used for selected files.")
            .style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(format!("{input}▌"))
            .style(Style::default().fg(palette().ink).bg(palette().selected)),
        Rect::new(inner.x, inner.y.saturating_add(2), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(error.unwrap_or("Examples: nvim · micro · code --wait")).style(
            Style::default().fg(if error.is_some() {
                palette().red
            } else {
                palette().faint
            }),
        ),
        Rect::new(inner.x, inner.y.saturating_add(4), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(if configure_only {
            "Enter save   Ctrl+U clear   Esc back"
        } else {
            "Enter save & open   Ctrl+U clear   Esc cancel"
        })
        .style(Style::default().fg(palette().muted))
        .alignment(Alignment::Right),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    area
}

pub(super) fn draw_help(frame: &mut Frame<'_>) {
    let area = centered_min(frame.area(), 72, 0, 58, 24);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "KEYBOARD",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Quick reference", Style::default().fg(palette().faint)),
        ])),
        Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    let body = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(4),
        area.width.saturating_sub(4),
        area.height.saturating_sub(5),
    );
    let columns = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Length(2),
        Constraint::Percentage(50),
    ])
    .split(body);
    let navigation = vec![
        Line::styled(
            "NAVIGATION",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        help_line("1 / 2 / Tab", "Switch view"),
        help_line("j / k", "Move / scroll hunk ×10"),
        help_line("Home / G", "First / last"),
        help_line("r", "Refresh"),
        help_line("o", "Explorer"),
        help_line("W", "Linked worktrees"),
        help_line("b", "Branches / PRs / issues"),
        help_line("w", "Open workspace manager"),
        help_line("p", "Workspace presets"),
        help_line("s", "Settings"),
        help_line("x", "Git actions"),
        help_line("g", "Git command"),
        help_line("F1", "Send to Herdr pane below"),
        help_line("e / E", "Edit / configure editor"),
        help_line("f", "Changes / files"),
        help_line("m", "Markdown preview / source"),
        help_line("F3", "Find repository file"),
        help_line("Alt+w", "Wrap preview"),
    ];
    let worktree = vec![
        Line::styled(
            "CHANGES / FILES",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        help_line("← / h", "Tree / exit hunk"),
        help_line("→ / l", "Enter / stage hunk"),
        help_line("Enter", "Toggle folder"),
        help_line("Space", "Stage file / hunk"),
        help_line("Delete", "Discard unstaged file changes"),
        help_line("a / u", "Stage / unstage all"),
        help_line("F2", "Rename file / folder / workspace"),
        help_line("Ctrl+Delete", "Delete from Files"),
        help_line("Ctrl+S", "Format selected file"),
        help_line("Drag", "Move file / folder"),
        help_line("c", "Commit editor"),
        help_line("Arrow keys", "Commit cursor"),
        help_line("C-A / C-⌫", "Select all / del word"),
        help_line("Ctrl+Enter", "Commit"),
        help_line("Esc", "Close / unfocus"),
        help_line("q", "Quit"),
    ];
    frame.render_widget(Paragraph::new(navigation), columns[0]);
    frame.render_widget(Paragraph::new(worktree), columns[2]);
    frame.render_widget(
        Paragraph::new("? / Esc close")
            .style(Style::default().fg(palette().muted))
            .alignment(Alignment::Right),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
}

fn truncate_start_width(value: &str, width: usize) -> String {
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

fn help_line<'a>(key: &'a str, description: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {key:<12}"),
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description, Style::default().fg(palette().ink)),
    ])
}

fn centered_min(
    area: Rect,
    width_percent: u16,
    height_percent: u16,
    minimum_width: u16,
    minimum_height: u16,
) -> Rect {
    let width = area
        .width
        .saturating_mul(width_percent)
        .checked_div(100)
        .unwrap_or(0)
        .max(minimum_width)
        .min(area.width.saturating_sub(4));
    let height = area
        .height
        .saturating_mul(height_percent)
        .checked_div(100)
        .unwrap_or(0)
        .max(minimum_height)
        .min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}
