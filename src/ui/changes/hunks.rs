use super::*;

pub(super) struct VisibleHunk {
    index: usize,
    area: Rect,
    header_y: Option<u16>,
    continues_above: bool,
    continues_below: bool,
    scroll_start: usize,
    scroll_end: usize,
}

pub(super) fn visible_hunks(
    rows: &[(usize, usize)],
    rendered_height: usize,
    body: Rect,
    scroll: usize,
) -> Vec<VisibleHunk> {
    let top = scroll;
    let bottom = top.saturating_add(usize::from(body.height));
    rows.iter()
        .enumerate()
        .filter_map(|(position, (index, header))| {
            let end = rows
                .get(position + 1)
                .map_or(rendered_height, |(_, next)| next.saturating_sub(1));
            let visible_start = (*header).max(top);
            let visible_end = end.min(bottom);
            let scroll_start = *header;
            let scroll_end = end.saturating_sub(usize::from(body.height)).max(*header);
            (visible_start < visible_end).then(|| VisibleHunk {
                index: *index,
                area: Rect::new(
                    body.x,
                    body.y.saturating_add((visible_start - top) as u16),
                    body.width,
                    (visible_end - visible_start) as u16,
                ),
                header_y: (*header >= top && *header < bottom)
                    .then(|| body.y.saturating_add((*header - top) as u16)),
                continues_above: *header < top,
                continues_below: end > bottom,
                scroll_start,
                scroll_end,
            })
        })
        .collect()
}

pub(super) fn scroll_to_row(row: usize, rendered_height: usize) -> usize {
    row.min(rendered_height.saturating_sub(1))
}

pub(super) fn draw_hunk_actions(frame: &mut Frame<'_>, app: &mut App, body: Rect, hunks: Vec<VisibleHunk>) {
    if body.width < 3 {
        return;
    }
    for hunk in hunks {
        let selected = app.changes.hunk_selection == Some(hunk.index);
        if selected && let Some(y) = hunk.header_y {
            frame.buffer_mut().set_style(
                Rect::new(body.x, y, body.width, 1),
                Style::default().bg(palette().selected),
            );
        }
        if let Some(y) = hunk.header_y {
            let rect = Rect::new(body.right().saturating_sub(3), y, 3, 1);
            app.regions.register_hit_target(
                HitTarget::Changes(app.changes.hunk_action_target(hunk.index)),
                rect,
            );
            frame.render_widget(
                Paragraph::new("[+]").style(
                    Style::default()
                        .fg(if selected {
                            palette().ink
                        } else {
                            palette().green
                        })
                        .bg(if selected {
                            palette().accent
                        } else {
                            palette().raised
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                rect,
            );
        }
        app.regions.diff_hunks.push(DiffHunkRegion {
            rect: hunk.area,
            index: hunk.index,
            continues_above: hunk.continues_above,
            continues_below: hunk.continues_below,
            scroll_start: hunk.scroll_start,
            scroll_end: hunk.scroll_end,
        });
    }
}

pub(super) fn diff_scroll_thumb(
    track: Rect,
    content_height: usize,
    viewport_height: usize,
    scroll: usize,
    max_scroll: usize,
) -> Rect {
    let thumb_height = (usize::from(track.height) * viewport_height)
        .checked_div(content_height.max(1))
        .unwrap_or(0)
        .max(1)
        .min(usize::from(track.height)) as u16;
    let travel = track.height.saturating_sub(thumb_height);
    let offset = ((scroll as u128 * u128::from(travel) + max_scroll as u128 / 2)
        .checked_div(max_scroll as u128)
        .unwrap_or(0)) as u16;
    Rect::new(
        track.x,
        track.y.saturating_add(offset),
        track.width,
        thumb_height,
    )
}
