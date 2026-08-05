use super::super::agents;
use super::*;

pub(in crate::ui) struct AgentPreviewModalRegions {
    pub(in crate::ui) targets: Vec<(HitTarget, Rect)>,
    pub(in crate::ui) scroll_target: Option<(ScrollTarget, Rect)>,
    pub(in crate::ui) scroll: usize,
    pub(in crate::ui) scroll_max: usize,
}

pub(in crate::ui) fn draw_agent_preview_modal(
    frame: &mut Frame<'_>,
    app: &mut App,
    profile: LayoutProfile,
) -> AgentPreviewModalRegions {
    let minimum_width = if profile.is_single() { 40 } else { 88 };
    let mut outer = centered_min(frame.area(), 92, 94, minimum_width, 28);
    outer.width = outer.width.min(132);
    outer.height = outer.height.min(70);
    outer.x = frame.area().x + frame.area().width.saturating_sub(outer.width) / 2;
    outer.y = frame.area().y + frame.area().height.saturating_sub(outer.height) / 2;
    frame.render_widget(Clear, outer);
    fill(frame, outer, palette().panel);

    let header = Rect::new(outer.x, outer.y, outer.width, 3.min(outer.height));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "AGENT PREVIEW",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Structured OpenCode conversation",
                Style::default().fg(palette().soft),
            ),
        ]))
        .style(Style::default().bg(palette().surface_alt)),
        Rect::new(
            header.x.saturating_add(2),
            header.y.saturating_add(1),
            header.width.saturating_sub(4),
            1,
        ),
    );
    let close = Rect::new(
        outer.right().saturating_sub(9),
        outer.y.saturating_add(1),
        7,
        1,
    );
    frame.render_widget(
        Paragraph::new("CLOSE").alignment(Alignment::Right).style(
            Style::default()
                .fg(palette().cyan)
                .bg(palette().surface_alt),
        ),
        close,
    );

    let body_y = header.bottom();
    let footer_height = u16::from(outer.height > 6).saturating_mul(2);
    let body = Rect::new(
        outer.x.saturating_add(3),
        body_y,
        outer.width.saturating_sub(6),
        outer
            .bottom()
            .saturating_sub(body_y)
            .saturating_sub(footer_height),
    );
    let mut targets = vec![
        (HitTarget::AgentPreviewModalOverlay, outer),
        (HitTarget::AgentPreviewModalClose, close),
    ];
    let Some(index) = app.agent_preview_index() else {
        if let Some(run) = app
            .agent_preview_scheduled_run
            .and_then(|id| app.herdr.scheduled_runs().iter().find(|run| run.id == id))
        {
            let messages = run
                .session_id
                .as_deref()
                .and_then(|session| app.herdr.scheduled_conversation(session))
                .unwrap_or_default();
            if messages.is_empty() {
                let text = run
                    .session_id
                    .as_deref()
                    .and_then(|session| app.herdr.scheduled_conversation_error(session))
                    .or(run.error.as_deref())
                    .unwrap_or(if run.status.is_active() {
                        "Waiting for this run's OpenCode session…"
                    } else {
                        "No OpenCode conversation was recorded for this run."
                    });
                frame.render_widget(
                    Paragraph::new(text)
                        .alignment(Alignment::Center)
                        .wrap(Wrap { trim: true })
                        .style(Style::default().fg(palette().faint).bg(palette().panel)),
                    body,
                );
                return AgentPreviewModalRegions {
                    targets,
                    scroll_target: None,
                    scroll: 0,
                    scroll_max: 0,
                };
            }
            let (history_targets, scroll_max, scroll) = agents::draw_scheduled_history(
                frame,
                messages,
                app.scheduler.conversation_message,
                app.scheduler.conversation_scroll,
                &app.scheduler.conversation_expanded_requests,
                app.herdr.spinner_frame(),
                body,
            );
            targets.extend(history_targets);
            return AgentPreviewModalRegions {
                targets,
                scroll_target: Some((ScrollTarget::SchedulerConversation, body)),
                scroll,
                scroll_max,
            };
        }
        frame.render_widget(
            Paragraph::new("Agent is no longer available")
                .alignment(Alignment::Center)
                .style(Style::default().fg(palette().faint).bg(palette().panel)),
            body,
        );
        return AgentPreviewModalRegions {
            targets,
            scroll_target: None,
            scroll: 0,
            scroll_max: 0,
        };
    };
    app.herdr.request_agent_latest_user_message(index);
    let status_area = Rect::new(
        header.x,
        header.y.saturating_add(1),
        header.width.saturating_sub(11),
        1,
    );
    let (history_targets, scroll_max, scroll) = agents::draw_history(
        frame,
        &app.herdr,
        index,
        app.agent_preview_message(index),
        app.agent_preview_transcript_scroll(index),
        app.agent_preview_expanded_requests(index),
        app.agent_preview_picker_open(),
        app.hovered_hit_target.clone(),
        status_area,
        body,
    );
    targets.extend(history_targets);
    let key = app.herdr.agent_key(index);
    let scroll_target = key.map(|key| (ScrollTarget::AgentTranscript(key), body));

    if footer_height > 0 {
        let footer = Rect::new(
            outer.x.saturating_add(2),
            outer.bottom().saturating_sub(2),
            outer.width.saturating_sub(4),
            1,
        );
        frame.render_widget(
            Paragraph::new("↑↓ scroll   PgUp/PgDn page   [ ] messages   Esc close")
                .style(Style::default().fg(palette().faint).bg(palette().panel)),
            footer,
        );
    }
    AgentPreviewModalRegions {
        targets,
        scroll_target,
        scroll,
        scroll_max,
    }
}
