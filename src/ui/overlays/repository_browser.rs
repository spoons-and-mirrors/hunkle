use super::*;

pub(crate) fn draw_repository_browser(
    frame: &mut Frame<'_>,
    browser: &mut RepositoryBrowser,
    shortcuts: &Shortcuts,
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
    hit_targets.extend(draw_explorer_tabs(
        frame,
        area,
        ExplorerTab::Branches,
        shortcuts,
    ));
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

    let delete = shortcuts.label(ShortcutAction::BranchDelete);
    let footer = if browser.tab == BrowserTab::Branches {
        key_hint_line(
            &[
                ("Enter", "graph"),
                (delete.as_str(), "delete"),
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

pub(crate) fn draw_branch_delete_dialog(frame: &mut Frame<'_>, dialog: &BranchDeleteDialog) {
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

pub(crate) fn draw_repository_browser_details(
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
