use super::*;

pub(crate) fn draw_file_add_popover(
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

pub(crate) fn draw_file_dialog(frame: &mut Frame<'_>, dialog: &FileDialog) -> FileDialogRegions {
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

pub(crate) fn draw_explorer(
    frame: &mut Frame<'_>,
    explorer: &mut Explorer,
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
    let tab_targets = draw_explorer_tabs(frame, area, ExplorerTab::Explorer, shortcuts);
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
        let favorite = shortcuts.label(ShortcutAction::ExplorerFavorite);
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
                    (favorite.as_str(), "favorite"),
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

pub(super) fn draw_explorer_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    active: ExplorerTab,
    shortcuts: &Shortcuts,
) -> Vec<(HitTarget, Rect)> {
    let mut x = area.x.saturating_add(2);
    ExplorerTab::ALL
        .into_iter()
        .enumerate()
        .map(|(index, tab)| {
            let (action, title) = match tab {
                ExplorerTab::Explorer => (ShortcutAction::ExplorerTabFiles, "EXPLORER"),
                ExplorerTab::Worktrees => (ShortcutAction::ExplorerTabWorktrees, "WORKTREES"),
                ExplorerTab::Branches => (ShortcutAction::ExplorerTabBranches, "BRANCHES"),
            };
            let label = format!("{}  {title}", shortcuts.label(action));
            let width =
                u16::try_from(UnicodeWidthStr::width(label.as_str()) + 2).unwrap_or(u16::MAX);
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

pub(crate) fn draw_file_search(
    frame: &mut Frame<'_>,
    search: &mut FileSearch,
    files: &[RepoPath],
    shortcuts: &Shortcuts,
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
        Paragraph::new(format!(
            "Enter open   ↑↓ select   Ctrl+U clear   {} / Esc close",
            shortcuts.label(ShortcutAction::FindFile)
        ))
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

pub(super) fn explorer_empty_list(message: &'static str) -> List<'static> {
    List::new([ListItem::new(Line::styled(
        format!("  {message}"),
        Style::default().fg(palette().faint),
    ))])
}

pub(super) fn key_hint_line<'a>(items: &[(&'a str, &'a str)], maximum_width: usize) -> Line<'a> {
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
