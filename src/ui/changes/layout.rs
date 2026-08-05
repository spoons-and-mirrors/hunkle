use super::*;

pub(super) fn layout_agents_pane(
    app: &mut App,
    content: Rect,
    list_y: u16,
    show_agents: bool,
) -> Rect {
    app.regions.agents_list = None;
    app.regions.agents_splitter = None;
    app.regions.agents_bounds = None;

    let available = content.bottom().saturating_sub(list_y);
    if !show_agents || available < 5 {
        return Rect::new(content.x, list_y, content.width, available);
    }

    let live_count = app.herdr.agent_card_count();
    let agent_count = if app.herdr.showing_stash {
        live_count.max(app.herdr.stashed_agents().len())
    } else {
        live_count
    };
    if app.agents_height_fit_for != Some(agent_count) {
        app.agents_height_fit_for = Some(agent_count);
        app.settings.agents_height = (3 * agent_count).saturating_add(2).clamp(5, 256) as u16;
    }
    let agents_height = app
        .settings
        .agents_height
        .clamp(5, available.saturating_sub(1).max(5));
    let agents_area = Rect::new(
        content.x,
        content.bottom().saturating_sub(agents_height),
        content.width,
        agents_height,
    );
    app.regions.agents_bounds = Some(Rect::new(
        content.x,
        list_y.saturating_add(1),
        content.width,
        available.saturating_sub(1),
    ));
    app.regions.agents_splitter = Some(Rect::new(
        agents_area.x,
        agents_area.y,
        agents_area.width,
        1,
    ));
    app.regions.agents_list = Some(Rect::new(
        agents_area.x.saturating_sub(1),
        agents_area.y.saturating_add(1),
        agents_area.width.saturating_add(2),
        agents_area.height.saturating_sub(1),
    ));
    if let Some(list) = app.regions.agents_list {
        app.regions
            .register_scroll_target(ScrollTarget::Agents, list);
    }
    Rect::new(
        content.x,
        list_y,
        content.width,
        agents_area.y.saturating_sub(list_y),
    )
}

pub(super) fn draw_agents_section(frame: &mut Frame<'_>, app: &mut App) {
    let (Some(header), Some(list)) = (app.regions.agents_splitter, app.regions.agents_list) else {
        return;
    };
    let hovered = app.hovered_hit_target.clone().filter(|target| {
        matches!(
            target,
            HitTarget::Agent(_)
                | HitTarget::AgentPaneId(_)
                | HitTarget::AgentStashToggle
                | HitTarget::AgentStash(_)
                | HitTarget::StashedAgent(_)
                | HitTarget::AgentPreviewPicker(_)
                | HitTarget::AgentPreviewPickerItem(_)
                | HitTarget::AgentPreviewMessageTimeline(_)
                | HitTarget::AgentPreviewRequest { .. }
                | HitTarget::AgentTooltip { .. }
                | HitTarget::AgentMessage { .. }
        )
    });
    for (target, rect) in agents::draw(
        frame,
        &mut app.herdr,
        &app.linked_worktrees,
        &app.settings,
        header,
        list,
        app.dragging_agents,
        hovered,
    ) {
        app.regions.register_hit_target(target, rect);
    }
}

pub(super) struct PreviewLayout {
    pub(super) preview: PreparedPreview,
    pub(super) viewport: Rect,
    pub(super) preview_body: Rect,
    pub(super) outer_scroll: usize,
    pub(super) content_scroll: usize,
    pub(super) leading_height: usize,
    pub(super) total_height: usize,
    pub(super) max_scroll: usize,
    pub(super) scrollbar: Rect,
    pub(super) thumb: Option<Rect>,
}

#[derive(Debug, PartialEq, Eq)]
struct ScrollGeometry {
    outer: usize,
    content: usize,
    preview_offset: usize,
    total: usize,
    max: usize,
}

fn scroll_geometry(
    leading: usize,
    content: usize,
    viewport: usize,
    requested: usize,
    hunk_selected: bool,
) -> ScrollGeometry {
    let total = leading.saturating_add(content);
    let max = total.saturating_sub(viewport);
    let limit = if hunk_selected {
        total.saturating_sub(1)
    } else {
        max
    };
    let outer = requested.min(limit);
    ScrollGeometry {
        outer,
        content: outer.saturating_sub(leading),
        preview_offset: leading.saturating_sub(outer),
        total,
        max,
    }
}

pub(super) fn prepare_preview_layout(
    app: &mut App,
    panel: Rect,
    body: Rect,
    path: &str,
    markdown: bool,
    leading_height: u16,
) -> PreviewLayout {
    let leading_height = usize::from(leading_height);
    let content = app
        .changes
        .preview
        .content()
        .expect("text preview layout requires text content");
    let generation = app.changes.preview.generation();
    let show_initial_diff_header = app.changes.preview.show_file_headers();
    let mut content_scroll = app.changes.diff_scroll.saturating_sub(leading_height);
    let input = PreviewInput {
        content,
        generation,
        path,
        markdown,
        show_initial_diff_header,
        width: usize::from(body.width),
        viewport_height: usize::from(body.height),
        wrapped: app.changes.diff_wrap && app.changes.preview.wrappable(),
    };
    let mut preview = app
        .changes
        .preview_presentation
        .prepare(input, &mut content_scroll);
    let viewport_height = usize::from(body.height);
    let geometry = scroll_geometry(
        leading_height,
        preview.rendered_height,
        viewport_height,
        app.changes.diff_scroll,
        app.changes.hunk_selection.is_some(),
    );
    let outer_scroll = geometry.outer;
    let final_content_scroll = geometry.content;
    if final_content_scroll != content_scroll {
        content_scroll = final_content_scroll;
        preview = app
            .changes
            .preview_presentation
            .prepare(input, &mut content_scroll);
    }
    app.changes.diff_scroll = outer_scroll;
    let preview_body = Rect::new(
        body.x,
        body.y.saturating_add(geometry.preview_offset as u16),
        body.width,
        body.height.saturating_sub(geometry.preview_offset as u16),
    );
    let scrollbar = Rect::new(panel.right().saturating_sub(1), body.y, 1, body.height);
    let thumb = (geometry.max > 0).then(|| {
        diff_scroll_thumb(
            scrollbar,
            geometry.total,
            viewport_height,
            outer_scroll.min(geometry.max),
            geometry.max,
        )
    });
    PreviewLayout {
        preview,
        viewport: body,
        preview_body,
        outer_scroll,
        content_scroll,
        leading_height,
        total_height: geometry.total,
        max_scroll: geometry.max,
        scrollbar,
        thumb,
    }
}

pub(super) fn render_scrollable_content(
    frame: &mut Frame<'_>,
    app: &mut App,
    layout: &mut PreviewLayout,
) {
    debug_assert_eq!(
        layout.total_height,
        layout.leading_height + layout.preview.rendered_height
    );
    app.regions.diff_scroll_max = layout.max_scroll;
    app.regions.diff_scrollbar = Some(layout.scrollbar);
    app.regions.diff_scroll_thumb = layout.thumb;
    if app.regions.preview_body.is_some() {
        app.regions.preview_body = Some(layout.preview_body);
        app.regions.preview_scroll = layout.content_scroll;
    }
    let file_headers = (0..layout
        .preview
        .lines
        .len()
        .min(usize::from(layout.preview_body.height)))
        .filter_map(|row| {
            let document = app.changes.preview.document()?;
            app.changes
                .preview_presentation
                .diff_file_header_at_rendered_row(
                    document,
                    layout.content_scroll.saturating_add(row),
                )
                .map(|destination| (row, destination))
        })
        .collect::<Vec<_>>();
    for (row, (path, line)) in file_headers {
        let index = app.regions.diff_file_headers.len();
        let target = ChangesHitTarget::DiffFileHeader {
            generation: app.changes.preview.generation(),
            index,
        };
        if app.hovered_hit_target == Some(HitTarget::Changes(target)) {
            layout.preview.lines[row].style = layout.preview.lines[row].style.bg(palette().raised);
        }
        let rect = Rect::new(
            layout.preview_body.x,
            layout.preview_body.y.saturating_add(row as u16),
            layout.preview_body.width,
            1,
        );
        app.regions
            .diff_file_headers
            .push(crate::app::DiffFileHeaderRegion { rect, path, line });
        app.regions
            .register_hit_target(HitTarget::Changes(target), rect);
    }
    let paragraph = Paragraph::new(std::mem::take(&mut layout.preview.lines))
        .style(Style::default().bg(palette().panel));
    frame.render_widget(paragraph, layout.preview_body);
    if let Some(thumb) = layout.thumb {
        frame.render_widget(
            Paragraph::new(Text::from(
                (0..layout.scrollbar.height)
                    .map(|_| Line::styled("│", Style::default().fg(palette().faint)))
                    .collect::<Vec<_>>(),
            )),
            layout.scrollbar,
        );
        frame.render_widget(
            Paragraph::new(Text::from(
                (0..thumb.height)
                    .map(|_| {
                        Line::styled(
                            "┃",
                            Style::default().fg(if app.dragging_diff_scrollbar {
                                palette().accent
                            } else {
                                palette().muted
                            }),
                        )
                    })
                    .collect::<Vec<_>>(),
            )),
            thumb,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_geometry_without_metadata_uses_content_range() {
        assert_eq!(
            scroll_geometry(0, 20, 8, 5, false),
            ScrollGeometry {
                outer: 5,
                content: 5,
                preview_offset: 0,
                total: 20,
                max: 12,
            }
        );
    }

    #[test]
    fn scroll_geometry_exposes_partial_and_boundary_metadata() {
        assert_eq!(scroll_geometry(4, 20, 8, 2, false).preview_offset, 2);
        let boundary = scroll_geometry(4, 20, 8, 4, false);
        assert_eq!(boundary.preview_offset, 0);
        assert_eq!(boundary.content, 0);
    }

    #[test]
    fn normal_scroll_clamps_to_full_viewport() {
        let geometry = scroll_geometry(4, 20, 8, usize::MAX, false);
        assert_eq!(geometry.outer, 16);
        assert_eq!(geometry.content, 12);
        assert_eq!(geometry.max, 16);
    }

    #[test]
    fn hunk_scroll_allows_tail_pinning_beyond_normal_max() {
        let geometry = scroll_geometry(4, 20, 8, usize::MAX, true);
        assert_eq!(geometry.outer, 23);
        assert_eq!(geometry.content, 19);
        assert_eq!(geometry.max, 16);
    }
}
