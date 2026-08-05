use super::*;

pub(super) struct LocationPickerAction<'a> {
    pub(super) target: HitTarget,
    pub(super) label: &'a str,
    pub(super) color: Color,
    pub(super) hovered: bool,
}

pub(super) enum LocationPickerRowKind {
    Location { branch: Option<String> },
    Choice,
}

pub(super) struct LocationPickerRow {
    pub(super) target: HitTarget,
    pub(super) label: String,
    pub(super) detail: String,
    pub(super) current: bool,
    pub(super) stats: Option<(u64, u64)>,
    pub(super) kind: LocationPickerRowKind,
    pub(super) selected: bool,
    pub(super) hovered: bool,
    pub(super) delete_target: Option<HitTarget>,
}

pub(super) struct LocationPickerView<'a> {
    pub(super) query: &'a TextInput,
    pub(super) placeholder: &'a str,
    pub(super) rows: &'a [LocationPickerRow],
    pub(super) visible_start: usize,
    pub(super) actions: &'a [LocationPickerAction<'a>],
    pub(super) maximum_width: u16,
    pub(super) overlay_target: HitTarget,
}

pub(super) fn location_picker_capacity(screen: Rect, bounds: Rect, anchor: Rect) -> usize {
    let available = bounds.bottom().saturating_sub(anchor.bottom());
    let maximum_height = screen.height.saturating_mul(4) / 5;
    usize::from(available.min(maximum_height).saturating_sub(4)).max(1)
}

pub(super) fn draw_location_picker(
    frame: &mut Frame<'_>,
    bounds: Rect,
    anchor: Rect,
    view: LocationPickerView<'_>,
) -> (Vec<(HitTarget, Rect)>, Rect) {
    let visible_rows =
        view.rows
            .len()
            .max(1)
            .min(location_picker_capacity(frame.area(), bounds, anchor));
    let width = bounds
        .width
        .min(view.maximum_width)
        .max(12.min(bounds.width));
    let x = anchor
        .x
        .max(bounds.x)
        .min(bounds.right().saturating_sub(width));
    let area = Rect::new(
        x,
        anchor.bottom(),
        width,
        u16::try_from(visible_rows + 4).unwrap_or(u16::MAX),
    );
    frame.render_widget(Clear, area);
    fill(frame, area, palette().surface_alt);

    let action_space = view.actions.iter().fold(0u16, |width, action| {
        width.saturating_add(UnicodeWidthStr::width(action.label) as u16 + 1)
    });
    draw_location_picker_search(frame, view.query, view.placeholder, area, action_space);
    let mut targets = vec![(view.overlay_target.clone(), area)];
    let mut action_x = area.right().saturating_sub(action_space);
    for action in view.actions {
        let width = UnicodeWidthStr::width(action.label) as u16;
        let rect = Rect::new(action_x, area.y + 1, width, 1);
        let background = if action.hovered {
            palette().selected
        } else {
            action.color
        };
        frame.render_widget(
            Paragraph::new(action.label).style(
                Style::default()
                    .fg(palette().canvas)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
        frame.render_widget(
            Paragraph::new("▌").style(Style::default().fg(background).bg(palette().surface_alt)),
            Rect::new(rect.right(), rect.y, 1, 1),
        );
        targets.push((action.target.clone(), rect));
        action_x = rect.right().saturating_add(1);
    }

    let scroll = Rect::new(
        area.x,
        area.y + 3,
        area.width,
        u16::try_from(visible_rows).unwrap_or(u16::MAX),
    );
    for (row, item) in view
        .rows
        .iter()
        .skip(view.visible_start)
        .take(visible_rows)
        .enumerate()
    {
        let rect = Rect::new(scroll.x, scroll.y + row as u16, scroll.width, 1);
        let background = if item.selected || item.hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        match &item.kind {
            LocationPickerRowKind::Location { branch } => draw_location_row(
                frame,
                rect,
                &item.label,
                &item.detail,
                branch.as_deref(),
                item.current,
                item.stats,
                background,
            ),
            LocationPickerRowKind::Choice => draw_choice_row(frame, rect, item, background),
        }
        targets.push((item.target.clone(), rect));
        if let Some(delete_target) = item.delete_target.as_ref().filter(|_| item.hovered) {
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
            targets.push((delete_target.clone(), delete));
        }
    }
    draw_half_padding(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        '▀',
        palette().surface_alt,
        Color::Rgb(0, 0, 0),
    );
    (targets, scroll)
}

pub(super) fn draw_location_picker_search(
    frame: &mut Frame<'_>,
    query: &TextInput,
    placeholder: &str,
    area: Rect,
    action_space: u16,
) {
    let mut value = query.text().to_owned();
    let cursor_visible = query.cursor_visible();
    if !value.is_empty() && cursor_visible {
        value.insert(query.cursor(), '▌');
    }
    let text = if value.is_empty() {
        format!(" {}{placeholder}", if cursor_visible { "▌" } else { " " })
    } else {
        format!(" {value}")
    };
    let search = Rect::new(
        area.x,
        area.y + 1,
        area.width.saturating_sub(action_space),
        1,
    );
    frame.render_widget(
        Paragraph::new(truncate_start_width(&text, usize::from(search.width))).style(
            Style::default()
                .fg(if value.is_empty() {
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
        Rect::new(area.x, area.y + 2, area.width, 1),
        '▀',
        palette().surface_alt,
        palette().surface_alt,
    );
}

fn draw_choice_row(frame: &mut Frame<'_>, rect: Rect, item: &LocationPickerRow, background: Color) {
    fill(frame, rect, background);
    let detail_width =
        (UnicodeWidthStr::width(item.detail.as_str()) as u16).min(rect.width.saturating_sub(5));
    let detail = Rect::new(
        rect.right().saturating_sub(detail_width).saturating_sub(1),
        rect.y,
        detail_width,
        1,
    );
    let label = Rect::new(
        rect.x,
        rect.y,
        detail.x.saturating_sub(rect.x).saturating_sub(1),
        1,
    );
    frame.render_widget(
        Paragraph::new(location_picker_label_line(
            &item.label,
            item.current,
            item.stats,
            usize::from(label.width),
        ))
        .style(Style::default().bg(background)),
        label,
    );
    frame.render_widget(
        Paragraph::new(truncate_start_width(
            &item.detail,
            usize::from(detail.width),
        ))
        .alignment(Alignment::Right)
        .style(Style::default().fg(palette().muted).bg(background)),
        detail,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_location_row(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    path: &str,
    branch: Option<&str>,
    current: bool,
    stats_value: Option<(u64, u64)>,
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
    let name = Rect::new(area.x, area.y, name_width, 1);
    let stats = Rect::new(name.right(), area.y, stats_width, 1);
    let branch_area = Rect::new(stats.right(), area.y, branch_width, 1);
    let path_area = Rect::new(branch_area.right(), area.y, path_width, 1);
    frame.render_widget(
        Paragraph::new(location_picker_label_line(
            label,
            current,
            None,
            usize::from(name.width),
        ))
        .style(Style::default().bg(background)),
        name,
    );
    if let Some((additions, deletions)) = stats_value {
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
            stats,
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

fn location_picker_label_line(
    label: &str,
    current: bool,
    stats: Option<(u64, u64)>,
    width: usize,
) -> Line<'static> {
    let style = Style::default().fg(if current {
        palette().accent
    } else {
        palette().ink
    });
    let prefix = format!(" {} ", if current { "●" } else { " " });
    let stats_text =
        stats.map(|(additions, deletions)| (format!("+{additions}"), format!("-{deletions}")));
    let stats_width = stats_text.as_ref().map_or(0, |(additions, deletions)| {
        UnicodeWidthStr::width(format!("  {additions} {deletions}").as_str())
    });
    let label_width = width.saturating_sub(UnicodeWidthStr::width(prefix.as_str()) + stats_width);
    let mut spans = vec![Span::styled(prefix, style)];
    spans.push(Span::styled(truncate_width(label, label_width), style));
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
