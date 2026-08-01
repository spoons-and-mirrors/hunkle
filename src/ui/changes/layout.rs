use super::*;

pub(super) fn layout_agents_pane(app: &mut App, content: Rect, list_y: u16) -> Rect {
    app.regions.agents_list = None;
    app.regions.agents_splitter = None;
    app.regions.agents_bounds = None;

    let available = content.bottom().saturating_sub(list_y);
    if !app.agents_visible || available < 5 {
        return Rect::new(content.x, list_y, content.width, available);
    }

    let agents_height = app
        .settings
        .agents_height
        .clamp(3, available.saturating_sub(2));
    let agents_area = Rect::new(
        content.x,
        content.bottom().saturating_sub(agents_height),
        content.width,
        agents_height,
    );
    app.regions.agents_bounds = Some(Rect::new(
        content.x,
        list_y.saturating_add(2),
        content.width,
        available.saturating_sub(2),
    ));
    app.regions.agents_splitter = Some(Rect::new(
        agents_area.x,
        agents_area.y,
        agents_area.width,
        1,
    ));
    app.regions.agents_list = Some(Rect::new(
        agents_area.x,
        agents_area.y.saturating_add(1),
        agents_area.width,
        agents_area.height.saturating_sub(1),
    ));
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
    let hovered = match &app.hovered_hit_target {
        Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(index))) => Some(*index),
        _ => None,
    };
    let enabled = app.workspace_panel_enabled();
    for (target, rect) in workspace_panel::draw_agents_pane(
        frame,
        &mut app.workspace_panel,
        &app.settings,
        header,
        list,
        app.dragging_agents,
        enabled,
        hovered,
    ) {
        app.regions.register_hit_target(target, rect);
    }
}

pub(super) fn prepare_preview_lines(
    app: &mut App,
    body: Rect,
    path: &str,
    is_diff: bool,
    show_initial_diff_header: bool,
    markdown: bool,
    leading_height: u16,
) -> PreparedPreview {
    let mut content_scroll = app
        .changes
        .diff_scroll
        .saturating_sub(usize::from(leading_height));
    let preview = app.changes.preview_presentation.prepare(
        PreviewInput {
            content: &app.changes.diff,
            generation: app.changes.preview_content_generation,
            path,
            is_diff,
            markdown,
            show_initial_diff_header,
            width: usize::from(body.width),
            viewport_height: usize::from(body.height),
            wrapped: app.changes.diff_wrap,
            hunk_selected: app.changes.hunk_selection.is_some(),
        },
        &mut content_scroll,
    );
    if app.changes.diff_scroll >= usize::from(leading_height) {
        app.changes.diff_scroll = usize::from(leading_height).saturating_add(content_scroll);
    }
    preview
}

pub(super) fn render_scrollable_content(
    frame: &mut Frame<'_>,
    app: &mut App,
    panel: Rect,
    body: Rect,
    mut preview: PreparedPreview,
    leading_height: u16,
) {
    let rendered_height = preview.rendered_height;
    let viewport_height = usize::from(body.height);
    let total_height = usize::from(leading_height).saturating_add(rendered_height);
    let max_scroll = total_height.saturating_sub(viewport_height);
    let scroll_limit = if app.changes.hunk_selection.is_some() {
        total_height.saturating_sub(1)
    } else {
        max_scroll
    };
    app.regions.diff_scroll_max = max_scroll;
    app.changes.diff_scroll = app.changes.diff_scroll.min(scroll_limit);
    let scrollbar = Rect::new(panel.right().saturating_sub(1), body.y, 1, body.height);
    app.regions.diff_scrollbar = Some(scrollbar);
    app.regions.diff_scroll_thumb = (max_scroll > 0).then(|| {
        diff_scroll_thumb(
            scrollbar,
            total_height,
            viewport_height,
            app.changes.diff_scroll.min(max_scroll),
            max_scroll,
        )
    });
    let preview_offset = usize::from(leading_height).saturating_sub(app.changes.diff_scroll);
    let preview_body = Rect::new(
        body.x,
        body.y.saturating_add(preview_offset as u16),
        body.width,
        body.height.saturating_sub(preview_offset as u16),
    );
    let content_scroll = app
        .changes
        .diff_scroll
        .saturating_sub(usize::from(leading_height));
    let file_headers = (0..preview.lines.len().min(usize::from(preview_body.height)))
        .filter_map(|row| {
            app.changes
                .preview_presentation
                .diff_file_header_at_rendered_row(
                    &app.changes.diff,
                    content_scroll.saturating_add(row),
                )
                .map(|destination| (row, destination))
        })
        .collect::<Vec<_>>();
    for (row, (path, line)) in file_headers {
        let index = app.regions.diff_file_headers.len();
        let target = ChangesHitTarget::DiffFileHeader {
            generation: app.changes.preview_content_generation,
            index,
        };
        if app.hovered_hit_target == Some(HitTarget::Changes(target)) {
            preview.lines[row].style = preview.lines[row].style.bg(palette().raised);
        }
        let rect = Rect::new(
            preview_body.x,
            preview_body.y.saturating_add(row as u16),
            preview_body.width,
            1,
        );
        app.regions
            .diff_file_headers
            .push(crate::app::DiffFileHeaderRegion { rect, path, line });
        app.regions
            .register_hit_target(HitTarget::Changes(target), rect);
    }
    let paragraph = Paragraph::new(preview.lines).style(Style::default().bg(palette().panel));
    frame.render_widget(paragraph, preview_body);
    if let Some(thumb) = app.regions.diff_scroll_thumb {
        frame.render_widget(
            Paragraph::new(Text::from(
                (0..scrollbar.height)
                    .map(|_| Line::styled("│", Style::default().fg(palette().faint)))
                    .collect::<Vec<_>>(),
            )),
            scrollbar,
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
