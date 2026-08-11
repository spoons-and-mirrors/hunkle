use super::*;

pub(super) fn draw_scrolled_summary_card(
    frame: &mut Frame<'_>,
    layout: &PreviewLayout,
    height: u16,
    summary: Option<&DiffSummary>,
    summary_unavailable: bool,
    summary_height: u16,
) {
    let viewport = layout.viewport;
    let scroll = layout.outer_scroll.min(usize::from(u16::MAX)) as u16;
    if scroll >= height {
        return;
    }
    let visible_height = height.saturating_sub(scroll).min(viewport.height);
    let card = Rect::new(viewport.x, viewport.y, viewport.width, visible_height);
    fill(frame, card, palette().surface_alt);
    let content_width = card.width.saturating_sub(2);
    draw_scrolled_text(
        frame,
        card,
        Rect::new(
            card.x.saturating_add(1),
            1,
            content_width,
            summary_height.saturating_sub(1),
        ),
        scroll,
        diff_summary_text(
            summary,
            summary_unavailable,
            true,
            content_width,
            summary_height.saturating_sub(1),
            true,
        ),
        false,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_scrolled_pull_request_card(
    frame: &mut Frame<'_>,
    layout: &PreviewLayout,
    height: u16,
    base_ref: &str,
    head_ref: &str,
    changed_files: Option<u64>,
    additions: Option<u64>,
    deletions: Option<u64>,
    body: &[Line<'static>],
) {
    let viewport = layout.viewport;
    let scroll = layout.outer_scroll.min(usize::from(u16::MAX)) as u16;
    if scroll >= height {
        return;
    }
    let visible_height = height.saturating_sub(scroll).min(viewport.height);
    let card = Rect::new(viewport.x, viewport.y, viewport.width, visible_height);
    fill(frame, card, palette().surface_alt);
    let content_x = card.x.saturating_add(1);
    let content_width = card.width.saturating_sub(2);
    draw_scrolled_text(
        frame,
        card,
        Rect::new(content_x, 1, content_width, 1),
        scroll,
        Text::from(pull_request_metadata_line(
            base_ref,
            head_ref,
            changed_files,
            additions,
            deletions,
        )),
        false,
    );
    draw_scrolled_text(
        frame,
        card,
        Rect::new(content_x, 3, content_width, 1),
        scroll,
        Text::from(Line::styled(
            "DESCRIPTION",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        false,
    );
    let body_y = card.y.saturating_add(5_u16.saturating_sub(scroll));
    let visible_height = card.bottom().saturating_sub(body_y);
    if visible_height > 0 {
        let first = usize::from(scroll.saturating_sub(5));
        let visible = body
            .iter()
            .skip(first)
            .take(usize::from(visible_height))
            .cloned()
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(visible)),
            Rect::new(content_x, body_y, content_width, visible_height),
        );
    }
}

fn pull_request_metadata_line(
    base_ref: &str,
    head_ref: &str,
    changed_files: Option<u64>,
    additions: Option<u64>,
    deletions: Option<u64>,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            "CHANGES ",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(head_ref.to_owned(), Style::default().fg(palette().accent)),
        Span::styled(" -> ", Style::default().fg(palette().faint)),
        Span::styled(base_ref.to_owned(), Style::default().fg(palette().ink)),
    ];
    if let Some(files) = changed_files {
        spans.push(Span::styled(
            format!("  {files} files"),
            Style::default().fg(palette().muted),
        ));
    }
    if let Some(additions) = additions {
        spans.push(Span::styled(
            format!(" +{additions}"),
            Style::default().fg(palette().green),
        ));
    }
    if let Some(deletions) = deletions {
        spans.push(Span::styled(
            format!(" -{deletions}"),
            Style::default().fg(palette().red),
        ));
    }
    Line::from(spans)
}

pub(super) struct CommitMetadata<'a> {
    pub(super) height: u16,
    pub(super) commit: &'a Commit,
    pub(super) message: &'a str,
    pub(super) message_height: u16,
    pub(super) summary: Option<&'a DiffSummary>,
    pub(super) summary_unavailable: bool,
    pub(super) summary_height: u16,
}

pub(super) fn draw_scrolled_metadata_card(
    frame: &mut Frame<'_>,
    layout: &PreviewLayout,
    metadata: CommitMetadata<'_>,
) {
    let viewport = layout.viewport;
    let scroll = layout.outer_scroll.min(usize::from(u16::MAX)) as u16;
    if scroll >= metadata.height {
        return;
    }
    let visible_height = metadata.height.saturating_sub(scroll).min(viewport.height);
    let card = Rect::new(viewport.x, viewport.y, viewport.width, visible_height);
    fill(frame, card, palette().surface_alt);

    let content_x = card.x.saturating_add(1);
    let content_width = card.width.saturating_sub(2);
    draw_scrolled_text(
        frame,
        card,
        Rect::new(content_x, 1, content_width, 1),
        scroll,
        Text::from(commit_metadata_line(
            metadata.commit,
            metadata.summary,
            metadata.summary_unavailable,
        )),
        false,
    );
    let mut message_lines = vec![Line::styled(
        "MESSAGE",
        Style::default()
            .fg(palette().muted)
            .add_modifier(Modifier::BOLD),
    )];
    message_lines.extend(commit_message_text(metadata.message).lines);
    draw_scrolled_text(
        frame,
        card,
        Rect::new(
            content_x,
            3,
            content_width,
            metadata.message_height.saturating_sub(1),
        ),
        scroll,
        Text::from(message_lines),
        true,
    );
    draw_scrolled_text(
        frame,
        card,
        Rect::new(
            content_x,
            metadata.message_height.saturating_add(3),
            content_width,
            metadata.summary_height.saturating_sub(2),
        ),
        scroll,
        diff_summary_text(
            metadata.summary,
            metadata.summary_unavailable,
            true,
            content_width,
            metadata.summary_height.saturating_sub(2),
            false,
        ),
        false,
    );
}

pub(super) fn commit_metadata_line(
    commit: &Commit,
    summary: Option<&DiffSummary>,
    summary_unavailable: bool,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            "COMMIT  ",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            commit.oid.chars().take(7).collect::<String>(),
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}  {}", commit.author, commit.date),
            Style::default().fg(palette().soft),
        ),
    ];
    if let Some(summary) = summary {
        spans.extend([
            Span::styled(
                format!("  +{}", summary.additions),
                Style::default().fg(palette().green),
            ),
            Span::styled(
                format!("  -{}", summary.deletions),
                Style::default().fg(palette().red),
            ),
            Span::styled(
                format!(
                    "  {}{} {}",
                    summary.files.len(),
                    if summary.files_truncated { "+" } else { "" },
                    if summary.files.len() == 1 {
                        "file"
                    } else {
                        "files"
                    }
                ),
                Style::default().fg(palette().faint),
            ),
        ]);
    } else {
        spans.push(Span::styled(
            if summary_unavailable {
                "  summary unavailable"
            } else {
                "  summary loading…"
            },
            Style::default().fg(palette().faint),
        ));
    }
    Line::from(spans)
}

pub(super) fn draw_scrolled_text(
    frame: &mut Frame<'_>,
    viewport: Rect,
    content: Rect,
    scroll: u16,
    text: Text<'static>,
    wrapped: bool,
) {
    let visible_start = content.y.max(scroll);
    let visible_end = content.bottom().min(scroll.saturating_add(viewport.height));
    if visible_start >= visible_end {
        return;
    }
    let area = Rect::new(
        content.x,
        viewport
            .y
            .saturating_add(visible_start.saturating_sub(scroll)),
        content.width,
        visible_end.saturating_sub(visible_start),
    );
    let paragraph = Paragraph::new(text).scroll((visible_start.saturating_sub(content.y), 0));
    if wrapped {
        frame.render_widget(paragraph.wrap(Wrap { trim: false }), area);
    } else {
        frame.render_widget(paragraph, area);
    }
}
