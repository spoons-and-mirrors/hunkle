use super::*;

pub(crate) fn draw_action_menu(
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

pub(crate) fn draw_command(frame: &mut Frame<'_>, actions: &mut ActionsState) -> CommandRegions {
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
    ensure_command_layout(actions, usize::from(output.width));
    actions.scroll_max = actions
        .command_layout
        .height
        .saturating_sub(usize::from(output.height))
        .min(usize::from(u16::MAX)) as u16;
    actions.scroll = actions.scroll.min(actions.scroll_max);
    let scroll = usize::from(actions.scroll);
    let first = actions
        .command_layout
        .starts
        .partition_point(|start| *start <= scroll)
        .saturating_sub(1);
    let rendered_end = scroll.saturating_add(usize::from(output.height));
    let end = actions
        .command_layout
        .starts
        .partition_point(|start| *start < rendered_end)
        .max(first.saturating_add(1))
        .min(actions.command_layout.sources.len());
    let lines = actions.command_layout.sources[first..end]
        .iter()
        .map(|source| command_line(actions, source))
        .collect::<Vec<_>>();
    let local_scroll = scroll.saturating_sub(
        actions
            .command_layout
            .starts
            .get(first)
            .copied()
            .unwrap_or_default(),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(local_scroll).unwrap_or(u16::MAX), 0))
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
