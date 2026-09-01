use ratatui::widgets::Block;

use super::*;
use crate::app::layout_configuration::{
    LayoutConfiguration, LayoutFocus, PaneDirection, PaneLayout,
};

pub(crate) fn draw_layout_configuration(
    frame: &mut Frame<'_>,
    configuration: &LayoutConfiguration,
    hovered: Option<LayoutFocus>,
) -> Vec<(LayoutFocus, Rect)> {
    let area = Rect::new(
        frame.area().x,
        frame.area().y.saturating_add(2),
        frame.area().width,
        frame.area().height.saturating_sub(2),
    );
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
        Paragraph::new("CONFIGURE LAYOUT")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
        Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            area.width.saturating_sub(4),
            1,
        ),
    );

    let canvas = Rect::new(
        area.x.saturating_add(3),
        area.y.saturating_add(4),
        area.width.saturating_sub(6),
        area.height.saturating_sub(7),
    );
    let Some(layout) = configuration.layout() else {
        frame.render_widget(
            Paragraph::new("Reading layout...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(palette().muted)),
            Rect::new(
                canvas.x,
                canvas.y.saturating_add(canvas.height / 2),
                canvas.width,
                1,
            ),
        );
        return Vec::new();
    };

    let scaled = fit_pane_layout(canvas, layout);
    let mut targets = Vec::new();
    for (index, pane) in layout.panes.iter().enumerate() {
        let left = scale_pane_edge(pane.x, layout.x, layout.width, scaled.width);
        let top = scale_pane_edge(pane.y, layout.y, layout.height, scaled.height);
        let right = scale_pane_edge(
            pane.x.saturating_add(pane.width),
            layout.x,
            layout.width,
            scaled.width,
        )
        .max(left.saturating_add(1));
        let bottom = scale_pane_edge(
            pane.y.saturating_add(pane.height),
            layout.y,
            layout.height,
            scaled.height,
        )
        .max(top.saturating_add(1));
        let pane_area = Rect::new(
            scaled.x.saturating_add(left),
            scaled.y.saturating_add(top),
            right
                .saturating_sub(left)
                .min(scaled.width.saturating_sub(left)),
            bottom
                .saturating_sub(top)
                .min(scaled.height.saturating_sub(top)),
        );
        let pane_focus = LayoutFocus::Pane(index);
        let protected = configuration.protected_pane_id() == Some(pane.id.as_str());
        let selected = hovered == Some(pane_focus) || configuration.focus() == Some(pane_focus);
        let background = if selected {
            palette().selected
        } else if protected {
            palette().surface_alt
        } else {
            palette().raised
        };
        let display = Rect::new(
            pane_area.x,
            pane_area.y,
            pane_area.width.saturating_sub(1).max(1),
            pane_area.height.saturating_sub(1).max(1),
        );
        frame.render_widget(
            Block::default().style(Style::default().fg(palette().ink).bg(background)),
            display,
        );
        if !protected {
            targets.push((pane_focus, pane_area));
        }
        if display.width >= 3 && display.height >= 3 {
            let horizontal_depth = display.height.div_ceil(5);
            let vertical_depth = display.width.div_ceil(5);
            let edges = [
                (
                    PaneDirection::Up,
                    Rect::new(display.x, display.y, display.width, horizontal_depth),
                ),
                (
                    PaneDirection::Down,
                    Rect::new(
                        display.x,
                        display.bottom().saturating_sub(horizontal_depth),
                        display.width,
                        horizontal_depth,
                    ),
                ),
                (
                    PaneDirection::Left,
                    Rect::new(display.x, display.y, vertical_depth, display.height),
                ),
                (
                    PaneDirection::Right,
                    Rect::new(
                        display.right().saturating_sub(vertical_depth),
                        display.y,
                        vertical_depth,
                        display.height,
                    ),
                ),
            ];
            for (direction, edge) in edges {
                let target = LayoutFocus::Split(index, direction);
                if hovered == Some(target) || configuration.focus() == Some(target) {
                    fill(frame, edge, palette().selected);
                    frame.render_widget(
                        Paragraph::new("+")
                            .style(Style::default().fg(palette().ink).bg(palette().selected)),
                        Rect::new(
                            edge.x.saturating_add(edge.width / 2),
                            edge.y.saturating_add(edge.height / 2),
                            1,
                            1,
                        ),
                    );
                }
                targets.push((target, edge));
            }
        }
    }

    frame.render_widget(
        Paragraph::new("ARROWS SELECT CENTER/EDGE | ENTER ACTIVATE | TAB CYCLE | ESC CANCEL")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().muted)),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    targets
}

fn fit_pane_layout(area: Rect, layout: &PaneLayout) -> Rect {
    let source_width = layout.width.max(1);
    let source_height = layout.height.max(1);
    let width_limited = u32::from(area.width) * u32::from(source_height)
        <= u32::from(area.height) * u32::from(source_width);
    let (width, height) = if width_limited {
        (
            area.width,
            (u32::from(source_height) * u32::from(area.width) / u32::from(source_width)).max(1)
                as u16,
        )
    } else {
        (
            (u32::from(source_width) * u32::from(area.height) / u32::from(source_height)).max(1)
                as u16,
            area.height,
        )
    };
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

fn scale_pane_edge(value: u16, origin: u16, source: u16, target: u16) -> u16 {
    let source = source.max(1);
    let relative = value.saturating_sub(origin).min(source);
    ((u32::from(relative) * u32::from(target) + u32::from(source) / 2) / u32::from(source)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_geometry_preserves_aspect_ratio_and_bounds() {
        let layout = PaneLayout {
            x: 10,
            y: 20,
            width: 200,
            height: 100,
            panes: Vec::new(),
        };
        let fitted = fit_pane_layout(Rect::new(5, 7, 30, 30), &layout);

        assert_eq!(fitted, Rect::new(5, 14, 30, 15));
        assert_eq!(scale_pane_edge(10, 10, 200, fitted.width), 0);
        assert_eq!(scale_pane_edge(110, 10, 200, fitted.width), 15);
        assert_eq!(scale_pane_edge(210, 10, 200, fitted.width), 30);
    }
}
