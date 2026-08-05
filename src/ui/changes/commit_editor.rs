use super::*;

pub(super) fn draw_commit_editor(
    frame: &mut Frame<'_>,
    app: &mut App,
    commit_area: Rect,
    actions_row: Rect,
    local_workspace: bool,
    has_changes: bool,
    details_ready: bool,
) {
    if !local_workspace && details_ready {
        draw_commit_message_action(frame, actions_row, app, has_changes);
    }
    let commit_active = app.mode == Mode::Commit;
    fill(frame, commit_area, palette().panel);
    let commit_content = commit_area.inner(Margin::new(1, 0));
    fill(frame, commit_content, palette().canvas);
    if commit_area.width >= 2 {
        for y in commit_area.y..commit_area.bottom() {
            if let Some(cell) = frame.buffer_mut().cell_mut((commit_area.x, y)) {
                cell.set_symbol("▐")
                    .set_fg(palette().canvas)
                    .set_bg(palette().panel);
            }
            if let Some(cell) = frame
                .buffer_mut()
                .cell_mut((commit_area.right().saturating_sub(1), y))
            {
                cell.set_symbol("▌")
                    .set_fg(palette().canvas)
                    .set_bg(palette().panel);
            }
        }
    }
    let (commit_text, commit_height) = if local_workspace {
        (
            Text::from(vec![
                Line::styled(
                    "Local file workspace",
                    Style::default()
                        .fg(palette().muted)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    "Git status and commits are unavailable",
                    Style::default().fg(palette().faint),
                ),
            ]),
            2,
        )
    } else if !details_ready {
        (
            Text::from(Line::styled(
                "Loading Git status…",
                Style::default().fg(palette().faint),
            )),
            1,
        )
    } else if app.commit_running() {
        (
            Text::from(Line::styled(
                "Creating commit...",
                Style::default().fg(palette().yellow),
            )),
            1,
        )
    } else if commit_active || !app.commit_input.is_empty() {
        let lines = text_input_lines(&app.commit_input, commit_active, palette().muted);
        let height = rendered_text_height(&lines, usize::from(commit_content.width), true);
        (Text::from(lines), height)
    } else {
        let hint = format!(
            "{} commit",
            app.settings.shortcuts.label(ShortcutAction::SubmitCommit)
        );
        let placeholder = "Write a commit message";
        if commit_content.width >= 40 {
            let padding =
                usize::from(commit_content.width).saturating_sub(placeholder.len() + hint.len());
            (
                Text::from(Line::from(vec![
                    Span::styled(placeholder, Style::default().fg(palette().muted)),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(hint, Style::default().fg(palette().faint)),
                ])),
                1,
            )
        } else {
            (
                Text::from(Line::styled(
                    placeholder,
                    Style::default().fg(palette().muted),
                )),
                1,
            )
        }
    };
    let automatic_commit_scroll = if commit_active {
        TextInput::visual_cursor_row(&app.commit_input, usize::from(commit_content.width))
            .saturating_sub(usize::from(commit_content.height).saturating_sub(1))
    } else {
        commit_height.saturating_sub(usize::from(commit_content.height))
    };
    let commit_scroll_max = commit_height.saturating_sub(usize::from(commit_content.height));
    let commit_scroll = app
        .commit_scroll
        .unwrap_or(automatic_commit_scroll)
        .min(commit_scroll_max)
        .min(usize::from(u16::MAX));
    if app.commit_scroll.is_some() {
        app.commit_scroll = Some(commit_scroll);
    }
    app.regions.commit_scroll = commit_scroll;
    app.regions.commit_scroll_max = commit_scroll_max;
    frame.render_widget(
        Paragraph::new(commit_text)
            .wrap(Wrap { trim: false })
            .scroll((commit_scroll as u16, 0))
            .style(Style::default().bg(palette().canvas)),
        commit_content,
    );
}

pub(super) fn draw_actions(frame: &mut Frame<'_>, area: Rect, mode: Mode) -> Rect {
    let label = " x ACTIONS ▾ ";
    let width = (UnicodeWidthStr::width(label) as u16).min(area.width);
    let button = Rect::new(area.right().saturating_sub(width), area.y, width, 1);
    fill(frame, area, palette().panel);
    frame.render_widget(
        Paragraph::new(Line::styled(
            label,
            Style::default()
                .fg(palette().accent)
                .bg(if mode == Mode::ActionMenu {
                    palette().selected
                } else {
                    palette().raised
                })
                .add_modifier(Modifier::BOLD),
        )),
        button,
    );
    button
}

pub(super) fn draw_commit_message_action(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App,
    has_changes: bool,
) {
    if app.commit_running() || !app.commit_message_available() || !has_changes || area.width < 3 {
        return;
    }

    let button = Rect::new(area.x, area.y, 3, 1);
    app.regions
        .register_hit_target(HitTarget::CommitMessageGenerate, button);
    let hovered = app.hovered_hit_target == Some(HitTarget::CommitMessageGenerate);
    let running = app.commit_message_running();
    let style = if hovered && !running {
        Style::default()
            .fg(palette().canvas)
            .bg(palette().accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(if running {
                palette().yellow
            } else {
                palette().accent
            })
            .bg(palette().raised)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(if running {
            format!(" {} ", app.commit_message_spinner())
        } else {
            " ✦ ".to_owned()
        })
        .alignment(Alignment::Center)
        .style(style),
        button,
    );
}

pub(super) fn commit_message_text(message: &str) -> Text<'static> {
    Text::from(
        message
            .lines()
            .enumerate()
            .map(|(index, line)| {
                Line::styled(
                    line.to_owned(),
                    Style::default()
                        .fg(palette().ink)
                        .add_modifier(if index == 0 {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                )
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn commit_message_height(message: &str, width: u16, maximum: u16) -> u16 {
    if maximum == 0 {
        return 0;
    }
    let content_height = message
        .lines()
        .map(|line| word_wrapped_height(line, usize::from(width.max(1))))
        .sum::<usize>()
        .max(1)
        .min(usize::from(u16::MAX)) as u16;
    content_height.saturating_add(2).min(maximum)
}
