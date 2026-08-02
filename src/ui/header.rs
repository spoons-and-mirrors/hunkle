use super::*;

pub(super) fn draw_header(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let row = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().surface_alt)),
        row,
    );
    let fullscreen_rect = app.herdr.is_enabled().then(|| {
        let label = " ⛶ ";
        let width = UnicodeWidthStr::width(label) as u16;
        let rect = Rect::new(area.right().saturating_sub(width), area.y, width, 1);
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderFullscreen);
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(if app.herdr.fullscreen_running() {
                        palette().faint
                    } else if hovered {
                        palette().canvas
                    } else {
                        palette().cyan
                    })
                    .bg(if hovered && !app.herdr.fullscreen_running() {
                        palette().selected
                    } else if app.herdr.fullscreen() {
                        palette().accent
                    } else {
                        palette().raised
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
        if !app.herdr.fullscreen_running() {
            app.regions
                .register_hit_target(HitTarget::HeaderFullscreen, rect);
        }
        rect
    });
    let header_right = fullscreen_rect
        .map(|rect| rect.x.saturating_sub(1))
        .unwrap_or_else(|| area.right());
    let Some(repo) = app.repository() else {
        frame.render_widget(
            Paragraph::new("  No workspace selected").style(Style::default().fg(palette().muted)),
            Rect::new(row.x, row.y, header_right.saturating_sub(row.x), 1),
        );
        return;
    };

    let repository = repository_label(repo);
    let worktree = app
        .linked_worktrees
        .worktree_name(&repo.root)
        .unwrap_or_else(|| "basetree".to_owned());
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
    let available = usize::from(header_right.saturating_sub(area.x));
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
        .filter(|notice| !notice_is_error(notice))
        .map(|notice| truncate_width(notice, notice_budget));
    let notice_width = notice
        .as_deref()
        .map_or(0, |notice| UnicodeWidthStr::width(notice) + 2);
    let content_right = header_right.saturating_sub(notice_width as u16);
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
                header_right.saturating_sub(content_right),
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
    let anchor = app
        .regions
        .hit_target_rect(HitTarget::HeaderRepository)
        .unwrap_or(Rect::new(frame.area().x, frame.area().y, 1, 1));
    let picker_y = anchor.bottom();
    let available_height = frame.area().bottom().saturating_sub(picker_y);
    if available_height < 5 || frame.area().width < 12 {
        return;
    }
    let naming_branch = app.header_picker.naming_branch();
    let cloning_repository = app.header_picker.cloning_repository();
    let creating_worktree = app.header_picker.creating_worktree();
    let deleting_worktree = app.header_picker.deleting_worktree();
    let filtering = app.header_picker.filtering();
    let picker_chrome = if filtering { 4 } else { 1 };
    let item_offset = if filtering { 3 } else { 1 };
    let visible_items =
        header_picker_visible_items(frame.area().height, available_height, picker_chrome);
    let row_count = if cloning_repository {
        5
    } else if creating_worktree {
        3
    } else if deleting_worktree {
        3
    } else if naming_branch {
        2
    } else if app.header_picker.items.is_empty() {
        1
    } else {
        app.header_picker.items.len().min(visible_items)
    };
    app.header_picker.set_viewport_rows(row_count);
    let maximum_width = match kind {
        HeaderPickerKind::Repositories => 80,
        _ => 58,
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
        HeaderPickerKind::Repositories => match app.header_picker.repository_step {
            RepositoryPickerStep::Repositories => " RECENT REPOSITORIES".to_owned(),
            RepositoryPickerStep::Clone => " CLONE REPOSITORY".to_owned(),
        },
        HeaderPickerKind::Worktrees => match app.header_picker.worktree_step {
            WorktreePickerStep::Worktrees => " WORKTREES".to_owned(),
            WorktreePickerStep::Create => " NEW WORKTREE".to_owned(),
            WorktreePickerStep::Base => " NEW WORKTREE · SELECT BASE".to_owned(),
            WorktreePickerStep::Delete => " DELETE WORKTREE".to_owned(),
        },
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
    };
    let new_branch_action = kind == HeaderPickerKind::Branches
        && app.header_picker.branch_step == BranchPickerStep::Branches;
    let clone_action = kind == HeaderPickerKind::Repositories
        && app.header_picker.repository_step == RepositoryPickerStep::Repositories;
    let new_worktree_action = kind == HeaderPickerKind::Worktrees
        && app.header_picker.worktree_step == WorktreePickerStep::Worktrees
        && filtering;
    let action_width = if new_branch_action {
        11
    } else if clone_action {
        7
    } else if new_worktree_action {
        10
    } else {
        0
    };
    let has_action = new_branch_action || clone_action || new_worktree_action;
    let action_space = action_width + u16::from(has_action);
    if filtering {
        draw_header_picker_search(frame, app, area, action_space, 1);
    } else {
        frame.render_widget(
            Paragraph::new(truncate_width(
                &title,
                usize::from(area.width.saturating_sub(action_width)),
            ))
            .style(Style::default().fg(if deleting_worktree {
                palette().red
            } else {
                palette().muted
            })),
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
    } else if clone_action {
        let action_row = Rect::new(
            area.right().saturating_sub(action_space),
            area.y.saturating_add(1),
            action_width,
            1,
        );
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderPickerClone);
        let background = if hovered {
            palette().selected
        } else {
            palette().green
        };
        frame.render_widget(
            Paragraph::new(" Clone ").style(
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
            .register_hit_target(HitTarget::HeaderPickerClone, action_row);
    } else if new_worktree_action {
        let action_row = Rect::new(
            area.right().saturating_sub(action_space),
            area.y.saturating_add(1),
            action_width,
            1,
        );
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderPickerNewWorktree);
        let background = if hovered {
            palette().selected
        } else {
            palette().green
        };
        frame.render_widget(
            Paragraph::new(" New tree ").style(
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
            .register_hit_target(HitTarget::HeaderPickerNewWorktree, action_row);
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

    if cloning_repository {
        frame.render_widget(
            Paragraph::new(" DIRECTORY").style(Style::default().fg(palette().muted)),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
        let directory_row = Rect::new(area.x, area.y.saturating_add(2), area.width, 1);
        draw_picker_input(
            frame,
            &app.header_picker.clone_directory,
            directory_row,
            app.header_picker.clone_field == CloneField::Directory,
        );
        app.regions
            .register_hit_target(HitTarget::HeaderPickerCloneDirectory, directory_row);
        frame.render_widget(
            Paragraph::new(" GIT URL").style(Style::default().fg(palette().muted)),
            Rect::new(area.x, area.y.saturating_add(3), area.width, 1),
        );
        let url_row = Rect::new(area.x, area.y.saturating_add(4), area.width, 1);
        draw_picker_input(
            frame,
            &app.header_picker.clone_url,
            url_row,
            app.header_picker.clone_field == CloneField::Url,
        );
        app.regions
            .register_hit_target(HitTarget::HeaderPickerCloneUrl, url_row);
        let (hint, color) = app.header_picker.message.as_ref().map_or(
            (" Enter clone · Tab switch · Esc back", palette().muted),
            |message| (message.as_str(), palette().red),
        );
        frame.render_widget(
            Paragraph::new(truncate_width(hint, usize::from(area.width)))
                .style(Style::default().fg(color)),
            Rect::new(area.x, area.y.saturating_add(5), area.width, 1),
        );
        return;
    }

    if creating_worktree {
        frame.render_widget(
            Paragraph::new(" NEW BRANCH / WORKTREE NAME")
                .style(Style::default().fg(palette().muted)),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
        let name_row = Rect::new(area.x, area.y.saturating_add(2), area.width, 1);
        draw_picker_input(frame, &app.header_picker.worktree_name, name_row, true);
        app.regions
            .register_hit_target(HitTarget::HeaderPickerWorktreeName, name_row);
        let (hint, color) = app.header_picker.message.as_ref().map_or(
            (" Enter select base · Esc back", palette().muted),
            |message| (message.as_str(), palette().red),
        );
        frame.render_widget(
            Paragraph::new(truncate_width(hint, usize::from(area.width)))
                .style(Style::default().fg(color)),
            Rect::new(area.x, area.y.saturating_add(3), area.width, 1),
        );
        return;
    }

    if deleting_worktree {
        let path = app
            .header_picker
            .worktree_delete
            .as_deref()
            .map_or_else(|| "worktree".to_owned(), |path| path.display().to_string());
        frame.render_widget(
            Paragraph::new(truncate_start_width(
                &format!(" Delete {path}?"),
                usize::from(area.width),
            ))
            .style(Style::default().fg(palette().ink)),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(" Uncommitted changes prevent deletion")
                .style(Style::default().fg(palette().muted)),
            Rect::new(area.x, area.y.saturating_add(2), area.width, 1),
        );
        let button_width = area.width.saturating_sub(1).min(18) / 2;
        let cancel = Rect::new(
            area.right().saturating_sub(button_width),
            area.y.saturating_add(3),
            button_width,
            1,
        );
        let delete = Rect::new(
            cancel.x.saturating_sub(button_width).saturating_sub(1),
            cancel.y,
            button_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(truncate_width(" Delete", usize::from(delete.width))).style(
                Style::default()
                    .fg(palette().canvas)
                    .bg(palette().red)
                    .add_modifier(Modifier::BOLD),
            ),
            delete,
        );
        frame.render_widget(
            Paragraph::new(truncate_width(" Cancel", usize::from(cancel.width)))
                .style(Style::default().fg(palette().ink).bg(palette().selected)),
            cancel,
        );
        app.regions
            .register_hit_target(HitTarget::HeaderPickerConfirmDeleteWorktree, delete);
        app.regions
            .register_hit_target(HitTarget::HeaderPickerCancelDeleteWorktree, cancel);
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
    let rows = app
        .header_picker
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(row_count)
        .map(|(index, item)| {
            let (label, detail, branch, current, minimum_label_width, stats, columnar) = match item
            {
                HeaderPickerItem::Repository {
                    path,
                    label,
                    stats,
                    branch,
                    ..
                } => (
                    label.clone(),
                    path.display().to_string(),
                    Some(branch.clone().unwrap_or_default()),
                    current_root.is_some_and(|current| current == path),
                    None,
                    *stats,
                    true,
                ),
                HeaderPickerItem::Worktree { worktree, stats } => (
                    worktree
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("worktree")
                        .to_owned(),
                    worktree.path.display().to_string(),
                    None,
                    current_root.is_some_and(|current| current == worktree.path),
                    None,
                    *stats,
                    true,
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
                    None,
                    branch.current,
                    Some(4),
                    None,
                    false,
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
                    None,
                    branch.current,
                    Some(4),
                    None,
                    false,
                ),
                HeaderPickerItem::DiffTarget(branch) => (
                    branch.name.clone(),
                    if branch.remote { "remote" } else { "local" }.to_owned(),
                    None,
                    branch.default,
                    Some(4),
                    None,
                    false,
                ),
            };
            (
                index,
                label,
                detail,
                branch,
                current,
                minimum_label_width,
                stats,
                columnar,
            )
        })
        .collect::<Vec<_>>();
    for (row, (index, label, detail, branch, current, minimum_label_width, stats, columnar)) in
        rows.into_iter().enumerate()
    {
        let rect = Rect::new(
            area.x,
            area.y.saturating_add(item_offset + row as u16),
            area.width,
            1,
        );
        let selected = app.header_picker.selected == index;
        let hovered = matches!(
            app.hovered_hit_target,
            Some(HitTarget::HeaderPickerItem(hovered_index))
                | Some(HitTarget::HeaderPickerDeleteWorktree(hovered_index))
                if hovered_index == index
        );
        let marker = if current { "●" } else { " " };
        let background = if selected || hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        if columnar {
            draw_change_location_row(
                frame,
                rect,
                marker,
                &label,
                &detail,
                branch.as_deref(),
                current,
                stats,
                background,
            );
        } else if let Some(minimum_label_width) = minimum_label_width {
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
        let deletable = matches!(
            app.header_picker.items.get(index),
            Some(HeaderPickerItem::Worktree { worktree, .. }) if !worktree.is_main && !current
        );
        if hovered && deletable {
            let delete = Rect::new(rect.right().saturating_sub(3), rect.y, 3.min(rect.width), 1);
            frame.render_widget(
                Paragraph::new(" X ").style(
                    Style::default()
                        .fg(palette().canvas)
                        .bg(palette().red)
                        .add_modifier(Modifier::BOLD),
                ),
                delete,
            );
            app.regions
                .register_hit_target(HitTarget::HeaderPickerDeleteWorktree(index), delete);
        }
    }
}

fn header_picker_visible_items(screen_height: u16, available_height: u16, chrome: usize) -> usize {
    let maximum_height = screen_height.saturating_mul(4) / 5;
    usize::from(
        available_height
            .min(maximum_height)
            .saturating_sub(u16::try_from(chrome).unwrap_or(u16::MAX)),
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_change_location_row(
    frame: &mut Frame<'_>,
    area: Rect,
    marker: &str,
    label: &str,
    path: &str,
    branch: Option<&str>,
    current: bool,
    stats: Option<(u64, u64)>,
    background: Color,
) {
    fill(frame, area, background);
    let stats_width = area.width.min(13);
    let branch_width = if branch.is_some() {
        area.width
            .saturating_sub(stats_width)
            .saturating_sub(24)
            .min(20)
    } else {
        0
    };
    let name_width = area
        .width
        .saturating_sub(stats_width)
        .saturating_sub(branch_width)
        .saturating_sub(12)
        .min(24);
    let path_width = area
        .width
        .saturating_sub(name_width + stats_width + branch_width);
    let name_area = Rect::new(area.x, area.y, name_width, 1);
    let stats_area = Rect::new(area.x.saturating_add(name_width), area.y, stats_width, 1);
    let branch_area = Rect::new(
        stats_area.x.saturating_add(stats_width),
        area.y,
        branch_width,
        1,
    );
    let path_area = Rect::new(
        branch_area.x.saturating_add(branch_width),
        area.y,
        path_width,
        1,
    );
    frame.render_widget(
        Paragraph::new(header_picker_label_line(
            marker,
            label,
            current,
            None,
            usize::from(name_width),
        ))
        .style(Style::default().bg(background)),
        name_area,
    );
    if let Some((additions, deletions)) = stats {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("+{additions}"),
                    Style::default().fg(palette().green),
                ),
                Span::raw(" "),
                Span::styled(format!("-{deletions}"), Style::default().fg(palette().red)),
            ]))
            .style(Style::default().bg(background)),
            stats_area,
        );
    }
    if let Some(branch) = branch {
        frame.render_widget(
            Paragraph::new(truncate_width(branch, usize::from(branch_width)))
                .style(Style::default().fg(palette().accent).bg(background)),
            branch_area,
        );
    }
    frame.render_widget(
        Paragraph::new(truncate_start_width(path, usize::from(path_width)))
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted).bg(background)),
        path_area,
    );
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

fn draw_picker_input(frame: &mut Frame<'_>, input: &TextInput, area: Rect, active: bool) {
    let mut text = input.text().to_owned();
    if input.cursor_visible() {
        text.insert(input.cursor(), '▌');
    }
    frame.render_widget(
        Paragraph::new(truncate_start_width(
            &format!(" {text}"),
            usize::from(area.width),
        ))
        .style(Style::default().fg(palette().ink).bg(if active {
            palette().selected
        } else {
            palette().panel
        })),
        area,
    );
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

#[cfg(test)]
mod tests {
    use super::header_picker_visible_items;

    #[test]
    fn header_pickers_use_at_most_eighty_percent_of_the_screen() {
        assert_eq!(header_picker_visible_items(40, 39, 4), 28);
        assert_eq!(header_picker_visible_items(40, 20, 4), 16);
    }
}
