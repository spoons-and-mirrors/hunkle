use super::*;

pub(super) fn draw_header_card(
    frame: &mut Frame<'_>,
    rect: Rect,
    label: &str,
    color: Color,
    active: bool,
    bottom_padding: bool,
) {
    let background = header_card_background(active);
    frame.render_widget(
        Paragraph::new(truncate_width(label, usize::from(rect.width)))
            .style(header_badge_style(color, active)),
        rect,
    );
    draw_half_padding(
        frame,
        Rect::new(rect.x, rect.y.saturating_sub(1), rect.width, 1),
        '▄',
        background,
        palette().canvas,
    );
    draw_header_card_edge(frame, rect.x, rect.y, "▌", color);
    if active {
        draw_header_card_edge(frame, rect.x, rect.y.saturating_sub(1), "▄", color);
    }
    if bottom_padding {
        draw_header_card_bottom(frame, rect, color, active);
    }
}

pub(super) fn draw_header_card_bottom(
    frame: &mut Frame<'_>,
    rect: Rect,
    color: Color,
    active: bool,
) {
    draw_half_padding(
        frame,
        Rect::new(rect.x, rect.bottom(), rect.width, 1),
        '▀',
        header_card_background(active),
        palette().canvas,
    );
    if active {
        draw_header_card_edge(frame, rect.x, rect.bottom(), "▀", color);
    }
}

pub(super) fn header_badge_style(color: Color, active: bool) -> Style {
    Style::default()
        .fg(if active { color } else { palette().ink })
        .bg(header_card_background(active))
        .add_modifier(Modifier::BOLD)
}

pub(super) fn header_card_background(active: bool) -> Color {
    if active {
        palette().raised
    } else {
        palette().panel
    }
}

fn draw_header_card_edge(frame: &mut Frame<'_>, x: u16, y: u16, symbol: &str, color: Color) {
    if let Some(x) = x.checked_sub(1)
        && let Some(cell) = frame.buffer_mut().cell_mut((x, y))
    {
        cell.set_symbol(symbol)
            .set_fg(color)
            .set_bg(palette().canvas);
    }
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
