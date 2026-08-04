use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    AgentActivityPreview, AgentDestinationMetadata, AgentEntryState, AgentKey,
    AgentRequestPartPreview, AgentRequestPreview, AgentStatus, AgentUserMessage, HerdrSession,
    HitTarget, LinkedWorktreeCatalog, Settings,
};

use super::{
    fill, palette, preview::hard_wrap_preview_lines, text::styled_markdown_preserving_breaks,
    truncate_width,
};

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct TranscriptBlock {
    user: bool,
    lines: Vec<Line<'static>>,
    start: usize,
    height: usize,
    elapsed: Option<String>,
    request_count: usize,
    request: Option<usize>,
    expandable: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    frame: &mut Frame<'_>,
    herdr: &mut HerdrSession,
    linked_worktrees: &LinkedWorktreeCatalog,
    settings: &Settings,
    header: Rect,
    list: Rect,
    dragging: bool,
    hovered: Option<HitTarget>,
) -> Vec<(HitTarget, Rect)> {
    let mut targets = Vec::new();
    if header.width == 0 || header.height == 0 {
        return targets;
    }
    let toggle_label = if herdr.showing_stash {
        " LIVE "
    } else {
        " STASH "
    };
    let toggle_width = u16::try_from(UnicodeWidthStr::width(toggle_label)).unwrap_or(0);
    let toggle = Rect::new(
        header.right().saturating_sub(toggle_width),
        header.y,
        toggle_width.min(header.width),
        1,
    );
    if header.height == 0 || list.height == 0 {
        return targets;
    }
    let count = if herdr.showing_stash {
        herdr.stashed_agents().len()
    } else {
        herdr.agents.len()
    };
    let section_header = Rect::new(
        header.x,
        header.y,
        toggle.x.saturating_sub(header.x).saturating_sub(1),
        1,
    );
    let title = truncate_width(
        &format!(
            "{} {count}",
            if herdr.showing_stash {
                "STASHED"
            } else {
                "AGENTS"
            }
        ),
        usize::from(section_header.width),
    );
    let separator_width = usize::from(section_header.width)
        .saturating_sub(UnicodeWidthStr::width(title.as_str()).saturating_add(1));
    let mut header_spans = vec![Span::styled(
        title,
        Style::default()
            .fg(palette().cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if separator_width > 0 {
        header_spans.push(Span::raw(" "));
        header_spans.push(Span::styled(
            "─".repeat(separator_width),
            Style::default().fg(if dragging {
                palette().cyan
            } else {
                palette().faint
            }),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), section_header);
    frame.render_widget(
        Paragraph::new(toggle_label).style(
            Style::default()
                .fg(if hovered == Some(HitTarget::AgentStashToggle) {
                    palette().canvas
                } else {
                    palette().cyan
                })
                .bg(if hovered == Some(HitTarget::AgentStashToggle) {
                    palette().selected
                } else {
                    palette().raised
                })
                .add_modifier(Modifier::BOLD),
        ),
        toggle,
    );
    targets.push((HitTarget::AgentStashToggle, toggle));
    if herdr.showing_stash {
        draw_stashed_agents(frame, herdr, list, hovered, &mut targets);
        return targets;
    }
    if herdr.agents.is_empty() {
        let message = herdr.error.as_deref().unwrap_or(if herdr.loading {
            "Loading Herdr agents…"
        } else {
            "No agents detected"
        });
        if list.height > 0 {
            frame.render_widget(
                Paragraph::new(format!(
                    "  {}",
                    truncate_width(message, usize::from(list.width).saturating_sub(2))
                ))
                .style(
                    Style::default()
                        .fg(if herdr.error.is_some() {
                            palette().red
                        } else {
                            palette().faint
                        })
                        .bg(palette().surface_alt),
                ),
                list,
            );
        }
        return targets;
    }

    let card_height = if list.height >= 2 { 2 } else { 1 };
    let card_gap = 1;
    let card_groups = herdr.agent_card_groups();
    let top_padding = u16::from(list.height > card_height);
    let card_list = Rect::new(
        list.x,
        list.y.saturating_add(top_padding),
        list.width.saturating_sub(1),
        list.height.saturating_sub(top_padding),
    );
    let item_step = card_height + card_gap;
    let viewport = usize::from((card_list.height + card_gap) / item_step).max(1);
    let scroll = herdr
        .agent_scroll
        .min(card_groups.len().saturating_sub(viewport));
    let hovered_agent = match hovered.as_ref() {
        Some(HitTarget::Agent(index) | HitTarget::AgentStash(index)) => Some(index),
        Some(
            HitTarget::AgentPreviewPicker(agent)
            | HitTarget::AgentPreviewPickerItem(agent)
            | HitTarget::AgentPreviewPrevious(agent)
            | HitTarget::AgentPreviewNext(agent)
            | HitTarget::AgentPreviewMessageTimeline(agent)
            | HitTarget::AgentPreviewRequest { agent, .. }
            | HitTarget::AgentTooltip { agent, .. }
            | HitTarget::AgentMessage { agent, .. },
        ) => Some(agent),
        _ => None,
    }
    .and_then(|key| herdr.agent_index(key));
    let hovered_card = hovered_agent.and_then(|index| herdr.agent_card_index(index));
    let mut last_card = None;
    for (group_index, (index, agent_count)) in card_groups.iter().copied().enumerate().skip(scroll)
    {
        let screen_row = group_index - scroll;
        if screen_row >= viewport {
            break;
        }
        let offset = u16::try_from(screen_row).unwrap_or(0) * item_step;
        let row_area = Rect::new(
            card_list.x,
            card_list.y.saturating_add(offset),
            card_list.width,
            card_height.min(card_list.height.saturating_sub(offset)),
        );
        let full_row_area = Rect::new(list.x, row_area.y, list.width, row_area.height);
        herdr.request_agent_latest_user_message(index);
        let agent = &herdr.agents[index];
        let Some(agent_key) = herdr.agent_key(index) else {
            continue;
        };
        let state = herdr.agent_entry_state(index);
        let in_host_tab = herdr.agent_is_in_host_tab(index);
        let workspace = herdr
            .workspaces
            .iter()
            .find(|workspace| workspace.id == agent.workspace_id);
        let workspace_name = workspace.map_or("unassigned", |workspace| workspace.label.as_str());
        let destination = agent
            .destination_cwd
            .as_deref()
            .and_then(|path| linked_worktrees.agent_destination(path));
        let destination = agent_card_destination(
            destination,
            workspace_name,
            workspace.and_then(|workspace| workspace.branch.as_deref()),
        );
        let session = herdr
            .agent_display_name(index)
            .unwrap_or("terminal session");
        let elapsed = herdr
            .agent_elapsed(index, settings.agent_time_display)
            .map(format_duration);
        let change_stats = herdr.agent_change_stats(index);
        let is_hovered = hovered_card == Some(group_index);
        let background = row_background(&state, is_hovered);
        if row_area.y > list.y {
            let previous_background = group_index
                .checked_sub(1)
                .and_then(|previous| card_groups.get(previous))
                .map(|(previous, _)| {
                    row_background(
                        &herdr.agent_entry_state(*previous),
                        hovered_card == group_index.checked_sub(1),
                    )
                });
            draw_agent_gap(
                frame,
                Rect::new(list.x, row_area.y - 1, list.width, 1),
                previous_background.unwrap_or(palette().panel),
                background,
            );
        }
        fill(frame, full_row_area, background);
        draw_row(
            frame,
            row_area,
            destination,
            session,
            agent_count,
            change_stats,
            elapsed.as_deref(),
            agent.runtime.status,
            herdr.spinner_frame(),
            state,
            in_host_tab,
            is_hovered,
        );
        last_card = Some((full_row_area, background));
        targets.push((HitTarget::Agent(agent_key.clone()), full_row_area));
        if is_hovered && full_row_area.width >= 7 {
            let stash = Rect::new(full_row_area.right() - 7, full_row_area.y, 7, 1);
            frame.render_widget(
                Paragraph::new(" STASH ").style(
                    Style::default()
                        .fg(palette().canvas)
                        .bg(palette().red)
                        .add_modifier(Modifier::BOLD),
                ),
                stash,
            );
            targets.push((HitTarget::AgentStash(agent_key), stash));
        }
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        if gap.bottom() <= list.bottom() {
            draw_agent_gap(frame, gap, background, palette().panel);
        }
    }
    targets
}

fn draw_stashed_agents(
    frame: &mut Frame<'_>,
    herdr: &mut HerdrSession,
    list: Rect,
    hovered: Option<HitTarget>,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if herdr.stashed_agents().is_empty() {
        frame.render_widget(
            Paragraph::new("  No stashed agents").style(
                Style::default()
                    .fg(palette().faint)
                    .bg(palette().surface_alt),
            ),
            list,
        );
        return;
    }
    let card_height = if list.height >= 2 { 2 } else { 1 };
    let card_gap = 1;
    let top_padding = u16::from(list.height > card_height);
    let card_list = Rect::new(
        list.x,
        list.y.saturating_add(top_padding),
        list.width.saturating_sub(1),
        list.height.saturating_sub(top_padding),
    );
    let item_step = card_height + card_gap;
    let viewport = usize::from((card_list.height + card_gap) / item_step).max(1);
    let scroll = herdr
        .stash_scroll
        .min(herdr.stashed_agents().len().saturating_sub(viewport));
    let hovered_agent = match hovered {
        Some(HitTarget::StashedAgent(index)) => Some(index),
        _ => None,
    };
    let now_ms = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    let mut last_card = None;
    for (screen_row, index) in (scroll..herdr.stashed_agents().len()).enumerate() {
        if screen_row >= viewport {
            break;
        }
        let offset = u16::try_from(screen_row).unwrap_or(0) * item_step;
        let row_area = Rect::new(
            card_list.x,
            card_list.y.saturating_add(offset),
            card_list.width,
            card_height.min(card_list.height.saturating_sub(offset)),
        );
        let full_row_area = Rect::new(list.x, row_area.y, list.width, row_area.height);
        let agent = &herdr.stashed_agents()[index];
        let is_hovered = hovered_agent == Some(index);
        let background = if is_hovered {
            palette().selected
        } else {
            palette().surface_alt
        };
        if row_area.y > list.y {
            let previous_background = index.checked_sub(1).map_or(palette().panel, |previous| {
                if hovered_agent == Some(previous) {
                    palette().selected
                } else {
                    palette().surface_alt
                }
            });
            draw_agent_gap(
                frame,
                Rect::new(list.x, row_area.y - 1, list.width, 1),
                previous_background,
                background,
            );
        }
        fill(frame, full_row_area, background);
        draw_row(
            frame,
            row_area,
            AgentCardDestination {
                repository: &agent.repository_label,
                branch: &agent.branch,
            },
            agent.session_name.as_deref().unwrap_or(&agent.harness),
            1,
            None,
            Some(&format_duration(Duration::from_millis(
                now_ms.saturating_sub(agent.stashed_at_ms),
            ))),
            AgentStatus::Idle,
            herdr.spinner_frame(),
            AgentEntryState::default(),
            false,
            is_hovered,
        );
        last_card = Some((full_row_area, background));
        targets.push((HitTarget::StashedAgent(index), full_row_area));
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        if gap.bottom() <= list.bottom() {
            draw_agent_gap(frame, gap, background, palette().panel);
        }
    }
}

pub(super) fn draw_history(
    frame: &mut Frame<'_>,
    herdr: &HerdrSession,
    index: usize,
    selected_message: Option<usize>,
    transcript_scroll: Option<usize>,
    expanded_requests: &[usize],
    pressed_navigation: Option<bool>,
    picker_open: bool,
    hovered: Option<HitTarget>,
    repository_anchor: Option<Rect>,
    area: Rect,
) -> (Vec<(HitTarget, Rect)>, usize, usize) {
    if area.width < 24 || area.height < 10 {
        return (Vec::new(), 0, 0);
    }
    let Some(agent_key) = herdr.agent_key(index) else {
        return (Vec::new(), 0, 0);
    };
    fill(frame, area, palette().panel);
    let messages = herdr.agent_user_messages(index).unwrap_or_default();
    let status = herdr
        .agents
        .get(index)
        .map_or(AgentStatus::Unknown, |agent| agent.runtime.status);
    let phase = match status {
        AgentStatus::Working => Some(("LIVE", palette().orange)),
        AgentStatus::Blocked => Some(("PAUSED", palette().red)),
        AgentStatus::Done => Some(("COMPLETE", palette().green)),
        AgentStatus::Idle => None,
        AgentStatus::Unknown => Some(("UNKNOWN", palette().faint)),
    };
    let agent_count = herdr.agents.len();
    let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
    let repository_width = badge_width(repository)
        .min(repository_anchor.map_or_else(|| area.width.saturating_sub(6), |anchor| anchor.width));
    let repository_area = repository_anchor.map_or_else(
        || Rect::new(area.x, area.y, repository_width, 1),
        |anchor| Rect::new(anchor.x, anchor.y, repository_width, 1),
    );
    let previous_button = Rect::new(area.right().saturating_sub(6), area.y, 3.min(area.width), 1);
    let next_button = Rect::new(
        previous_button.right(),
        area.y,
        area.right().saturating_sub(previous_button.right()),
        1,
    );
    let button_style = |pressed| {
        Style::default()
            .fg(if agent_count <= 1 {
                palette().faint
            } else if pressed {
                palette().canvas
            } else {
                palette().accent
            })
            .bg(if pressed {
                palette().selected
            } else {
                palette().raised
            })
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(" ← ")
            .alignment(ratatui::layout::Alignment::Center)
            .style(button_style(pressed_navigation == Some(false))),
        previous_button,
    );
    draw_badge(
        frame,
        repository_area,
        repository,
        palette().cyan,
        palette().panel,
    );
    frame.render_widget(
        Paragraph::new(" → ")
            .alignment(ratatui::layout::Alignment::Center)
            .style(button_style(pressed_navigation == Some(true))),
        next_button,
    );
    if let Some((phase, phase_color)) = phase {
        let phase_width = u16::try_from(UnicodeWidthStr::width(phase)).unwrap_or(u16::MAX);
        let phase_x = area.x;
        let phase_right = previous_button.x.saturating_sub(1);
        let phase_area = Rect::new(
            if repository_anchor.is_some() {
                phase_x
            } else {
                repository_area.right().saturating_add(1)
            },
            area.y,
            if repository_anchor.is_some() {
                phase_right.saturating_sub(phase_x)
            } else {
                previous_button
                    .x
                    .saturating_sub(repository_area.right().saturating_add(2))
            },
            1,
        );
        if phase_area.width >= phase_width.saturating_add(2) {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(phase_color)),
                    Span::styled(
                        phase,
                        Style::default()
                            .fg(phase_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]))
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().bg(palette().panel)),
                phase_area,
            );
        }
    }
    let mut navigation_targets = Vec::new();
    if repository_area.width >= 3 {
        navigation_targets.push((
            HitTarget::AgentPreviewPicker(agent_key.clone()),
            repository_area,
        ));
    }
    if agent_count > 1 && area.width >= 6 {
        navigation_targets.push((
            HitTarget::AgentPreviewPrevious(agent_key.clone()),
            previous_button,
        ));
        navigation_targets.push((HitTarget::AgentPreviewNext(agent_key.clone()), next_button));
    }
    if messages.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for conversation history…")
                .style(Style::default().fg(palette().faint).bg(palette().panel))
                .wrap(Wrap { trim: true }),
            Rect::new(
                area.x,
                area.y.saturating_add(2),
                area.width,
                area.height.saturating_sub(2),
            ),
        );
        draw_agent_preview_picker(
            frame,
            herdr,
            index,
            repository_area,
            area,
            picker_open,
            hovered,
            &mut navigation_targets,
        );
        return (navigation_targets, 0, 0);
    }
    let selected_message = selected_message
        .unwrap_or_else(|| messages.len().saturating_sub(1))
        .min(messages.len().saturating_sub(1));
    let message = &messages[selected_message];
    let main = Rect::new(
        area.x,
        area.y.saturating_add(2),
        area.width,
        area.bottom().saturating_sub(area.y.saturating_add(2)),
    );
    let message_selector = Rect::new(main.x, main.y, main.width, 1.min(main.height));
    let content_width = usize::from(main.width.saturating_sub(4).max(1));
    let user_lines = styled_agent_text(&message.text, content_width);
    let desired_user_height = user_lines.len().saturating_add(3).min(8).max(3);
    let user_height = u16::try_from(desired_user_height)
        .unwrap_or(u16::MAX)
        .min(main.bottom().saturating_sub(message_selector.bottom()));
    let user_viewport = Rect::new(
        main.x.saturating_sub(1),
        message_selector.bottom(),
        main.width,
        user_height,
    );
    let viewport = Rect::new(
        main.x.saturating_sub(1),
        user_viewport.bottom(),
        main.width,
        main.bottom().saturating_sub(user_viewport.bottom()),
    );
    let user_cards = Rect::new(
        main.x.saturating_add(1),
        user_viewport.y,
        main.width.saturating_sub(1),
        user_viewport.height,
    );
    let cards = Rect::new(
        main.x.saturating_add(1),
        viewport.y,
        main.width.saturating_sub(1),
        viewport.height,
    );
    let live = status == AgentStatus::Working && selected_message + 1 == messages.len();
    let (blocks, request_height) = build_request_transcript(
        message,
        content_width,
        live,
        herdr.spinner_frame(),
        expanded_requests,
    );
    let scroll_max = request_height.saturating_sub(usize::from(viewport.height));
    let scroll = transcript_scroll.unwrap_or(scroll_max).min(scroll_max);
    let mut targets = vec![(
        HitTarget::AgentTooltip {
            agent: agent_key.clone(),
            message: selected_message,
        },
        area,
    )];
    targets.extend(navigation_targets);
    draw_message_timeline(
        frame,
        message_selector,
        agent_key.clone(),
        selected_message,
        messages.len(),
        &mut targets,
    );
    let user_block = TranscriptBlock {
        user: true,
        lines: user_lines,
        start: 0,
        height: usize::from(user_viewport.height),
        elapsed: message_total_duration(message).map(format_preview_duration),
        request_count: message.requests.len(),
        request: None,
        expandable: false,
    };
    if let Some(rect) = draw_transcript_card(frame, &user_block, user_cards, user_viewport, 0) {
        targets.push((
            HitTarget::AgentMessage {
                agent: agent_key.clone(),
                message: selected_message,
            },
            rect,
        ));
    }
    for block in &blocks {
        if let Some(rect) = draw_transcript_card(frame, block, cards, viewport, scroll)
            && block.expandable
            && let Some(request) = block.request
        {
            targets.push((
                HitTarget::AgentPreviewRequest {
                    agent: agent_key.clone(),
                    message: selected_message,
                    request,
                },
                rect,
            ));
        }
    }
    let progress_viewport = Rect::new(
        main.x.saturating_sub(1),
        user_viewport.y,
        main.width,
        main.bottom().saturating_sub(user_viewport.y),
    );
    draw_transcript_progress(frame, progress_viewport, scroll, scroll_max);
    draw_agent_preview_picker(
        frame,
        herdr,
        index,
        repository_area,
        area,
        picker_open,
        hovered,
        &mut targets,
    );
    (targets, scroll_max, scroll)
}

fn draw_agent_preview_picker(
    frame: &mut Frame<'_>,
    herdr: &HerdrSession,
    selected: usize,
    anchor: Rect,
    bounds: Rect,
    open: bool,
    hovered: Option<HitTarget>,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if !open || anchor.width == 0 || herdr.agents.is_empty() {
        return;
    }
    let y = anchor.bottom();
    let height = u16::try_from(herdr.agents.len())
        .unwrap_or(u16::MAX)
        .min(bounds.bottom().saturating_sub(y));
    if height == 0 {
        return;
    }
    let desired_width = herdr
        .agents
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
            let agent = herdr.agent_display_name(index).unwrap_or("agent");
            UnicodeWidthStr::width(repository)
                .saturating_add(UnicodeWidthStr::width(agent))
                .saturating_add(6)
        })
        .max()
        .unwrap_or(0);
    let width = u16::try_from(desired_width)
        .unwrap_or(u16::MAX)
        .max(anchor.width)
        .min(bounds.width);
    let x = anchor.x.min(bounds.right().saturating_sub(width));
    let popover = Rect::new(x, y, width, height);
    frame.render_widget(Clear, popover);
    fill(frame, popover, palette().raised);
    for (row, index) in (0..herdr.agents.len())
        .take(usize::from(height))
        .enumerate()
    {
        let rect = Rect::new(
            x,
            y.saturating_add(u16::try_from(row).unwrap_or(0)),
            width,
            1,
        );
        let current = index == selected;
        let Some(agent_key) = herdr.agent_key(index) else {
            continue;
        };
        let hovered =
            hovered.as_ref() == Some(&HitTarget::AgentPreviewPickerItem(agent_key.clone()));
        let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
        let agent = herdr.agent_display_name(index).unwrap_or("agent");
        let text = truncate_width(
            &format!(
                " {}  {}  {}",
                if current { "◉" } else { "○" },
                repository,
                agent
            ),
            usize::from(width),
        );
        frame.render_widget(
            Paragraph::new(text).style(
                Style::default()
                    .fg(if current {
                        palette().yellow
                    } else {
                        palette().ink
                    })
                    .bg(if hovered {
                        palette().selected
                    } else {
                        palette().raised
                    })
                    .add_modifier(if current {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            rect,
        );
        targets.push((HitTarget::AgentPreviewPickerItem(agent_key), rect));
    }
}

fn build_request_transcript(
    message: &AgentUserMessage,
    width: usize,
    live: bool,
    spinner_frame: usize,
    expanded_requests: &[usize],
) -> (Vec<TranscriptBlock>, usize) {
    let mut blocks = Vec::new();
    let mut document_height = 0;
    let request_count = message.requests.len();
    for (request_index, request) in message.requests.iter().enumerate() {
        let (lines, content_height) = request_content(
            Some(request),
            width,
            live && request_index + 1 == request_count,
            spinner_frame,
        );
        let full_height = content_height.max(1).saturating_add(2);
        let (mut summary, reasoning, hidden) = request_summary(
            request,
            width,
            live && request_index + 1 == request_count,
            spinner_frame,
        );
        let expandable = hidden > 0;
        let expanded = expanded_requests.contains(&request_index);
        let collapsed = expandable && !expanded;
        let (lines, height) = if collapsed {
            if let Some(reasoning) = reasoning {
                summary.insert(0, reasoning);
            }
            summary.push(Line::styled(
                format!("⌄ {hidden} more"),
                Style::default()
                    .fg(palette().cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            let height = summary.len().saturating_add(2);
            (summary, height)
        } else {
            (lines, full_height)
        };
        blocks.push(TranscriptBlock {
            user: false,
            lines,
            start: document_height,
            height,
            elapsed: request.duration_ms.map(format_preview_duration),
            request_count: 0,
            request: Some(request_index),
            expandable,
        });
        document_height = document_height.saturating_add(height);
    }
    if blocks.is_empty() {
        let (lines, height) = request_content(None, width, live, spinner_frame);
        blocks.push(TranscriptBlock {
            user: false,
            lines,
            start: 0,
            height: height.saturating_add(2),
            elapsed: None,
            request_count: 0,
            request: None,
            expandable: false,
        });
        document_height = height.saturating_add(2);
    }
    (blocks, document_height)
}

fn styled_agent_text(text: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    hard_wrap_preview_lines(
        styled_markdown_preserving_breaks(text, width, true),
        width,
        0,
        usize::MAX,
        false,
        false,
    )
}

fn draw_transcript_card(
    frame: &mut Frame<'_>,
    block: &TranscriptBlock,
    cards: Rect,
    viewport: Rect,
    scroll: usize,
) -> Option<Rect> {
    let visible_start = block.start.max(scroll);
    let visible_end = block
        .start
        .saturating_add(block.height)
        .min(scroll.saturating_add(usize::from(viewport.height)));
    if visible_start >= visible_end {
        return None;
    }
    let local_start = visible_start.saturating_sub(block.start);
    let local_end = visible_end.saturating_sub(block.start);
    let y = viewport
        .y
        .saturating_add(u16::try_from(visible_start.saturating_sub(scroll)).unwrap_or(u16::MAX));
    let visible = Rect::new(
        cards.x,
        y,
        cards.width,
        u16::try_from(visible_end - visible_start).unwrap_or(u16::MAX),
    );
    let background = palette().canvas;
    let accent = if block.user {
        palette().yellow
    } else {
        palette().cyan
    };
    let middle_start = local_start.max(1);
    let middle_end = local_end.min(block.height.saturating_sub(1));
    if middle_start < middle_end {
        let middle = Rect::new(
            cards.x,
            y.saturating_add(u16::try_from(middle_start - local_start).unwrap_or(0)),
            cards.width,
            u16::try_from(middle_end - middle_start).unwrap_or(u16::MAX),
        );
        fill(frame, middle, background);
        frame.render_widget(
            Paragraph::new("┃\n".repeat(usize::from(middle.height))).style(
                Style::default()
                    .fg(accent)
                    .bg(background)
                    .remove_modifier(Modifier::DIM),
            ),
            Rect::new(middle.x, middle.y, 1, middle.height),
        );
    }
    if local_start == 0 {
        frame.render_widget(
            Paragraph::new("▄".repeat(usize::from(cards.width)))
                .style(Style::default().fg(background).bg(palette().panel)),
            Rect::new(cards.x, y, cards.width, 1),
        );
        let label = if block.user {
            let request_word = if block.request_count == 1 {
                "request"
            } else {
                "requests"
            };
            if let Some(elapsed) = block.elapsed.as_deref() {
                Some(format!(
                    " {} {request_word} · total {elapsed} ",
                    block.request_count
                ))
            } else {
                Some(format!(" {} {request_word} ", block.request_count))
            }
        } else {
            block
                .elapsed
                .as_deref()
                .map(|elapsed| format!(" {elapsed} "))
        };
        if let Some(label) = label {
            let width = u16::try_from(UnicodeWidthStr::width(label.as_str())).unwrap_or(u16::MAX);
            if cards.width > width.saturating_add(2) {
                frame.render_widget(
                    Paragraph::new(label).style(
                        Style::default()
                            .fg(accent)
                            .bg(palette().panel)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Rect::new(
                        cards.right().saturating_sub(width).saturating_sub(1),
                        y,
                        width,
                        1,
                    ),
                );
            }
        }
    }
    if block.user && local_start <= 1 && local_end > 1 {
        frame.render_widget(
            Paragraph::new("YOU").style(
                Style::default()
                    .fg(accent)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(
                cards.x.saturating_add(2),
                y.saturating_add(u16::try_from(1_usize.saturating_sub(local_start)).unwrap_or(0)),
                cards.width.saturating_sub(3),
                1,
            ),
        );
    }
    let content_offset = 1 + usize::from(block.user);
    let content_start = local_start.max(content_offset);
    let content_end = local_end.min(block.height.saturating_sub(1));
    if content_start < content_end {
        let content = Rect::new(
            cards.x.saturating_add(2),
            y.saturating_add(u16::try_from(content_start - local_start).unwrap_or(0)),
            cards.width.saturating_sub(3),
            u16::try_from(content_end - content_start).unwrap_or(u16::MAX),
        );
        frame.render_widget(
            Paragraph::new(block.lines.clone())
                .style(Style::default().fg(palette().soft).bg(background))
                .wrap(Wrap { trim: true })
                .scroll((
                    u16::try_from(content_start.saturating_sub(content_offset)).unwrap_or(u16::MAX),
                    0,
                )),
            content,
        );
    }
    if local_end == block.height {
        let bottom_y = y.saturating_add(
            u16::try_from(local_end.saturating_sub(local_start).saturating_sub(1)).unwrap_or(0),
        );
        frame.render_widget(
            Paragraph::new("▀".repeat(usize::from(cards.width)))
                .style(Style::default().fg(background).bg(palette().panel)),
            Rect::new(cards.x, bottom_y, cards.width, 1),
        );
    }
    Some(visible)
}

fn request_content(
    request: Option<&AgentRequestPreview>,
    width: usize,
    live: bool,
    spinner_frame: usize,
) -> (Vec<Line<'static>>, usize) {
    let Some(request) = request else {
        return (
            vec![Line::styled(
                "Waiting for agent output...",
                Style::default().fg(palette().faint),
            )],
            1,
        );
    };
    let mut lines = Vec::new();
    let mut height = 0;
    let mut reasoning_seen = false;
    for part in &request.parts {
        let AgentRequestPartPreview::Activity(activity) = part else {
            let AgentRequestPartPreview::Text(text) = part else {
                unreachable!();
            };
            let text_lines = styled_agent_text(text, width);
            height += text_lines.len().max(1);
            lines.extend(text_lines);
            continue;
        };
        if matches!(activity, AgentActivityPreview::Reasoning) {
            if reasoning_seen {
                continue;
            }
            reasoning_seen = true;
        }
        let line = match activity {
            AgentActivityPreview::Reasoning => reasoning_line(request, live, spinner_frame),
            AgentActivityPreview::Tool { .. } => tool_line(activity, width, live, spinner_frame),
        };
        height += 1;
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "Waiting for agent output...",
            Style::default().fg(palette().faint),
        ));
        height = 1;
    }
    (lines, height)
}

fn request_summary(
    request: &AgentRequestPreview,
    width: usize,
    live: bool,
    spinner_frame: usize,
) -> (Vec<Line<'static>>, Option<Line<'static>>, usize) {
    const TEXT_ROWS: usize = 3;
    const TOOL_ROWS: usize = 3;

    let text = request
        .parts
        .iter()
        .filter_map(|part| match part {
            AgentRequestPartPreview::Text(text) => Some(text),
            AgentRequestPartPreview::Activity(_) => None,
        })
        .flat_map(|text| styled_agent_text(text, width))
        .collect::<Vec<_>>();
    let tools = request
        .parts
        .iter()
        .filter_map(|part| match part {
            AgentRequestPartPreview::Activity(activity @ AgentActivityPreview::Tool { .. }) => {
                Some(tool_line(activity, width, live, spinner_frame))
            }
            AgentRequestPartPreview::Text(_)
            | AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning) => None,
        })
        .collect::<Vec<_>>();
    let reasoning = request.parts.iter().find_map(|part| match part {
        AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning) => {
            Some(reasoning_line(request, live, spinner_frame))
        }
        AgentRequestPartPreview::Text(_)
        | AgentRequestPartPreview::Activity(AgentActivityPreview::Tool { .. }) => None,
    });
    let hidden = text
        .len()
        .saturating_sub(TEXT_ROWS)
        .saturating_add(tools.len().saturating_sub(TOOL_ROWS));
    let lines = text
        .into_iter()
        .take(TEXT_ROWS)
        .chain(tools.into_iter().take(TOOL_ROWS))
        .collect();
    (lines, reasoning, hidden)
}

fn reasoning_line(
    request: &AgentRequestPreview,
    live: bool,
    spinner_frame: usize,
) -> Line<'static> {
    let active = live && request.reasoning_active;
    let prefix = if active {
        SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()]
    } else {
        "›"
    };
    let mut spans = vec![
        Span::styled(prefix.to_owned(), Style::default().fg(palette().orange)),
        Span::styled(
            "  reasoning",
            Style::default()
                .fg(palette().orange)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(duration_ms) = request.reasoning_duration_ms {
        spans.push(Span::styled(
            format!("  {}", format_preview_duration(duration_ms)),
            Style::default().fg(palette().faint),
        ));
    }
    Line::from(spans)
}

fn tool_line(
    activity: &AgentActivityPreview,
    width: usize,
    live: bool,
    spinner_frame: usize,
) -> Line<'static> {
    let AgentActivityPreview::Tool {
        name,
        title,
        running,
    } = activity
    else {
        unreachable!();
    };
    let active = live && *running;
    let prefix = if active {
        SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()]
    } else {
        "›"
    };
    let mut spans = Vec::new();
    let mut remaining = width;
    push_truncated_span(
        &mut spans,
        prefix,
        Style::default().fg(palette().cyan),
        &mut remaining,
    );
    push_truncated_span(
        &mut spans,
        " tool  ",
        Style::default()
            .fg(palette().cyan)
            .add_modifier(Modifier::BOLD),
        &mut remaining,
    );
    push_truncated_span(
        &mut spans,
        name,
        Style::default().fg(palette().accent),
        &mut remaining,
    );
    if let Some(title) = title {
        push_truncated_span(&mut spans, "  ", Style::default(), &mut remaining);
        push_truncated_span(
            &mut spans,
            title,
            Style::default().fg(palette().soft),
            &mut remaining,
        );
    }
    Line::from(spans)
}

fn push_truncated_span(
    spans: &mut Vec<Span<'static>>,
    value: &str,
    style: Style,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    let value = truncate_width(value, *remaining);
    *remaining = remaining.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    spans.push(Span::styled(value, style));
}

fn draw_message_timeline(
    frame: &mut Frame<'_>,
    area: Rect,
    agent: AgentKey,
    message: usize,
    message_count: usize,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if area.height == 0 || area.width == 0 || message_count == 0 {
        return;
    }
    let capacity = usize::from(area.width).div_ceil(2).max(1);
    let visible = message_count.min(capacity);
    let start = message
        .saturating_sub(visible / 2)
        .min(message_count.saturating_sub(visible));
    let timeline_width = u16::try_from(visible.saturating_mul(2).saturating_sub(1))
        .unwrap_or(u16::MAX)
        .min(area.width);
    let timeline = Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(timeline_width) / 2),
        area.y,
        timeline_width,
        1,
    );
    let mut spans = Vec::with_capacity(visible.saturating_mul(2).saturating_sub(1));
    for index in start..start.saturating_add(visible) {
        if index > start {
            spans.push(Span::raw(" "));
        }
        let selected = index == message;
        spans.push(Span::styled(
            if selected { "●" } else { "○" },
            Style::default()
                .fg(if selected {
                    palette().cyan
                } else {
                    palette().faint
                })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(palette().panel)),
        timeline,
    );
    targets.push((HitTarget::AgentPreviewMessageTimeline(agent), area));
}

fn draw_transcript_progress(frame: &mut Frame<'_>, area: Rect, scroll: usize, scroll_max: usize) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("│\n".repeat(usize::from(area.height)))
            .style(Style::default().fg(palette().faint).bg(palette().panel)),
        Rect::new(area.x, area.y, 1, area.height),
    );
    let extent = area.height.saturating_sub(1);
    let offset = scroll
        .saturating_mul(usize::from(extent))
        .checked_div(scroll_max)
        .and_then(|offset| u16::try_from(offset).ok())
        .unwrap_or(extent);
    frame.render_widget(
        Paragraph::new("●").style(Style::default().fg(palette().yellow).bg(palette().panel)),
        Rect::new(area.x, area.y.saturating_add(offset), 1, 1),
    );
}

fn message_total_duration(message: &AgentUserMessage) -> Option<u64> {
    message
        .requests
        .iter()
        .filter_map(|request| request.duration_ms)
        .reduce(u64::saturating_add)
}

fn row_background(state: &AgentEntryState, hovered: bool) -> Color {
    if state.selected || hovered {
        palette().selected
    } else {
        palette().surface_alt
    }
}

#[derive(Clone, Copy)]
struct AgentCardDestination<'a> {
    repository: &'a str,
    branch: &'a str,
}

fn agent_card_destination<'a>(
    destination: Option<AgentDestinationMetadata<'a>>,
    fallback_repository: &'a str,
    fallback_branch: Option<&'a str>,
) -> AgentCardDestination<'a> {
    destination.map_or(
        AgentCardDestination {
            repository: fallback_repository,
            branch: fallback_branch.unwrap_or("unknown"),
        },
        |destination| AgentCardDestination {
            repository: destination.repository(),
            branch: destination.branch(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    frame: &mut Frame<'_>,
    area: Rect,
    destination: AgentCardDestination<'_>,
    session: &str,
    agent_count: usize,
    change_stats: Option<(u64, u64)>,
    elapsed: Option<&str>,
    status: AgentStatus,
    spinner_frame: usize,
    state: AgentEntryState,
    in_host_tab: bool,
    hovered: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let background = row_background(&state, hovered);
    fill(frame, area, background);
    let status_area = draw_agent_status(frame, area, status, spinner_frame, background);
    draw_agent_card_header(
        frame,
        Rect::new(area.x, area.y, status_area.x.saturating_sub(area.x), 1),
        destination,
        elapsed,
        background,
        in_host_tab,
    );
    if area.height > 1 {
        draw_agent_card_detail(
            frame,
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
            session,
            agent_count,
            change_stats,
            background,
        );
    }
}

fn draw_agent_card_header(
    frame: &mut Frame<'_>,
    area: Rect,
    destination: AgentCardDestination<'_>,
    elapsed: Option<&str>,
    background: Color,
    highlighted: bool,
) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let time_width = elapsed
        .map(|time| u16::try_from(UnicodeWidthStr::width(time)).unwrap_or(u16::MAX))
        .unwrap_or(0)
        .min(area.width.saturating_sub(1));
    let elapsed_x = area.right().saturating_sub(time_width).saturating_sub(1);
    if let Some(elapsed) = elapsed {
        frame.render_widget(
            Paragraph::new(elapsed)
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().fg(palette().soft).bg(background)),
            Rect::new(elapsed_x, area.y, time_width, 1),
        );
    }
    let marker_right = elapsed_x.saturating_sub(u16::from(time_width > 0));
    let available = marker_right.saturating_sub(area.x).saturating_sub(1);
    let mut widths = [
        badge_width(destination.repository).min(20),
        badge_width(destination.branch).min(20),
    ];
    while widths[0].saturating_add(widths[1]).saturating_add(1) > available {
        let index = usize::from(widths[1] > widths[0]);
        if widths[index] <= 3 {
            let other = 1 - index;
            if widths[other] <= 3 {
                break;
            }
            widths[other] -= 1;
        } else {
            widths[index] -= 1;
        }
    }
    draw_badge(
        frame,
        Rect::new(area.x, area.y, widths[0].min(available), 1),
        destination.repository,
        if highlighted {
            palette().yellow
        } else {
            palette().cyan
        },
        background,
    );
    let branch_x = area.x.saturating_add(widths[0]).saturating_add(1);
    draw_badge(
        frame,
        Rect::new(
            branch_x,
            area.y,
            widths[1].min(area.x.saturating_add(available).saturating_sub(branch_x)),
            1,
        ),
        destination.branch,
        palette().accent,
        background,
    );
}

fn draw_agent_card_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &str,
    agent_count: usize,
    change_stats: Option<(u64, u64)>,
    background: Color,
) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let stats = change_stats
        .map(|(additions, deletions)| (format!("+{additions}"), format!("-{deletions}")));
    let stats_width = stats.as_ref().map_or(0, |(additions, deletions)| {
        u16::try_from(
            UnicodeWidthStr::width(additions.as_str())
                + 1
                + UnicodeWidthStr::width(deletions.as_str()),
        )
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(2))
    });
    let stats_area = Rect::new(
        area.right().saturating_sub(stats_width),
        area.y,
        stats_width,
        1,
    );
    let group_label = (agent_count > 1).then(|| format!("{agent_count} agents"));
    let group_width = group_label
        .as_ref()
        .map(|label| {
            u16::try_from(UnicodeWidthStr::width(label.as_str()))
                .unwrap_or(u16::MAX)
                .min(area.width.saturating_sub(2))
        })
        .unwrap_or(0);
    let group_right = stats_area.x.saturating_sub(u16::from(stats_width > 0));
    let group_x = group_right.saturating_sub(group_width).max(area.x);
    let group_width = group_right.saturating_sub(group_x);
    let session_x = area.x.saturating_add(1);
    let trailing_x = if group_width > 0 {
        group_x
    } else if stats_width > 0 {
        stats_area.x
    } else {
        area.right()
    };
    let session_gap = u16::from(group_width > 0 || stats_width > 0);
    let session_width = trailing_x
        .saturating_sub(session_x)
        .saturating_sub(session_gap);
    frame.render_widget(
        Paragraph::new(truncate_width(session, usize::from(session_width)))
            .style(Style::default().fg(palette().muted).bg(background)),
        Rect::new(session_x, area.y, session_width, 1),
    );
    if let Some(group_label) = group_label {
        frame.render_widget(
            Paragraph::new(truncate_width(&group_label, usize::from(group_width)))
                .style(Style::default().fg(palette().cyan).bg(background)),
            Rect::new(group_x, area.y, group_width, 1),
        );
    }
    if let Some((additions, deletions)) = stats {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(additions, Style::default().fg(palette().green)),
                Span::raw(" "),
                Span::styled(deletions, Style::default().fg(palette().red)),
            ]))
            .style(Style::default().bg(background)),
            stats_area,
        );
    }
}

fn draw_agent_status(
    frame: &mut Frame<'_>,
    row: Rect,
    status: AgentStatus,
    spinner_frame: usize,
    background: Color,
) -> Rect {
    let area = Rect::new(row.right().saturating_sub(1), row.y, row.width.min(1), 1);
    frame.render_widget(
        Paragraph::new(status_marker(status, spinner_frame)).style(
            Style::default()
                .fg(status_color(status))
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
    area
}

fn draw_agent_gap(frame: &mut Frame<'_>, gap: Rect, above: Color, below: Color) {
    if gap.width > 0 && gap.height > 0 {
        frame.render_widget(
            Paragraph::new("▀".repeat(usize::from(gap.width)))
                .style(Style::default().fg(above).bg(below)),
            gap,
        );
    }
}

fn badge_width(label: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(label).saturating_add(2)).unwrap_or(u16::MAX)
}

fn draw_badge(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    foreground: Color,
    outer_background: Color,
) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("▐").style(Style::default().fg(palette().raised).bg(outer_background)),
        Rect::new(area.x, area.y, 1, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(
            label,
            usize::from(area.width.saturating_sub(2)),
        ))
        .style(
            Style::default()
                .fg(foreground)
                .bg(palette().raised)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            area.x.saturating_add(1),
            area.y,
            area.width.saturating_sub(2),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new("▌").style(Style::default().fg(palette().raised).bg(outer_background)),
        Rect::new(area.right().saturating_sub(1), area.y, 1, 1),
    );
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3_600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86_400 {
        return format_tenths(seconds, 3_600, 'h');
    }
    if seconds < 604_800 {
        return format_tenths(seconds, 86_400, 'd');
    }
    format_tenths(seconds, 604_800, 'w')
}

fn format_preview_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return format!("{duration_ms}ms");
    }
    if duration_ms < 60_000 {
        let tenths = duration_ms.saturating_add(50) / 100;
        return if tenths.is_multiple_of(10) {
            format!("{}s", tenths / 10)
        } else {
            format!("{}.{:01}s", tenths / 10, tenths % 10)
        };
    }
    format_duration(Duration::from_millis(duration_ms))
}

fn format_tenths(seconds: u64, unit: u64, suffix: char) -> String {
    let tenths = seconds.saturating_mul(10).saturating_add(unit / 2) / unit;
    if tenths.is_multiple_of(10) {
        format!("{}{suffix}", tenths / 10)
    } else {
        format!("{}.{}{}", tenths / 10, tenths % 10, suffix)
    }
}

fn status_marker(status: AgentStatus, spinner_frame: usize) -> &'static str {
    match status {
        AgentStatus::Idle => "·",
        AgentStatus::Working => SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()],
        AgentStatus::Blocked => "■",
        AgentStatus::Done => "●",
        AgentStatus::Unknown => "?",
    }
}

fn status_color(status: AgentStatus) -> Color {
    match status {
        AgentStatus::Idle => palette().cyan,
        AgentStatus::Working => palette().orange,
        AgentStatus::Blocked => palette().red,
        AgentStatus::Done => palette().green,
        AgentStatus::Unknown => palette().faint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_agent_durations_as_compact_units() {
        assert_eq!(format_duration(Duration::ZERO), "0s");
        assert_eq!(format_duration(Duration::from_secs(23)), "23s");
        assert_eq!(format_duration(Duration::from_secs(240)), "4m");
        assert_eq!(format_duration(Duration::from_secs(18_360)), "5.1h");
        assert_eq!(format_duration(Duration::from_secs(276_480)), "3.2d");
        assert_eq!(format_duration(Duration::from_secs(665_280)), "1.1w");
    }
}
