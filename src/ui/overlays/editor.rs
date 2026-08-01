use super::*;

pub(crate) fn draw_editor(
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
