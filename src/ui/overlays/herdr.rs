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

pub(crate) fn draw_agent_pane_picker(
    frame: &mut Frame<'_>,
    prompt: &crate::app::HerdrPrompt,
    hovered: Option<HitTarget>,
) -> Vec<(HitTarget, Rect)> {
    let frame_area = frame.area();
    let area = Rect::new(
        frame_area.x,
        frame_area.y.saturating_add(1),
        frame_area.width,
        frame_area.height.saturating_sub(1),
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

    let inner_x = area.x.saturating_add(2);
    let inner_width = area.width.saturating_sub(4);
    frame.render_widget(
        Paragraph::new("START AGENT")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
        Rect::new(inner_x, area.y.saturating_add(1), inner_width, 1),
    );

    let canvas = Rect::new(
        area.x.saturating_add(3),
        area.y.saturating_add(4),
        area.width.saturating_sub(6),
        area.height.saturating_sub(7),
    );
    let mut targets = vec![(HitTarget::AgentPanePickerOverlay, area)];
    let Some(layout) = prompt.agent_pane_layout() else {
        frame.render_widget(
            Paragraph::new("Reading active Herdr tab layout...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(palette().muted)),
            Rect::new(
                canvas.x,
                canvas.y.saturating_add(canvas.height / 2),
                canvas.width,
                1,
            ),
        );
        return targets;
    };

    let scaled = fit_pane_layout(canvas, layout.width, layout.height);
    let host_pane_id = prompt.agent_host_pane_id().unwrap_or_default();
    let keyboard_focus = prompt.agent_pane_focus();
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
        let is_host = pane.pane_id == host_pane_id;
        let is_hovered = hovered == Some(HitTarget::AgentPane(index));
        let is_selected = keyboard_focus == Some(HitTarget::AgentPane(index));
        let background = if is_hovered || is_selected {
            palette().selected
        } else if is_host {
            palette().surface_alt
        } else {
            palette().raised
        };
        let display_area = Rect::new(
            pane_area.x,
            pane_area.y,
            pane_area.width.saturating_sub(1).max(1),
            pane_area.height.saturating_sub(1).max(1),
        );
        frame.render_widget(
            Block::default().style(Style::default().fg(palette().ink).bg(background)),
            display_area,
        );
        let label = if is_host {
            "HUNKLE"
        } else if is_hovered || is_selected {
            "SELECT"
        } else {
            ""
        };
        if !label.is_empty() && display_area.width >= label.width() as u16 {
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(palette().ink).bg(background)),
                Rect::new(
                    display_area.x,
                    display_area.y.saturating_add(display_area.height / 2),
                    display_area.width,
                    1,
                ),
            );
        }
        if !is_host {
            targets.push((HitTarget::AgentPane(index), pane_area));
        }
        if display_area.width >= 3 && display_area.height >= 3 {
            let horizontal_depth = display_area.height.div_ceil(5);
            let vertical_depth = display_area.width.div_ceil(5);
            let edges = [
                (
                    AgentPaneDirection::Up,
                    Rect::new(
                        display_area.x,
                        display_area.y,
                        display_area.width,
                        horizontal_depth,
                    ),
                ),
                (
                    AgentPaneDirection::Down,
                    Rect::new(
                        display_area.x,
                        display_area.bottom().saturating_sub(horizontal_depth),
                        display_area.width,
                        horizontal_depth,
                    ),
                ),
                (
                    AgentPaneDirection::Left,
                    Rect::new(
                        display_area.x,
                        display_area.y,
                        vertical_depth,
                        display_area.height,
                    ),
                ),
                (
                    AgentPaneDirection::Right,
                    Rect::new(
                        display_area.right().saturating_sub(vertical_depth),
                        display_area.y,
                        vertical_depth,
                        display_area.height,
                    ),
                ),
            ];
            for (direction, edge) in edges {
                let target = HitTarget::AgentPaneSplit(index, direction);
                if hovered.as_ref() == Some(&target) || keyboard_focus.as_ref() == Some(&target) {
                    fill(frame, edge, palette().selected);
                    let plus = Rect::new(
                        edge.x.saturating_add(edge.width / 2),
                        edge.y.saturating_add(edge.height / 2),
                        1,
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new("+")
                            .style(Style::default().fg(palette().ink).bg(palette().selected)),
                        plus,
                    );
                }
                targets.push((target, edge));
            }
        }
    }

    let footer = "ARROWS SELECT CENTER/EDGE · ENTER ACTIVATE · TAB CYCLE · ESC CANCEL";
    frame.render_widget(
        Paragraph::new(truncate_width(footer, usize::from(inner_width)))
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner_x, area.bottom().saturating_sub(1), inner_width, 1),
    );
    targets
}

fn fit_pane_layout(area: Rect, source_width: u16, source_height: u16) -> Rect {
    let width_limited = u32::from(area.width) * u32::from(source_height)
        <= u32::from(area.height) * u32::from(source_width);
    let (width, height) = if width_limited {
        let height = (u32::from(source_height) * u32::from(area.width) / u32::from(source_width))
            .max(1) as u16;
        (area.width, height)
    } else {
        let width = (u32::from(source_width) * u32::from(area.height) / u32::from(source_height))
            .max(1) as u16;
        (width, area.height)
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
    let relative = value.saturating_sub(origin).min(source);
    ((u32::from(relative) * u32::from(target) + u32::from(source) / 2) / u32::from(source)) as u16
}
