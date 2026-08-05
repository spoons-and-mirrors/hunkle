use super::*;

pub(super) fn draw_header_card(
    frame: &mut Frame<'_>,
    rect: Rect,
    label: &str,
    color: Color,
    active: bool,
    bottom_padding: bool,
) {
    frame.render_widget(
        Paragraph::new(truncate_width(label, usize::from(rect.width)))
            .style(header_badge_style(color, active)),
        rect,
    );
    draw_half_padding(
        frame,
        Rect::new(rect.x, rect.y.saturating_sub(1), rect.width, 1),
        '▄',
        if active {
            palette().raised
        } else {
            palette().panel
        },
        palette().canvas,
    );
    if let Some(x) = rect.x.checked_sub(1)
        && let Some(cell) = frame.buffer_mut().cell_mut((x, rect.y))
    {
        cell.set_symbol("▌").set_fg(color).set_bg(palette().canvas);
    }
    if active
        && let Some(x) = rect.x.checked_sub(1)
        && let Some(cell) = frame.buffer_mut().cell_mut((x, rect.y.saturating_sub(1)))
    {
        cell.set_symbol("▄").set_fg(color).set_bg(palette().canvas);
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
        if active {
            palette().raised
        } else {
            palette().panel
        },
        palette().canvas,
    );
    if active
        && let Some(x) = rect.x.checked_sub(1)
        && let Some(cell) = frame.buffer_mut().cell_mut((x, rect.bottom()))
    {
        cell.set_symbol("▀").set_fg(color).set_bg(palette().canvas);
    }
}

fn header_badge_style(background: Color, hovered: bool) -> Style {
    let (foreground, background) = if hovered {
        (background, palette().raised)
    } else {
        (palette().ink, palette().panel)
    };
    Style::default()
        .fg(foreground)
        .bg(background)
        .add_modifier(Modifier::BOLD)
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
