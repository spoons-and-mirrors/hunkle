use super::super::{agents, changes};
use super::*;

pub(in crate::ui) struct AgentPreviewModalRegions {
    pub(in crate::ui) targets: Vec<(HitTarget, Rect)>,
    pub(in crate::ui) scroll_target: Option<(ScrollTarget, Rect, usize, usize)>,
    pub(in crate::ui) animation_presented: bool,
}

pub(in crate::ui) fn draw_agent_preview_modal(
    frame: &mut Frame<'_>,
    app: &mut App,
    profile: LayoutProfile,
) -> AgentPreviewModalRegions {
    let minimum_width = if profile.is_single() { 40 } else { 88 };
    let mut outer = if profile.is_single() {
        frame.area()
    } else {
        centered_min(frame.area(), 92, 94, minimum_width, 28)
    };
    if !profile.is_single() {
        outer.width = outer.width.min(132);
        outer.height = outer.height.min(70);
        outer.x = frame.area().x + frame.area().width.saturating_sub(outer.width) / 2;
        outer.y = frame.area().y + frame.area().height.saturating_sub(outer.height) / 2;
    }
    frame.render_widget(Clear, outer);
    fill(frame, outer, palette().panel);

    let selected_index = app.agent_preview_index();
    let header = Rect::new(outer.x, outer.y, outer.width, 3.min(outer.height));
    let header_line = Rect::new(
        header.x.saturating_add(2),
        header.y.saturating_add(1),
        header.width.saturating_sub(4),
        1,
    );
    let mut repository_area = None;
    let close = if profile.is_single() {
        let back_width = agents::badge_width("BACK").min(header_line.width);
        let back = Rect::new(header_line.x, header_line.y, back_width, 1);
        if let Some(index) = selected_index {
            let repository = app.herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
            let repository_width = agents::badge_width(repository).min(
                header_line
                    .width
                    .saturating_sub(back_width.saturating_add(1)),
            );
            if repository_width > 0 {
                let area = Rect::new(
                    header_line.right().saturating_sub(repository_width),
                    header_line.y,
                    repository_width,
                    1,
                );
                repository_area = Some(area);
            }
        }
        back
    } else {
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
            header_line,
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
        close
    };

    let body_y = if profile.is_single() {
        header.bottom().saturating_sub(2)
    } else {
        header.bottom()
    };
    let footer_height = if profile.is_single() {
        0
    } else {
        u16::from(outer.height > 6).saturating_mul(2)
    };
    let body = Rect::new(
        if profile.is_single() {
            outer.x.saturating_add(2)
        } else {
            outer.x.saturating_add(3)
        },
        body_y,
        if profile.is_single() {
            outer.width.saturating_sub(5)
        } else {
            outer.width.saturating_sub(6)
        },
        outer
            .bottom()
            .saturating_sub(body_y)
            .saturating_sub(footer_height),
    );
    let mut targets = vec![
        (HitTarget::AgentPreviewModalBackdrop, frame.area()),
        (HitTarget::AgentPreviewModalOverlay, outer),
        (HitTarget::AgentPreviewModalClose, close),
    ];
    let Some(index) = selected_index else {
        if let Some(run) = app
            .agent_preview
            .scheduled_run
            .and_then(|id| app.scheduled_tasks.runs().iter().find(|run| run.id == id))
        {
            let run_id = run.id;
            let prompt_available = run.session_id.is_some();
            let prompt_sending = run.status.is_active();
            let preview = app
                .agent_preview
                .scheduled_render_state(run.session_id.as_deref());
            let prompt_error = preview.prompt_error.or(run.error.as_deref());
            let conversation_message =
                preview
                    .conversation_error
                    .or(run.error.as_deref())
                    .or_else(|| {
                        preview
                            .transcript
                            .is_none_or(|transcript| transcript.messages.is_empty())
                            .then_some(if run.status.is_active() {
                                "Waiting for this run's OpenCode session…"
                            } else {
                                "No OpenCode conversation was recorded for this run."
                            })
                    });
            let (history_targets, scroll_max, scroll) = agents::draw_scheduled_history(
                frame,
                run_id,
                preview.transcript,
                preview.presentation,
                preview.message,
                preview.scroll,
                preview.expanded_requests,
                preview.user_message_expanded,
                preview.prompt,
                preview.prompt_focused,
                prompt_error,
                prompt_sending,
                prompt_available,
                conversation_message,
                if profile.is_single() { 1 } else { 2 },
                profile.is_single(),
                body,
            );
            targets.extend(history_targets);
            if footer_height > 0 {
                let footer = Rect::new(
                    outer.x.saturating_add(2),
                    outer.bottom().saturating_sub(2),
                    outer.width.saturating_sub(4),
                    1,
                );
                let help = if prompt_available {
                    "Enter message   ↑↓ scroll   [ ] messages   Esc close"
                } else {
                    "↑↓ scroll   [ ] messages   Esc close"
                };
                frame.render_widget(
                    Paragraph::new(help)
                        .style(Style::default().fg(palette().faint).bg(palette().panel)),
                    footer,
                );
            }
            if profile.is_single() {
                agents::draw_badge(frame, close, "BACK", palette().cyan, palette().panel);
            }
            return AgentPreviewModalRegions {
                targets,
                scroll_target: Some((
                    ScrollTarget::AgentScheduledTranscript(run_id),
                    body,
                    scroll,
                    scroll_max,
                )),
                animation_presented: false,
            };
        }
        frame.render_widget(
            Paragraph::new("Agent is no longer available")
                .alignment(Alignment::Center)
                .style(Style::default().fg(palette().faint).bg(palette().panel)),
            body,
        );
        if profile.is_single() {
            agents::draw_badge(frame, close, "BACK", palette().cyan, palette().panel);
        }
        return AgentPreviewModalRegions {
            targets,
            scroll_target: None,
            animation_presented: false,
        };
    };
    let status_area = if profile.is_single() {
        Rect::default()
    } else {
        Rect::new(
            header.x,
            header.y.saturating_add(1),
            header.width.saturating_sub(11),
            1,
        )
    };
    let selected_message = app.agent_preview_message(index);
    let transcript_scroll = app.agent_preview_transcript_scroll(index);
    let expanded_requests = app.agent_preview_expanded_requests(index).to_vec();
    let user_message_expanded = app.agent_preview_user_message_expanded(index);
    let picker_open = app.agent_preview_picker_open();
    let hovered = app.hovered_hit_target.clone();
    let (history_targets, scroll_max, scroll, animation_presented) = agents::draw_history(
        frame,
        &app.herdr,
        &mut app.agent_preview.presentation,
        index,
        selected_message,
        transcript_scroll,
        &expanded_requests,
        user_message_expanded,
        picker_open,
        hovered,
        &app.agent_preview.prompt,
        app.agent_preview.prompt_focused,
        app.agent_preview.prompt_error.as_deref(),
        app.agent_preview.prompt_delivery,
        status_area,
        repository_area,
        if profile.is_single() { 1 } else { 2 },
        u16::from(!profile.is_single()),
        profile.is_single(),
        body,
    );
    targets.extend(history_targets);
    let key = app.herdr.agent_key(index);
    let scroll_target =
        key.map(|key| (ScrollTarget::AgentTranscript(key), body, scroll, scroll_max));

    if let Some((offset, neighbor)) = app.agent_preview_message_swipe(index) {
        let label = format!("message {}", neighbor + 1);
        changes::slide_message_preview(frame, body, offset, &label);
    }

    if let Some(repository_area) = repository_area {
        let repository = app.herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
        agents::draw_badge(
            frame,
            repository_area,
            repository,
            palette().cyan,
            palette().panel,
        );
    }

    if footer_height > 0 {
        let footer = Rect::new(
            outer.x.saturating_add(2),
            outer.bottom().saturating_sub(2),
            outer.width.saturating_sub(4),
            1,
        );
        frame.render_widget(
            Paragraph::new("Enter message   ↑↓ scroll   [ ] messages   Esc close")
                .style(Style::default().fg(palette().faint).bg(palette().panel)),
            footer,
        );
    }
    if profile.is_single() {
        agents::draw_badge(frame, close, "BACK", palette().cyan, palette().panel);
    }
    AgentPreviewModalRegions {
        targets,
        scroll_target,
        animation_presented,
    }
}
