use super::*;

pub(super) fn draw_scrolled_summary_card(
    frame: &mut Frame<'_>,
    viewport: Rect,
    scroll: usize,
    height: u16,
    summary: Option<&DiffSummary>,
    summary_unavailable: bool,
    summary_height: u16,
) {
    let scroll = scroll.min(usize::from(u16::MAX)) as u16;
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
    viewport: Rect,
    scroll: usize,
    metadata: CommitMetadata<'_>,
) {
    let scroll = scroll.min(usize::from(u16::MAX)) as u16;
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
