use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn draw_header(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    profile: LayoutProfile,
) {
    let herdr_available = app.herdr_available();
    let row = area;
    let content_y = area.bottom().saturating_sub(1);
    let card_gap = if profile.is_single() { " " } else { "  " };
    let card_gap_width = UnicodeWidthStr::width(card_gap) as u16;
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().canvas)),
        row,
    );
    let fullscreen_rect = (herdr_available && !profile.is_single()).then(|| {
        let label = " ⛶ ";
        let width = UnicodeWidthStr::width(label) as u16;
        let rect = Rect::new(area.right().saturating_sub(width), content_y, width, 1);
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderFullscreen);
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(if app.herdr.fullscreen_running() {
                        palette().faint
                    } else if hovered || app.herdr.fullscreen() {
                        palette().accent
                    } else {
                        palette().cyan
                    })
                    .bg(palette().canvas)
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
    let controls_right = fullscreen_rect
        .map(|rect| rect.x.saturating_sub(1))
        .unwrap_or_else(|| area.right());
    let schedule_rect = herdr_available.then(|| {
        let label = " SCHEDULE F4 ";
        let width = UnicodeWidthStr::width(label) as u16;
        let rect = Rect::new(controls_right.saturating_sub(width), content_y, width, 1);
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderSchedule);
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(if hovered || app.mode == Mode::Scheduler {
                        palette().accent
                    } else {
                        palette().cyan
                    })
                    .bg(palette().canvas)
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
        app.regions
            .register_hit_target(HitTarget::HeaderSchedule, rect);
        rect
    });
    let controls_right = schedule_rect
        .map(|rect| rect.x.saturating_sub(1))
        .unwrap_or(controls_right);
    let local_build_rect = app.local_build_available().then(|| {
        let label = " ↻ ";
        let width = UnicodeWidthStr::width(label) as u16;
        let rect = Rect::new(controls_right.saturating_sub(width), content_y, width, 1);
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderLocalBuild);
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(if hovered {
                        palette().accent
                    } else {
                        palette().green
                    })
                    .bg(palette().canvas)
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
        app.regions
            .register_hit_target(HitTarget::HeaderLocalBuild, rect);
        rect
    });
    let header_right = local_build_rect
        .map(|rect| rect.x.saturating_sub(1))
        .unwrap_or(controls_right);
    let Some(repo) = app.repository() else {
        frame.render_widget(
            Paragraph::new("  No workspace selected").style(Style::default().fg(palette().muted)),
            Rect::new(row.x, content_y, header_right.saturating_sub(row.x), 1),
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
    let (ahead, behind) = (repo.ahead, repo.behind);
    let full_branch_badge = branch_badge(&branch, dirty, ahead, behind, usize::MAX);
    let branch_width = UnicodeWidthStr::width(full_branch_badge.as_str()) as u16;
    let show_agent_actions = !app.herdr_prompt.agent_pane_picker_open();
    let diff_badge = " DIFF ";
    let diff_width = show_agent_actions
        .then(|| UnicodeWidthStr::width(diff_badge) as u16)
        .unwrap_or_default();
    let issue_badge = " ISSUE ";
    let issue_width = if show_agent_actions {
        UnicodeWidthStr::width(issue_badge) as u16
    } else {
        0
    };
    let agent_badge = " AGENT ";
    let agent_width = (show_agent_actions && herdr_available)
        .then(|| UnicodeWidthStr::width(agent_badge) as u16)
        .unwrap_or_default();
    let local_agent_width = if agent_width > 0 {
        usize::from(card_gap_width) + usize::from(agent_width)
    } else {
        0
    };
    let comparison = show_agent_actions
        .then(|| {
            app.changes
                .branch_comparison()
                .map(|comparison| format!(" {}...{}", comparison.target, comparison.current))
        })
        .flatten();
    let requested_comparison_width = comparison
        .as_deref()
        .map_or(0, |comparison| UnicodeWidthStr::width(comparison).min(40));
    let available = usize::from(header_right.saturating_sub(area.x));
    let comparison_width = if is_local || !show_agent_actions {
        0
    } else {
        requested_comparison_width.min(available.saturating_sub(
            1 + 5
                + usize::from(card_gap_width)
                + 5
                + usize::from(card_gap_width)
                + usize::from(branch_width)
                + usize::from(card_gap_width)
                + usize::from(diff_width)
                + usize::from(card_gap_width)
                + usize::from(issue_width)
                + usize::from(card_gap_width)
                + usize::from(agent_width),
        ))
    };
    let repository_width = UnicodeWidthStr::width(repository.as_str())
        .saturating_add(2)
        .min(20);
    let worktree_width = UnicodeWidthStr::width(worktree.as_str())
        .saturating_add(2)
        .min(18);
    let action_width = if show_agent_actions {
        usize::from(card_gap_width)
            + usize::from(diff_width)
            + usize::from(card_gap_width)
            + usize::from(issue_width)
            + usize::from(card_gap_width)
            + usize::from(agent_width)
            + comparison_width
    } else {
        0
    };
    let badge_width = if is_local {
        2 + repository_width + usize::from(card_gap_width) + "LOCAL".len() + local_agent_width
    } else {
        2 + repository_width
            + usize::from(card_gap_width)
            + worktree_width
            + usize::from(card_gap_width)
            + usize::from(branch_width)
            + action_width
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
    let mut x = area.x.saturating_add(2);
    let render = |frame: &mut Frame<'_>, x: &mut u16, text: String, style: Style, limit: u16| {
        let width = (UnicodeWidthStr::width(text.as_str()) as u16).min(limit);
        if width == 0 {
            return None;
        }
        frame.render_widget(
            Paragraph::new(truncate_width(&text, usize::from(width))).style(style),
            Rect::new(*x, content_y, width, 1),
        );
        let rect = Rect::new(*x, content_y, width, 1);
        *x = x.saturating_add(width);
        Some(rect)
    };
    let render_card = |frame: &mut Frame<'_>,
                       x: &mut u16,
                       text: String,
                       color: Color,
                       active: bool,
                       limit: u16| {
        let width = (UnicodeWidthStr::width(text.as_str()) as u16).min(limit);
        if width == 0 {
            return None;
        }
        let rect = Rect::new(*x, content_y, width, 1);
        draw_header_card(frame, rect, &text, color, active, false);
        *x = x.saturating_add(width);
        Some(rect)
    };

    let room = content_right.saturating_sub(x);
    let repository_rect = render_card(
        frame,
        &mut x,
        format!(" {repository} "),
        palette().yellow,
        app.hovered_hit_target == Some(HitTarget::HeaderRepository),
        room.saturating_sub(if is_local {
            0
        } else {
            branch_width
                .saturating_add(diff_width)
                .saturating_add(issue_width)
                .saturating_add(agent_width)
                .saturating_add(comparison_width as u16)
                .saturating_add(10)
        })
        .min(20),
    );
    if let Some(rect) = repository_rect {
        app.regions
            .register_hit_target(HitTarget::HeaderRepository, rect);
    }
    let room = content_right.saturating_sub(x);
    let _ = render(frame, &mut x, card_gap.to_owned(), Style::default(), room);

    if is_local {
        let room = content_right.saturating_sub(x);
        let _ = render(
            frame,
            &mut x,
            "LOCAL".to_owned(),
            Style::default().fg(palette().muted),
            room,
        );
        if agent_width > 0 {
            let room = content_right.saturating_sub(x);
            let _ = render(frame, &mut x, card_gap.to_owned(), Style::default(), room);
            let room = content_right.saturating_sub(x);
            let agent_rect = render_card(
                frame,
                &mut x,
                agent_badge.to_owned(),
                palette().green,
                app.hovered_hit_target == Some(HitTarget::HeaderAgent),
                room,
            );
            if let Some(rect) = agent_rect {
                app.regions
                    .register_hit_target(HitTarget::HeaderAgent, rect);
            }
        }
    } else {
        let room = content_right.saturating_sub(x);
        let worktree_rect = render_card(
            frame,
            &mut x,
            format!(" {worktree} "),
            palette().orange,
            app.hovered_hit_target == Some(HitTarget::HeaderWorktrees),
            room.saturating_sub(
                branch_width
                    .saturating_add(diff_width)
                    .saturating_add(issue_width)
                    .saturating_add(agent_width)
                    .saturating_add(comparison_width as u16)
                    .saturating_add(4u16.saturating_sub(card_gap_width.saturating_sub(1))),
            )
            .min(18),
        );
        if let Some(rect) = worktree_rect {
            app.regions
                .register_hit_target(HitTarget::HeaderWorktrees, rect);
        }
        let room = content_right.saturating_sub(x);
        let _ = render(frame, &mut x, card_gap.to_owned(), Style::default(), room);
        let room = content_right.saturating_sub(x);
        let branch_limit = room.saturating_sub(
            diff_width
                .saturating_add(issue_width)
                .saturating_add(agent_width)
                .saturating_add(comparison_width as u16)
                .saturating_add(2 + card_gap_width.saturating_sub(1).saturating_mul(3)),
        );
        let branch_rect = render_card(
            frame,
            &mut x,
            branch_badge(&branch, dirty, ahead, behind, usize::from(branch_limit)),
            palette().accent,
            app.hovered_hit_target == Some(HitTarget::HeaderBranch),
            branch_limit,
        );
        if let Some(rect) = branch_rect {
            app.regions
                .register_hit_target(HitTarget::HeaderBranch, rect);
        }
        if show_agent_actions {
            let room = content_right.saturating_sub(x);
            let _ = render(frame, &mut x, card_gap.to_owned(), Style::default(), room);
            let room = content_right.saturating_sub(x);
            let diff_rect = render_card(
                frame,
                &mut x,
                diff_badge.to_owned(),
                palette().purple,
                app.hovered_hit_target == Some(HitTarget::HeaderDiff),
                room.saturating_sub(
                    issue_width
                        .saturating_add(agent_width)
                        .saturating_add(card_gap_width.saturating_mul(2)),
                ),
            );
            if let Some(rect) = diff_rect {
                app.regions.register_hit_target(HitTarget::HeaderDiff, rect);
            }
            let room = content_right.saturating_sub(x);
            let _ = render(frame, &mut x, card_gap.to_owned(), Style::default(), room);
            let room = content_right.saturating_sub(x);
            let issue_rect = render_card(
                frame,
                &mut x,
                issue_badge.to_owned(),
                palette().cyan,
                app.hovered_hit_target == Some(HitTarget::HeaderIssue)
                    || app.header_picker.kind == Some(HeaderPickerKind::Issues),
                room.saturating_sub(agent_width.saturating_add(card_gap_width)),
            );
            if let Some(rect) = issue_rect {
                app.regions
                    .register_hit_target(HitTarget::HeaderIssue, rect);
            }
            if herdr_available {
                let room = content_right.saturating_sub(x);
                let _ = render(frame, &mut x, card_gap.to_owned(), Style::default(), room);
                let room = content_right.saturating_sub(x);
                let agent_rect = render_card(
                    frame,
                    &mut x,
                    agent_badge.to_owned(),
                    palette().green,
                    app.hovered_hit_target == Some(HitTarget::HeaderAgent),
                    room,
                );
                if let Some(rect) = agent_rect {
                    app.regions
                        .register_hit_target(HitTarget::HeaderAgent, rect);
                }
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
    }

    if let Some(notice) = notice {
        frame.render_widget(
            Paragraph::new(notice)
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().yellow)),
            Rect::new(
                content_right,
                content_y,
                header_right.saturating_sub(content_right),
                1,
            ),
        );
    }
}

fn branch_badge(branch: &str, dirty: bool, ahead: u64, behind: u64, width: usize) -> String {
    let dirty = if dirty { "*" } else { "" };
    let ahead = (ahead > 0).then(|| format!(" ↑{ahead}"));
    let behind = (behind > 0).then(|| format!(" ↓{behind}"));
    let suffix = format!(
        "{dirty}{}{} ",
        ahead.as_deref().unwrap_or_default(),
        behind.as_deref().unwrap_or_default()
    );
    let reserved = 1usize.saturating_add(UnicodeWidthStr::width(suffix.as_str()));
    let branch = truncate_width(branch, width.saturating_sub(reserved));
    format!(" {branch}{suffix}")
}

pub(super) fn draw_main_top_padding(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    profile: LayoutProfile,
) {
    let transition = Rect::new(area.x, area.y, area.width, 1);
    if profile.is_single() && app.regions.worktree.is_some() {
        frame.render_widget(
            Paragraph::new("▄".repeat(usize::from(transition.width)))
                .style(Style::default().fg(palette().panel).bg(palette().canvas)),
            transition,
        );
    } else {
        fill(frame, transition, palette().canvas);
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

pub(super) fn draw_header_card_bottom_padding(frame: &mut Frame<'_>, app: &App) {
    for (target, color) in [
        (HitTarget::HeaderRepository, palette().yellow),
        (HitTarget::HeaderWorktrees, palette().orange),
        (HitTarget::HeaderBranch, palette().accent),
        (HitTarget::HeaderDiff, palette().purple),
        (HitTarget::HeaderIssue, palette().cyan),
        (HitTarget::HeaderAgent, palette().green),
    ] {
        let Some(rect) = app.regions.hit_target_rect(target.clone()) else {
            continue;
        };
        let active = header_card_active(app, &target);
        draw_header_card_bottom(frame, rect, color, active);
    }
}

fn header_card_active(app: &App, target: &HitTarget) -> bool {
    app.hovered_hit_target.as_ref() == Some(target)
        || (target == &HitTarget::HeaderIssue
            && app.header_picker.kind == Some(HeaderPickerKind::Issues))
}

pub(super) fn draw_header_picker(frame: &mut Frame<'_>, app: &mut App, profile: LayoutProfile) {
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
    if app.header_picker.filtering() {
        if kind == HeaderPickerKind::Issues {
            draw_header_issue_picker(frame, app, anchor, available_height, profile);
        } else {
            draw_header_location_picker(frame, app, anchor, kind);
        }
        return;
    }
    let naming_branch = app.header_picker.naming_branch();
    let deleting_branch = app.header_picker.deleting_branch();
    let cloning_repository = kind == HeaderPickerKind::Repositories
        && app.header_picker.repository_step == RepositoryPickerStep::Clone;
    let creating_worktree = app.header_picker.creating_worktree();
    let deleting_worktree = app.header_picker.deleting_worktree();
    let row_count = if cloning_repository {
        5
    } else if creating_worktree {
        3
    } else if deleting_worktree || deleting_branch {
        3
    } else if naming_branch {
        2
    } else {
        return;
    };
    app.header_picker.set_viewport_rows(row_count);
    let maximum_width = if cloning_repository { 80 } else { 58 };
    let width = frame
        .area()
        .width
        .saturating_sub(2)
        .min(maximum_width)
        .max(12);
    let x = anchor
        .x
        .min(frame.area().right().saturating_sub(width).saturating_sub(1));
    let picker_height = row_count.saturating_add(1);
    let area = Rect::new(
        x,
        picker_y,
        width,
        u16::try_from(picker_height).unwrap_or(available_height),
    );
    frame.render_widget(Clear, area);
    fill(frame, area, palette().surface_alt);
    let title = if cloning_repository {
        " CLONE REPOSITORY".to_owned()
    } else if creating_worktree {
        " NEW WORKTREE".to_owned()
    } else if deleting_worktree {
        " DELETE WORKTREE".to_owned()
    } else if naming_branch {
        format!(
            " NEW BRANCH FROM {}",
            app.header_picker
                .branch_base
                .as_ref()
                .map_or("branch", |branch| branch.name.as_str())
        )
    } else {
        " DELETE BRANCH".to_owned()
    };
    frame.render_widget(
        Paragraph::new(truncate_width(&title, usize::from(area.width))).style(Style::default().fg(
            if deleting_worktree || deleting_branch {
                palette().red
            } else {
                palette().muted
            },
        )),
        Rect::new(area.x, area.y, area.width, 1),
    );
    app.regions
        .register_hit_target(HitTarget::HeaderPickerOverlay, area);

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

    if deleting_branch {
        let branch = app
            .header_picker
            .branch_delete
            .as_deref()
            .unwrap_or("branch");
        frame.render_widget(
            Paragraph::new(truncate_width(
                &format!(" Delete branch {branch}?"),
                usize::from(area.width),
            ))
            .style(Style::default().fg(palette().ink)),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(" Unmerged branches cannot be deleted")
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
            .register_hit_target(HitTarget::HeaderPickerConfirmDeleteBranch, delete);
        app.regions
            .register_hit_target(HitTarget::HeaderPickerCancelDeleteBranch, cancel);
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
}

fn draw_header_issue_picker(
    frame: &mut Frame<'_>,
    app: &mut App,
    anchor: Rect,
    available_height: u16,
    profile: LayoutProfile,
) {
    const PICKER_CHROME: usize = 4;
    const ITEM_OFFSET: u16 = 3;
    const ACTION_WIDTH: u16 = 8;
    const ACTION_SPACE: u16 = ACTION_WIDTH + 1;

    let mobile = profile.is_single();
    let item_height = if mobile { 3 } else { 1 };
    let visible_item_rows = if mobile {
        usize::from(
            available_height.saturating_sub(u16::try_from(PICKER_CHROME).unwrap_or(u16::MAX)),
        )
    } else {
        header_picker_visible_items(frame.area().height, available_height, PICKER_CHROME)
    };
    let visible_items = (visible_item_rows / item_height).max(1);
    let row_count = app.header_picker.items.len().min(visible_items).max(1);
    app.header_picker.set_viewport_rows(row_count);
    let width = if mobile {
        frame.area().width
    } else {
        frame
            .area()
            .width
            .saturating_sub(2)
            .min(frame.area().width.saturating_mul(9) / 10)
            .max(12)
    };
    let x = if mobile {
        frame.area().x
    } else {
        anchor
            .x
            .min(frame.area().right().saturating_sub(width).saturating_sub(1))
    };
    let picker_height = row_count
        .saturating_mul(item_height)
        .saturating_add(PICKER_CHROME);
    let area = Rect::new(
        x,
        anchor.bottom(),
        width,
        u16::try_from(picker_height).unwrap_or(available_height),
    );
    frame.render_widget(Clear, area);
    fill(frame, area, palette().surface_alt);
    draw_location_picker_search(
        frame,
        &app.header_picker.query,
        "Search issues and pull requests...",
        area,
        ACTION_SPACE,
    );
    app.regions
        .register_hit_target(HitTarget::HeaderPickerOverlay, area);

    let action_row = Rect::new(
        area.right().saturating_sub(ACTION_SPACE),
        area.y.saturating_add(1),
        ACTION_WIDTH,
        1,
    );
    let hovered = app.hovered_hit_target == Some(HitTarget::HeaderPickerIssueScope);
    let background = if hovered {
        palette().selected
    } else {
        palette().cyan
    };
    frame.render_widget(
        Paragraph::new(format!(" {:^6} ", app.issues.scope().label())).style(
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
        .register_hit_target(HitTarget::HeaderPickerIssueScope, action_row);
    draw_half_padding(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        '▀',
        palette().surface_alt,
        Color::Rgb(0, 0, 0),
    );

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
            Rect::new(area.x, area.y.saturating_add(ITEM_OFFSET), area.width, 1),
        );
        return;
    }

    let start = app.header_picker.visible_start();
    let rows = app
        .header_picker
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(row_count)
        .filter_map(|(index, item)| matches!(item, HeaderPickerItem::Issue { .. }).then_some(index))
        .collect::<Vec<_>>();
    for (row, index) in rows.into_iter().enumerate() {
        let rect = Rect::new(
            area.x,
            area.y.saturating_add(
                ITEM_OFFSET + u16::try_from(row.saturating_mul(item_height)).unwrap_or(u16::MAX),
            ),
            area.width,
            u16::try_from(item_height).unwrap_or(1),
        );
        let hovered = app.hovered_hit_target == Some(HitTarget::HeaderPickerItem(index));
        let background = if app.header_picker.selected == index || hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        draw_issue_picker_row(frame, rect, &app.header_picker.items[index], background);
        app.regions
            .register_hit_target(HitTarget::HeaderPickerItem(index), rect);
    }
}

fn draw_header_location_picker(
    frame: &mut Frame<'_>,
    app: &mut App,
    anchor: Rect,
    kind: HeaderPickerKind,
) {
    let current_root = app.repository().map(|repository| repository.root.as_path());
    let rows = app
        .header_picker
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let hovered = matches!(
                app.hovered_hit_target,
                Some(HitTarget::HeaderPickerItem(hovered_index))
                    | Some(HitTarget::HeaderPickerDeleteBranch(hovered_index))
                    | Some(HitTarget::HeaderPickerDeleteWorktree(hovered_index))
                    if hovered_index == index
            );
            match item {
                HeaderPickerItem::Repository {
                    path,
                    label,
                    stats,
                    branch,
                } => Some(LocationPickerRow {
                    target: HitTarget::HeaderPickerItem(index),
                    label: label.clone(),
                    detail: path.display().to_string(),
                    current: current_root.is_some_and(|current| current == path),
                    stats: *stats,
                    kind: LocationPickerRowKind::Location {
                        branch: branch.clone(),
                    },
                    selected: app.header_picker.selected == index,
                    hovered,
                    delete_target: None,
                }),
                HeaderPickerItem::Worktree { worktree, stats } => {
                    let current = current_root.is_some_and(|root| root == worktree.path);
                    Some(LocationPickerRow {
                        target: HitTarget::HeaderPickerItem(index),
                        label: worktree
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("worktree")
                            .to_owned(),
                        detail: worktree.path.display().to_string(),
                        current,
                        stats: *stats,
                        kind: LocationPickerRowKind::Location { branch: None },
                        selected: app.header_picker.selected == index,
                        hovered,
                        delete_target: (!worktree.is_main && !current)
                            .then_some(HitTarget::HeaderPickerDeleteWorktree(index)),
                    })
                }
                HeaderPickerItem::Branch(branch) | HeaderPickerItem::BranchBase(branch) => {
                    Some(LocationPickerRow {
                        target: HitTarget::HeaderPickerItem(index),
                        label: branch.name.clone(),
                        detail: branch_picker_detail(branch),
                        current: branch.current,
                        stats: None,
                        kind: LocationPickerRowKind::Choice,
                        selected: app.header_picker.selected == index,
                        hovered,
                        delete_target: matches!(item, HeaderPickerItem::Branch(branch) if !branch.remote && !branch.current)
                            .then_some(HitTarget::HeaderPickerDeleteBranch(index)),
                    })
                }
                HeaderPickerItem::DiffTarget {
                    label,
                    detail,
                    default,
                    ..
                } => Some(LocationPickerRow {
                    target: HitTarget::HeaderPickerItem(index),
                    label: label.clone(),
                    detail: detail.clone(),
                    current: *default,
                    stats: None,
                    kind: LocationPickerRowKind::Choice,
                    selected: app.header_picker.selected == index,
                    hovered,
                    delete_target: None,
                }),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    let actions = match kind {
        HeaderPickerKind::Repositories => vec![
            LocationPickerAction {
                target: HitTarget::HeaderPickerOpenExplorer,
                label: " Open ",
                color: palette().cyan,
                hovered: app.hovered_hit_target == Some(HitTarget::HeaderPickerOpenExplorer),
            },
            LocationPickerAction {
                target: HitTarget::HeaderPickerClone,
                label: " Clone ",
                color: palette().green,
                hovered: app.hovered_hit_target == Some(HitTarget::HeaderPickerClone),
            },
        ],
        HeaderPickerKind::Worktrees
            if app.header_picker.worktree_step == WorktreePickerStep::Worktrees =>
        {
            vec![LocationPickerAction {
                target: HitTarget::HeaderPickerNewWorktree,
                label: " New tree ",
                color: palette().green,
                hovered: app.hovered_hit_target == Some(HitTarget::HeaderPickerNewWorktree),
            }]
        }
        HeaderPickerKind::Branches
            if app.header_picker.branch_step == BranchPickerStep::Branches =>
        {
            vec![LocationPickerAction {
                target: HitTarget::HeaderPickerNewBranch,
                label: " New branch",
                color: palette().green,
                hovered: app.hovered_hit_target == Some(HitTarget::HeaderPickerNewBranch),
            }]
        }
        _ => Vec::new(),
    };
    let capacity = location_picker_capacity(frame.area(), frame.area(), anchor);
    app.header_picker
        .set_viewport_rows(rows.len().min(capacity).max(1));
    let placeholder = match kind {
        HeaderPickerKind::Repositories => "Search repositories...",
        HeaderPickerKind::Worktrees => "Search worktrees...",
        HeaderPickerKind::Branches if app.header_picker.branch_step == BranchPickerStep::Base => {
            "Search base branch..."
        }
        HeaderPickerKind::Branches => "Search branch...",
        HeaderPickerKind::DiffTargets => "Search target branch...",
        _ => "Search...",
    };
    let (targets, _) = draw_location_picker(
        frame,
        frame.area(),
        anchor,
        LocationPickerView {
            query: &app.header_picker.query,
            placeholder,
            rows: &rows,
            visible_start: app.header_picker.visible_start(),
            actions: &actions,
            maximum_width: if kind == HeaderPickerKind::Repositories {
                80
            } else {
                58
            },
            overlay_target: HitTarget::HeaderPickerOverlay,
        },
    );
    for (target, rect) in targets {
        app.regions.register_hit_target(target, rect);
    }
}

fn branch_picker_detail(branch: &crate::git::Branch) -> String {
    let location = branch_picker_location(branch);
    let age = branch.last_touched_at.map(branch_age).unwrap_or_default();
    let age = truncate_start_width(&age, 8);
    let separator = if age.is_empty() { "   " } else { " · " };
    format!("{location:<7}{separator}{age:>8}")
}

fn branch_picker_location(branch: &crate::git::Branch) -> &'static str {
    if branch.remote {
        "remote"
    } else if branch.current {
        "current"
    } else {
        "local"
    }
}

fn branch_age(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(timestamp);
    let seconds = now.saturating_sub(timestamp).max(0) as u64;
    if seconds < 60 {
        return "just now".to_owned();
    }
    if seconds < 3_600 {
        return format!("{}m ago", seconds / 60);
    }
    if seconds < 86_400 {
        return format!("{}h ago", seconds / 3_600);
    }
    if seconds < 604_800 {
        return format!("{}d ago", seconds / 86_400);
    }
    if seconds < 2_592_000 {
        return format!("{}w ago", seconds / 604_800);
    }
    if seconds < 31_536_000 {
        return format!("{}mo ago", seconds / 2_592_000);
    }
    format!("{}y ago", seconds / 31_536_000)
}

fn draw_issue_picker_row(
    frame: &mut Frame<'_>,
    area: Rect,
    item: &HeaderPickerItem,
    background: Color,
) {
    let HeaderPickerItem::Issue {
        number,
        title,
        pull_request,
        status,
        author,
        labels,
        changed_files,
        additions,
        deletions,
        ..
    } = item
    else {
        return;
    };
    let (kind, kind_color) = if *pull_request {
        ("PR", palette().purple)
    } else {
        ("ISSUE", palette().cyan)
    };
    let status_color = match status.as_str() {
        "MERGED" => palette().purple,
        "CLOSED" => palette().red,
        "DRAFT" => palette().yellow,
        "READY" => palette().green,
        _ => palette().cyan,
    };
    if area.height >= 3 {
        fill(frame, area, background);
        let number_area = Rect::new(area.x.saturating_add(1), area.y, 7.min(area.width), 1);
        let kind_area = Rect::new(number_area.right(), area.y, 7.min(area.width), 1);
        let status_area = Rect::new(area.right().saturating_sub(8), area.y, 8.min(area.width), 1);
        frame.render_widget(
            Paragraph::new(format!("#{number}"))
                .style(Style::default().fg(palette().muted).bg(background)),
            number_area,
        );
        frame.render_widget(
            Paragraph::new(format!(" {:^5} ", kind)).style(
                Style::default()
                    .fg(kind_color)
                    .bg(palette().canvas)
                    .add_modifier(Modifier::BOLD),
            ),
            kind_area,
        );
        frame.render_widget(
            Paragraph::new(format!(" {:^6} ", status)).style(
                Style::default()
                    .fg(status_color)
                    .bg(palette().canvas)
                    .add_modifier(Modifier::BOLD),
            ),
            status_area,
        );
        frame.render_widget(
            Paragraph::new(truncate_width(
                title,
                usize::from(area.width.saturating_sub(2)),
            ))
            .style(Style::default().fg(palette().ink).bg(background)),
            Rect::new(
                area.x.saturating_add(1),
                area.y.saturating_add(1),
                area.width.saturating_sub(2),
                1,
            ),
        );
        let mut metadata = Vec::new();
        if let Some(author) = author {
            metadata.push(Span::styled(
                format!("@{}", author.chars().take(6).collect::<String>()),
                Style::default().fg(palette().muted),
            ));
        }
        if let Some(files) = changed_files {
            if !metadata.is_empty() {
                metadata.push(Span::styled(" · ", Style::default().fg(palette().faint)));
            }
            metadata.push(Span::styled(
                format!("{files} {}", if *files == 1 { "file" } else { "files" }),
                Style::default().fg(palette().muted),
            ));
        }
        if let Some(additions) = additions {
            metadata.push(Span::styled(
                format!("  +{additions}"),
                Style::default().fg(palette().green),
            ));
        }
        if let Some(deletions) = deletions {
            metadata.push(Span::styled(
                format!(" -{deletions}"),
                Style::default().fg(palette().red),
            ));
        }
        if let Some(label) = labels.first() {
            if !metadata.is_empty() {
                metadata.push(Span::styled(" · ", Style::default().fg(palette().faint)));
            }
            metadata.push(Span::styled(
                label.clone(),
                Style::default().fg(palette().soft),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(metadata)).style(Style::default().bg(background)),
            Rect::new(
                area.x.saturating_add(1),
                area.y.saturating_add(2),
                area.width.saturating_sub(2),
                1,
            ),
        );
        return;
    }
    const NUMBER_WIDTH: u16 = 7;
    const KIND_WIDTH: u16 = 7;
    const STATUS_WIDTH: u16 = 8;
    const AUTHOR_WIDTH: u16 = 8;
    const COLUMN_GAPS: u16 = 7;

    fill(frame, area, background);
    let spacious = area.width >= 80;
    let outer_padding = u16::from(spacious);
    let gutter = u16::from(spacious);
    let files_width = if spacious { 10 } else { 8 };
    let loc_width = if spacious { 9 } else { 7 };
    let fixed_width = NUMBER_WIDTH
        + KIND_WIDTH
        + STATUS_WIDTH
        + files_width
        + loc_width * 2
        + AUTHOR_WIDTH
        + outer_padding * 2
        + gutter * COLUMN_GAPS;
    let title_width = area.width.saturating_sub(fixed_width);
    let mut column_x = area.x.saturating_add(outer_padding);
    let column = |x: &mut u16, width| {
        let column = Rect::new(*x, area.y, width, 1);
        *x = column.right().saturating_add(gutter);
        column
    };
    let number_area = column(&mut column_x, NUMBER_WIDTH);
    let kind_area = column(&mut column_x, KIND_WIDTH);
    let title_area = column(&mut column_x, title_width);
    let status_area = column(&mut column_x, STATUS_WIDTH);
    let files_area = column(&mut column_x, files_width);
    let additions_area = column(&mut column_x, loc_width);
    let deletions_area = column(&mut column_x, loc_width);
    let author_area = column(&mut column_x, AUTHOR_WIDTH);

    frame.render_widget(
        Paragraph::new(format!("#{number}"))
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted).bg(background)),
        number_area,
    );
    frame.render_widget(
        Paragraph::new(format!(" {:^5} ", kind)).style(
            Style::default()
                .fg(kind_color)
                .bg(palette().canvas)
                .add_modifier(Modifier::BOLD),
        ),
        kind_area,
    );
    let title = if spacious {
        title.clone()
    } else {
        format!(" {title}")
    };
    frame.render_widget(
        Paragraph::new(truncate_width(&title, usize::from(title_width)))
            .style(Style::default().fg(palette().ink).bg(background)),
        title_area,
    );
    frame.render_widget(
        Paragraph::new(format!(" {:^6} ", status)).style(
            Style::default()
                .fg(status_color)
                .bg(palette().canvas)
                .add_modifier(Modifier::BOLD),
        ),
        status_area,
    );
    if let Some(files) = changed_files {
        let noun = if *files == 1 { "file" } else { "files" };
        frame.render_widget(
            Paragraph::new(format!("{files} {noun}"))
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().muted).bg(background)),
            files_area,
        );
    }
    if let Some(additions) = additions {
        frame.render_widget(
            Paragraph::new(format!("+{additions}"))
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().green).bg(background)),
            additions_area,
        );
    }
    if let Some(deletions) = deletions {
        frame.render_widget(
            Paragraph::new(format!("-{deletions}"))
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().red).bg(background)),
            deletions_area,
        );
    }
    if let Some(author) = author {
        let author = author.chars().take(6).collect::<String>();
        frame.render_widget(
            Paragraph::new(format!("@{author}"))
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().muted).bg(background)),
            author_area,
        );
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

#[cfg(test)]
mod tests {
    use super::header_picker_visible_items;

    #[test]
    fn header_pickers_use_at_most_eighty_percent_of_the_screen() {
        assert_eq!(header_picker_visible_items(40, 39, 4), 28);
        assert_eq!(header_picker_visible_items(40, 20, 4), 16);
    }
}
