use super::*;

pub(crate) fn draw_herdr_prompt(
    frame: &mut Frame<'_>,
    prompt: &HerdrPrompt,
    shortcuts: &Shortcuts,
) -> Rect {
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
            "Sending…   Esc close".to_owned()
        } else {
            format!(
                "Enter send   Ctrl+U clear   {} / Esc close",
                shortcuts.label(ShortcutAction::OpenHerdr)
            )
        })
        .alignment(Alignment::Right)
        .style(Style::default().fg(palette().muted)),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );
    area
}
