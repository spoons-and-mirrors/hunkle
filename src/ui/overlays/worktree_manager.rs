use super::*;

pub(crate) fn draw_worktree_manager(
    frame: &mut Frame<'_>,
    manager: &mut WorktreeManager,
    shortcuts: &Shortcuts,
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
    hit_targets.extend(draw_explorer_tabs(
        frame,
        area,
        ExplorerTab::Worktrees,
        shortcuts,
    ));

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

    let create = shortcuts.label(ShortcutAction::CreateWorktree);
    let remove = shortcuts.label(ShortcutAction::DeleteWorktree);
    let refresh = shortcuts.label(ShortcutAction::RefreshWorktrees);
    frame.render_widget(
        Paragraph::new(key_hint_line(
            &[
                ("Enter", "open"),
                (create.as_str(), "new"),
                (remove.as_str(), "remove"),
                ("↑↓", "select"),
                (refresh.as_str(), "refresh"),
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

pub(super) fn detail_label(label: &'static str) -> Line<'static> {
    Line::styled(
        label,
        Style::default()
            .fg(palette().muted)
            .add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn draw_worktree_create_dialog(frame: &mut Frame<'_>, dialog: &WorktreeCreateDialog) {
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

pub(crate) fn draw_worktree_remove_dialog(frame: &mut Frame<'_>, dialog: &WorktreeRemoveDialog) {
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
