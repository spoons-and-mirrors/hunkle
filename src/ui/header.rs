use super::*;

pub(super) fn draw_header(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let row = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().surface_alt)),
        row,
    );
    let Some(repo) = app.repository() else {
        frame.render_widget(
            Paragraph::new("  No workspace selected").style(Style::default().fg(palette().muted)),
            row,
        );
        return;
    };

    let repository = repository_label(repo);
    let worktree = app
        .linked_worktrees
        .worktree_name(&repo.root)
        .unwrap_or_else(|| "worktree".to_owned());
    let is_local = repo.is_local();
    let branch = if !is_local && !repo.details_ready {
        "loading".to_owned()
    } else {
        repo.branch.clone()
    };
    let dirty = !repo.changes.is_empty();
    let dirty_marker = if dirty { "*" } else { "" };
    let branch_badge = format!(" {branch}{dirty_marker} ");
    let branch_width = UnicodeWidthStr::width(branch_badge.as_str()) as u16;
    let diff_badge = " DIFF ";
    let diff_width = UnicodeWidthStr::width(diff_badge) as u16;
    let agent_badge = " AGENT ";
    let agent_width = UnicodeWidthStr::width(agent_badge) as u16;
    let comparison = app
        .changes
        .branch_comparison()
        .map(|comparison| format!(" {}...{}", comparison.target, comparison.current));
    let requested_comparison_width = comparison
        .as_deref()
        .map_or(0, |comparison| UnicodeWidthStr::width(comparison).min(40));
    let available = usize::from(area.width);
    let comparison_width = if is_local {
        0
    } else {
        requested_comparison_width.min(available.saturating_sub(
            1 + 5
                + 1
                + 5
                + 1
                + usize::from(branch_width)
                + 1
                + usize::from(diff_width)
                + 1
                + usize::from(agent_width),
        ))
    };
    let repository_width = UnicodeWidthStr::width(repository.as_str())
        .saturating_add(2)
        .min(20);
    let worktree_width = UnicodeWidthStr::width(worktree.as_str())
        .saturating_add(2)
        .min(18);
    let badge_width = if is_local {
        1 + repository_width + 1 + "LOCAL".len()
    } else {
        1 + repository_width
            + 1
            + worktree_width
            + 1
            + usize::from(branch_width)
            + 1
            + usize::from(diff_width)
            + 1
            + usize::from(agent_width)
            + comparison_width
    };
    let notice_budget = available
        .saturating_sub(badge_width.saturating_add(4))
        .min(30);
    let notice = (available >= 100 && notice_budget > 0)
        .then_some(app.notice.as_deref())
        .flatten()
        .map(|notice| truncate_width(notice, notice_budget));
    let notice_width = notice
        .as_deref()
        .map_or(0, |notice| UnicodeWidthStr::width(notice) + 2);
    let content_right = area.right().saturating_sub(notice_width as u16);
    let mut x = area.x.saturating_add(1);
    let render = |frame: &mut Frame<'_>, x: &mut u16, text: String, style: Style, limit: u16| {
        let width = (UnicodeWidthStr::width(text.as_str()) as u16).min(limit);
        if width == 0 {
            return None;
        }
        frame.render_widget(
            Paragraph::new(truncate_width(&text, usize::from(width))).style(style),
            Rect::new(*x, area.y, width, 1),
        );
        let rect = Rect::new(*x, area.y, width, 1);
        *x = x.saturating_add(width);
        Some(rect)
    };

    let room = content_right.saturating_sub(x);
    let repository_rect = render(
        frame,
        &mut x,
        format!(" {repository} "),
        header_badge_style(
            palette().yellow,
            app.hovered_hit_target == Some(HitTarget::HeaderRepository),
        ),
        room.saturating_sub(if is_local {
            0
        } else {
            branch_width
                .saturating_add(diff_width)
                .saturating_add(agent_width)
                .saturating_add(comparison_width as u16)
                .saturating_add(10)
        })
        .min(20),
    );
    if let Some(rect) = repository_rect {
        draw_header_badge_border(frame, rect, palette().yellow);
        app.regions
            .register_hit_target(HitTarget::HeaderRepository, rect);
    }
    let room = content_right.saturating_sub(x);
    let _ = render(frame, &mut x, " ".to_owned(), Style::default(), room);

    if is_local {
        let room = content_right.saturating_sub(x);
        let _ = render(
            frame,
            &mut x,
            "LOCAL".to_owned(),
            Style::default().fg(palette().muted),
            room,
        );
    } else {
        let room = content_right.saturating_sub(x);
        let worktree_rect = render(
            frame,
            &mut x,
            format!(" {worktree} "),
            header_badge_style(
                palette().orange,
                app.hovered_hit_target == Some(HitTarget::HeaderWorktrees),
            ),
            room.saturating_sub(
                branch_width
                    .saturating_add(diff_width)
                    .saturating_add(agent_width)
                    .saturating_add(comparison_width as u16)
                    .saturating_add(4),
            )
            .min(18),
        );
        if let Some(rect) = worktree_rect {
            draw_header_badge_border(frame, rect, palette().orange);
            app.regions
                .register_hit_target(HitTarget::HeaderWorktrees, rect);
        }
        let room = content_right.saturating_sub(x);
        let _ = render(frame, &mut x, " ".to_owned(), Style::default(), room);
        let room = content_right.saturating_sub(x);
        let branch_rect = render(
            frame,
            &mut x,
            branch_badge,
            header_badge_style(
                palette().accent,
                app.hovered_hit_target == Some(HitTarget::HeaderBranch),
            ),
            room.saturating_sub(
                diff_width
                    .saturating_add(agent_width)
                    .saturating_add(comparison_width as u16)
                    .saturating_add(2),
            ),
        );
        if let Some(rect) = branch_rect {
            draw_header_badge_border(frame, rect, palette().accent);
            app.regions
                .register_hit_target(HitTarget::HeaderBranch, rect);
        }
        let room = content_right.saturating_sub(x);
        let _ = render(frame, &mut x, " ".to_owned(), Style::default(), room);
        let room = content_right.saturating_sub(x);
        let diff_rect = render(
            frame,
            &mut x,
            diff_badge.to_owned(),
            header_badge_style(
                palette().purple,
                app.hovered_hit_target == Some(HitTarget::HeaderDiff),
            ),
            room.saturating_sub(agent_width.saturating_add(1)),
        );
        if let Some(rect) = diff_rect {
            draw_header_badge_border(frame, rect, palette().purple);
            app.regions.register_hit_target(HitTarget::HeaderDiff, rect);
        }
        let room = content_right.saturating_sub(x);
        let _ = render(frame, &mut x, " ".to_owned(), Style::default(), room);
        let room = content_right.saturating_sub(x);
        let agent_rect = render(
            frame,
            &mut x,
            agent_badge.to_owned(),
            header_badge_style(
                palette().green,
                app.hovered_hit_target == Some(HitTarget::HeaderAgent),
            ),
            room,
        );
        if let Some(rect) = agent_rect {
            draw_header_badge_border(frame, rect, palette().green);
            app.regions
                .register_hit_target(HitTarget::HeaderAgent, rect);
        }
        if let Some(comparison) = comparison {
            let room = content_right.saturating_sub(x);
            let _ = render(
                frame,
                &mut x,
                comparison,
                Style::default()
                    .fg(palette().purple)
                    .add_modifier(Modifier::BOLD),
                room.min(comparison_width as u16),
            );
        }
    }

    if let Some(notice) = notice {
        frame.render_widget(
            Paragraph::new(notice)
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().yellow)),
            Rect::new(
                content_right,
                area.y,
                area.right().saturating_sub(content_right),
                1,
            ),
        );
    }
}

pub(super) fn draw_main_top_padding(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let transition = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(
        Paragraph::new("▀".repeat(usize::from(transition.width))).style(
            Style::default()
                .fg(palette().surface_alt)
                .bg(palette().canvas),
        ),
        transition,
    );
    if let (Some(bounds), Some(left)) = (app.regions.split_bounds, app.regions.worktree) {
        let right_x = left.right().saturating_add(1);
        let right = Rect::new(right_x, bounds.y, bounds.right().saturating_sub(right_x), 1);
        for pane in [Rect::new(left.x, left.y, left.width, 1), right] {
            if pane.is_empty() {
                continue;
            }
            frame.render_widget(
                Paragraph::new("▀".repeat(usize::from(pane.width))).style(
                    Style::default()
                        .fg(palette().surface_alt)
                        .bg(palette().panel),
                ),
                pane,
            );
        }
    }
}

pub(super) fn repository_label(repo: &crate::git::RepositoryData) -> String {
    repository_root(repo)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("hunkle")
        .trim_end_matches(".git")
        .to_owned()
}

pub(super) fn repository_root(repo: &crate::git::RepositoryData) -> &std::path::Path {
    let common_dir = repo.common_dir.as_deref().unwrap_or(&repo.root);
    if common_dir.file_name().is_some_and(|name| name == ".git") {
        common_dir.parent().unwrap_or(common_dir)
    } else {
        common_dir
    }
}

pub(super) fn header_badge_style(background: Color, hovered: bool) -> Style {
    let (foreground, background) = if hovered {
        (palette().canvas, lighter(background))
    } else {
        (palette().ink, palette().surface_alt)
    };
    Style::default()
        .fg(foreground)
        .bg(background)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn draw_header_badge_border(frame: &mut Frame<'_>, rect: Rect, color: Color) {
    if let Some(cell) = frame.buffer_mut().cell_mut((rect.x, rect.y)) {
        cell.set_symbol("▌").set_fg(color);
    }
}

pub(super) fn lighter(color: Color) -> Color {
    match color {
        Color::Rgb(red, green, blue) => Color::Rgb(
            red.saturating_add((u8::MAX - red) / 3),
            green.saturating_add((u8::MAX - green) / 3),
            blue.saturating_add((u8::MAX - blue) / 3),
        ),
        Color::Red => Color::LightRed,
        Color::Green => Color::LightGreen,
        Color::Yellow => Color::LightYellow,
        Color::Blue => Color::LightBlue,
        Color::Magenta => Color::LightMagenta,
        Color::Cyan => Color::LightCyan,
        _ => Color::White,
    }
}

pub(super) fn draw_header_picker(frame: &mut Frame<'_>, app: &mut App) {
    let Some(kind) = app.header_picker.kind else {
        return;
    };
    let anchor_target = if kind == HeaderPickerKind::AgentDestinations {
        HitTarget::HeaderAgent
    } else {
        HitTarget::HeaderRepository
    };
    let anchor = app
        .regions
        .hit_target_rect(anchor_target)
        .unwrap_or(Rect::new(frame.area().x, frame.area().y, 1, 1));
    let picker_y = anchor.bottom();
    let available_height = frame.area().bottom().saturating_sub(picker_y);
    if available_height < 5 || frame.area().width < 12 {
        return;
    }
    let naming_branch = app.header_picker.naming_branch();
    let filtering = app.header_picker.filtering();
    let destination_title = filtering && kind == HeaderPickerKind::AgentDestinations;
    let picker_chrome = if filtering {
        4 + usize::from(destination_title)
    } else {
        1
    };
    let item_offset = if filtering {
        3 + u16::from(destination_title)
    } else {
        1
    };
    let visible_items = usize::from(
        available_height
            .saturating_sub(u16::try_from(picker_chrome).unwrap_or(u16::MAX))
            .min(10),
    );
    let row_count = if naming_branch {
        2
    } else if app.header_picker.items.is_empty() {
        1
    } else {
        app.header_picker.items.len().min(visible_items)
    };
    app.header_picker.set_viewport_rows(row_count);
    let maximum_width = if kind == HeaderPickerKind::AgentDestinations {
        72
    } else {
        58
    };
    let width = frame
        .area()
        .width
        .saturating_sub(2)
        .min(maximum_width)
        .max(12);
    let x = anchor
        .x
        .min(frame.area().right().saturating_sub(width).saturating_sub(1));
    let area = Rect::new(
        x,
        picker_y,
        width,
        u16::try_from(row_count + picker_chrome).unwrap_or(available_height),
    );
    frame.render_widget(Clear, area);
    fill(frame, area, palette().surface_alt);
    let title = match kind {
        HeaderPickerKind::Repositories => " RECENT REPOSITORIES".to_owned(),
        HeaderPickerKind::Worktrees => " WORKTREES".to_owned(),
        HeaderPickerKind::Branches => match app.header_picker.branch_step {
            BranchPickerStep::Branches => " BRANCHES".to_owned(),
            BranchPickerStep::Base => " NEW BRANCH · SELECT BASE".to_owned(),
            BranchPickerStep::Name => format!(
                " NEW BRANCH FROM {}",
                app.header_picker
                    .branch_base
                    .as_ref()
                    .map_or("branch", |branch| branch.name.as_str())
            ),
        },
        HeaderPickerKind::DiffTargets => " DIFF AGAINST".to_owned(),
        HeaderPickerKind::AgentDestinations => " AGENT DESTINATION".to_owned(),
    };
    let new_branch_action = kind == HeaderPickerKind::Branches
        && app.header_picker.branch_step == BranchPickerStep::Branches;
    let action_width = if new_branch_action { 11 } else { 0 };
    let action_space = action_width + u16::from(new_branch_action);
    if filtering {
        if destination_title {
            let title_width = area.width.min(14);
            frame.render_widget(
                Paragraph::new(" START AGENT").style(
                    Style::default()
                        .fg(palette().green)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(area.x, area.y.saturating_add(1), title_width, 1),
            );
            frame.render_widget(
                Paragraph::new("↑↓ SELECT · ENTER START ")
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(palette().muted)),
                Rect::new(
                    area.x.saturating_add(title_width),
                    area.y.saturating_add(1),
                    area.width.saturating_sub(title_width),
                    1,
                ),
            );
        }
        draw_header_picker_search(
            frame,
            app,
            area,
            action_space,
            1 + u16::from(destination_title),
        );
    } else {
        frame.render_widget(
            Paragraph::new(truncate_width(
                &title,
                usize::from(area.width.saturating_sub(action_width)),
            ))
            .style(Style::default().fg(palette().muted)),
            Rect::new(area.x, area.y, area.width.saturating_sub(action_width), 1),
        );
    }
    app.regions
        .register_hit_target(HitTarget::HeaderPickerOverlay, area);
    if new_branch_action {
        let action_row = Rect::new(
            area.right().saturating_sub(action_space),
            area.y.saturating_add(1),
            action_width,
            1,
        );
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderPickerNewBranch);
        let background = if hovered {
            palette().selected
        } else {
            palette().green
        };
        frame.render_widget(
            Paragraph::new(" New branch").style(
                Style::default()
                    .fg(palette().canvas)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            action_row,
        );
        frame.render_widget(
            Paragraph::new("▌").style(Style::default().fg(background).bg(palette().surface_alt)),
            Rect::new(action_row.right(), action_row.y, 1, 1),
        );
        app.regions
            .register_hit_target(HitTarget::HeaderPickerNewBranch, action_row);
    }
    if filtering {
        draw_half_padding(
            frame,
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
            '▀',
            palette().surface_alt,
            Color::Rgb(0, 0, 0),
        );
    }

    if naming_branch {
        let mut input = app.header_picker.branch_name.text().to_owned();
        if app.header_picker.branch_name.cursor_visible() {
            input.insert(app.header_picker.branch_name.cursor(), '▌');
        }
        frame.render_widget(
            Paragraph::new(truncate_start_width(
                &format!(" {input}"),
                usize::from(area.width),
            ))
            .style(Style::default().fg(palette().ink).bg(palette().selected)),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
        let (hint, color) = app
            .header_picker
            .message
            .as_ref()
            .map_or((" Enter create · Esc back", palette().muted), |message| {
                (message.as_str(), palette().red)
            });
        frame.render_widget(
            Paragraph::new(truncate_width(hint, usize::from(area.width)))
                .style(Style::default().fg(color)),
            Rect::new(area.x, area.y.saturating_add(2), area.width, 1),
        );
        return;
    }

    if app.header_picker.items.is_empty() {
        let message = app.header_picker.message.as_deref().unwrap_or_else(|| {
            if app.header_picker.query.is_empty() {
                "No entries"
            } else {
                "No matches"
            }
        });
        frame.render_widget(
            Paragraph::new(format!(" {message}")).style(Style::default().fg(palette().faint)),
            Rect::new(area.x, area.y.saturating_add(item_offset), area.width, 1),
        );
        return;
    }

    let start = app.header_picker.visible_start();
    let current_root = app.repository().map(|repository| repository.root.as_path());
    let current_common_dir = app
        .repository()
        .and_then(|repository| repository.common_dir.as_deref());
    let rows = app
        .header_picker
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(row_count)
        .map(|(index, item)| {
            let (label, detail, current, minimum_label_width, stats) = match item {
                HeaderPickerItem::Repository {
                    common_dir,
                    path,
                    stats,
                } => (
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("repository")
                        .to_owned(),
                    path.display().to_string(),
                    current_common_dir.is_some_and(|current| current == common_dir),
                    Some(12),
                    *stats,
                ),
                HeaderPickerItem::Worktree(worktree) => (
                    if worktree.is_main {
                        "worktree".to_owned()
                    } else {
                        worktree
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("worktree")
                            .to_owned()
                    },
                    worktree
                        .branch
                        .as_deref()
                        .and_then(|branch| branch.strip_prefix("refs/heads/"))
                        .unwrap_or(if worktree.is_detached { "detached" } else { "" })
                        .to_owned(),
                    current_root.is_some_and(|current| current == worktree.path),
                    None,
                    None,
                ),
                HeaderPickerItem::Branch(branch) => (
                    branch.name.clone(),
                    if branch.remote {
                        "remote"
                    } else if branch.current {
                        "current"
                    } else {
                        "local"
                    }
                    .to_owned(),
                    branch.current,
                    Some(4),
                    None,
                ),
                HeaderPickerItem::BranchBase(branch) => (
                    branch.name.clone(),
                    if branch.remote {
                        "remote"
                    } else if branch.current {
                        "current"
                    } else {
                        "local"
                    }
                    .to_owned(),
                    branch.current,
                    Some(4),
                    None,
                ),
                HeaderPickerItem::DiffTarget(branch) => (
                    branch.name.clone(),
                    if branch.remote { "remote" } else { "local" }.to_owned(),
                    branch.default,
                    Some(4),
                    None,
                ),
                HeaderPickerItem::AgentDestination {
                    path,
                    repository,
                    kind,
                    ..
                } => (
                    match kind {
                        AgentDestinationKind::Repository => repository.clone(),
                        AgentDestinationKind::Worktree => format!(
                            "  {}",
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("worktree")
                        ),
                    },
                    path.display().to_string(),
                    current_root.is_some_and(|current| current == path),
                    Some(22),
                    None,
                ),
            };
            (index, label, detail, current, minimum_label_width, stats)
        })
        .collect::<Vec<_>>();
    for (row, (index, label, detail, current, minimum_label_width, stats)) in
        rows.into_iter().enumerate()
    {
        let rect = Rect::new(
            area.x,
            area.y.saturating_add(item_offset + row as u16),
            area.width,
            1,
        );
        let selected = app.header_picker.selected == index;
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderPickerItem(index));
        let marker = if current { "●" } else { " " };
        let background = if selected || hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        if let Some(minimum_label_width) = minimum_label_width {
            fill(frame, rect, background);
            let detail_width = u16::try_from(UnicodeWidthStr::width(detail.as_str()))
                .unwrap_or(u16::MAX)
                .min(
                    rect.width
                        .saturating_sub(minimum_label_width)
                        .saturating_sub(1),
                );
            let detail_rect = Rect::new(
                rect.right().saturating_sub(detail_width).saturating_sub(1),
                rect.y,
                detail_width,
                1,
            );
            let label_rect = Rect::new(
                rect.x,
                rect.y,
                detail_rect.x.saturating_sub(rect.x).saturating_sub(1),
                1,
            );
            frame.render_widget(
                Paragraph::new(header_picker_label_line(
                    marker,
                    &label,
                    current,
                    stats,
                    usize::from(label_rect.width),
                ))
                .style(Style::default().bg(background)),
                label_rect,
            );
            frame.render_widget(
                Paragraph::new(truncate_start_width(
                    &detail,
                    usize::from(detail_rect.width),
                ))
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().muted).bg(background)),
                detail_rect,
            );
        } else {
            let text = truncate_width(
                &format!(" {marker} {label}  {detail}"),
                usize::from(rect.width),
            );
            frame.render_widget(
                Paragraph::new(text).style(
                    Style::default()
                        .fg(if current {
                            palette().accent
                        } else {
                            palette().ink
                        })
                        .bg(background),
                ),
                rect,
            );
        }
        app.regions
            .register_hit_target(HitTarget::HeaderPickerItem(index), rect);
    }
}

pub(super) fn header_picker_label_line(
    marker: &str,
    label: &str,
    current: bool,
    stats: Option<(u64, u64)>,
    width: usize,
) -> Line<'static> {
    let label_style = Style::default().fg(if current {
        palette().accent
    } else {
        palette().ink
    });
    let prefix = format!(" {marker} ");
    let stats_text =
        stats.map(|(additions, deletions)| (format!("+{additions}"), format!("-{deletions}")));
    let stats_width = stats_text.as_ref().map_or(0, |(additions, deletions)| {
        UnicodeWidthStr::width(format!("  {additions} {deletions}").as_str())
    });
    let label_width = width.saturating_sub(UnicodeWidthStr::width(prefix.as_str()) + stats_width);
    let mut spans = vec![Span::styled(prefix, label_style)];
    spans.push(Span::styled(
        truncate_width(label, label_width),
        label_style,
    ));
    if let Some((additions, deletions)) = stats_text {
        spans.extend([
            Span::raw("  "),
            Span::styled(additions, Style::default().fg(palette().green)),
            Span::raw(" "),
            Span::styled(deletions, Style::default().fg(palette().red)),
        ]);
    }
    Line::from(spans)
}

pub(super) fn draw_header_picker_search(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    action_width: u16,
    row_offset: u16,
) {
    let mut query = app.header_picker.query.text().to_owned();
    let cursor_visible = app.header_picker.query.cursor_visible();
    if !query.is_empty() && cursor_visible {
        query.insert(app.header_picker.query.cursor(), '▌');
    }
    let text = if query.is_empty() {
        let placeholder = match app.header_picker.kind {
            Some(HeaderPickerKind::Repositories) => "Search repositories...",
            Some(HeaderPickerKind::Worktrees) => "Search worktrees...",
            Some(HeaderPickerKind::Branches)
                if app.header_picker.branch_step == BranchPickerStep::Base =>
            {
                "Search base branch..."
            }
            Some(HeaderPickerKind::Branches) => "Search branch...",
            Some(HeaderPickerKind::DiffTargets) => "Search target branch...",
            Some(HeaderPickerKind::AgentDestinations) => "Filter repositories and worktrees...",
            None => "Search...",
        };
        format!(" {}{placeholder}", if cursor_visible { "▌" } else { " " })
    } else {
        format!(" {query}")
    };
    let search = Rect::new(
        area.x,
        area.y.saturating_add(row_offset),
        area.width.saturating_sub(action_width),
        1,
    );
    frame.render_widget(
        Paragraph::new(truncate_start_width(&text, usize::from(search.width))).style(
            Style::default()
                .fg(if query.is_empty() {
                    palette().soft
                } else {
                    palette().ink
                })
                .bg(palette().surface_alt),
        ),
        search,
    );
    draw_half_padding(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        '▄',
        palette().surface_alt,
        Color::Rgb(0, 0, 0),
    );
    draw_half_padding(
        frame,
        Rect::new(
            area.x,
            area.y.saturating_add(row_offset.saturating_add(1)),
            area.width,
            1,
        ),
        '▀',
        palette().surface_alt,
        palette().surface_alt,
    );
}

pub(super) fn draw_half_padding(
    frame: &mut Frame<'_>,
    area: Rect,
    glyph: char,
    foreground: Color,
    background: Color,
) {
    frame.render_widget(
        Paragraph::new(glyph.to_string().repeat(usize::from(area.width)))
            .style(Style::default().fg(foreground).bg(background)),
        area,
    );
}
