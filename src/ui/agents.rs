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
    AgentActivityPreview, AgentDestinationMetadata, AgentEntryState, AgentStatus, HerdrSession,
    HitTarget, LinkedWorktreeCatalog, Settings,
};

use super::{fill, palette, truncate_width};

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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
        .min(herdr.agents.len().saturating_sub(viewport));
    let hovered_agent = match hovered {
        Some(HitTarget::Agent(index) | HitTarget::AgentStash(index)) => Some(index),
        Some(
            HitTarget::AgentPreviewPicker(agent)
            | HitTarget::AgentPreviewPickerItem(agent)
            | HitTarget::AgentPreviewPrevious(agent)
            | HitTarget::AgentPreviewNext(agent)
            | HitTarget::AgentTooltip { agent, .. }
            | HitTarget::AgentMessage { agent, .. },
        ) => Some(agent),
        _ => None,
    };
    let mut last_card = None;
    for (screen_row, index) in (scroll..herdr.agents.len()).enumerate() {
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
        let is_hovered = hovered_agent == Some(index);
        let background = row_background(&state, is_hovered);
        if row_area.y > list.y {
            let previous_background = index.checked_sub(1).map(|previous| {
                row_background(
                    &herdr.agent_entry_state(previous),
                    hovered_agent == Some(previous),
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
            change_stats,
            elapsed.as_deref(),
            agent.status,
            herdr.spinner_frame(),
            state,
            in_host_tab,
            is_hovered,
        );
        last_card = Some((full_row_area, background));
        targets.push((HitTarget::Agent(index), full_row_area));
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
            targets.push((HitTarget::AgentStash(index), stash));
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
    pressed_navigation: Option<bool>,
    picker_open: bool,
    hovered: Option<HitTarget>,
    area: Rect,
) -> Vec<(HitTarget, Rect)> {
    if area.width < 24 || area.height < 10 {
        return Vec::new();
    }
    fill(frame, area, palette().panel);
    let messages = herdr.agent_user_messages(index).unwrap_or_default();
    let status = herdr
        .agents
        .get(index)
        .map_or(AgentStatus::Unknown, |agent| agent.status);
    let (phase, phase_color) = match status {
        AgentStatus::Working => ("LIVE", palette().orange),
        AgentStatus::Blocked => ("PAUSED", palette().red),
        AgentStatus::Done => ("COMPLETE", palette().green),
        AgentStatus::Idle => ("IDLE", palette().cyan),
        AgentStatus::Unknown => ("UNKNOWN", palette().faint),
    };
    let agent_count = herdr.agents.len();
    let repository = herdr.agent_repository_name(index).unwrap_or("UNKNOWN");
    let desired_navigation_width = badge_width(repository).saturating_add(6);
    let navigation_width = desired_navigation_width
        .min(area.width.saturating_sub(13))
        .max(6)
        .min(area.width);
    frame.render_widget(
        Paragraph::new("CONVERSATION LOG").style(
            Style::default()
                .fg(palette().cyan)
                .bg(palette().panel)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            area.x,
            area.y,
            area.width
                .saturating_sub(navigation_width)
                .saturating_sub(1),
            1,
        ),
    );
    let navigation_x = area.right().saturating_sub(navigation_width);
    let repository_width = navigation_width.saturating_sub(6);
    let repository_area = Rect::new(navigation_x, area.y, repository_width, 1);
    let previous_button = Rect::new(
        repository_area.right(),
        area.y,
        3.min(navigation_width.saturating_sub(repository_width)),
        1,
    );
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
    let mut navigation_targets = Vec::new();
    if repository_area.width >= 3 {
        navigation_targets.push((HitTarget::AgentPreviewPicker(index), repository_area));
    }
    if agent_count > 1 && navigation_width >= 2 {
        navigation_targets.push((HitTarget::AgentPreviewPrevious(index), previous_button));
        navigation_targets.push((HitTarget::AgentPreviewNext(index), next_button));
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
        return navigation_targets;
    }
    let selected_message = selected_message
        .unwrap_or_else(|| messages.len().saturating_sub(1))
        .min(messages.len().saturating_sub(1));
    let message = &messages[selected_message];
    let turn = format!("TURN {} OF {}", selected_message + 1, messages.len());
    let phase_width = u16::try_from(UnicodeWidthStr::width(phase)).unwrap_or(u16::MAX);
    let phase_right = area.right();
    let phase_x = phase_right.saturating_sub(phase_width);
    frame.render_widget(
        Paragraph::new(truncate_width(
            &turn,
            usize::from(phase_x.saturating_sub(area.x).saturating_sub(1)),
        ))
        .style(Style::default().fg(palette().faint).bg(palette().panel)),
        Rect::new(
            area.x,
            area.y.saturating_add(1),
            phase_x.saturating_sub(area.x).saturating_sub(1),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new(phase)
            .alignment(ratatui::layout::Alignment::Right)
            .style(
                Style::default()
                    .fg(phase_color)
                    .bg(palette().panel)
                    .add_modifier(Modifier::BOLD),
            ),
        Rect::new(phase_x, area.y.saturating_add(1), phase_width, 1),
    );
    let mut targets = vec![(
        HitTarget::AgentTooltip {
            agent: index,
            message: selected_message,
        },
        area,
    )];
    targets.extend(navigation_targets);
    let main = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.bottom().saturating_sub(area.y.saturating_add(3)),
    );
    let timeline = Rect::new(main.x, main.y, 2.min(main.width), main.height);
    let marker_count = messages.len().min(usize::from(timeline.height));
    let marker_start = selected_message
        .saturating_sub(marker_count / 2)
        .min(messages.len().saturating_sub(marker_count));
    for visible_index in 0..marker_count {
        let message_index = marker_start + visible_index;
        let offset = u16::try_from(visible_index).unwrap_or(0);
        let marker = Rect::new(
            timeline.x,
            timeline.y.saturating_add(offset),
            timeline.width,
            1,
        );
        let color = if message_index == selected_message {
            palette().yellow
        } else {
            palette().muted
        };
        let symbol = if message_index == selected_message {
            "◉"
        } else {
            "○"
        };
        frame.render_widget(
            Paragraph::new(symbol).style(Style::default().fg(color).bg(palette().panel)),
            Rect::new(marker.x, marker.y, 1, 1),
        );
        targets.push((
            HitTarget::AgentMessage {
                agent: index,
                message: message_index,
            },
            marker,
        ));
    }
    let cards = Rect::new(
        main.x.saturating_add(2),
        main.y,
        main.width.saturating_sub(2),
        main.height,
    );
    let activity_count = message
        .activities
        .len()
        .min(2)
        .min(usize::from(cards.height.saturating_sub(15) / 3));
    let activity_height = u16::try_from(activity_count).unwrap_or(0).saturating_mul(3);
    let message_height = cards.height.min(14);
    let user_height = message_height.div_ceil(2);
    let agent_height = message_height.saturating_sub(user_height);
    let user_card = Rect::new(cards.x, cards.y, cards.width, user_height);
    let user_body = draw_half_cell_card(frame, user_card, palette().surface_alt, palette().panel);
    frame.render_widget(
        Paragraph::new("YOU").style(
            Style::default()
                .fg(palette().yellow)
                .bg(palette().surface_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(user_body.x, user_body.y, user_body.width, 1),
    );
    frame.render_widget(
        Paragraph::new(message.text.as_str())
            .style(
                Style::default()
                    .fg(palette().soft)
                    .bg(palette().surface_alt),
            )
            .wrap(Wrap { trim: true }),
        Rect::new(
            user_body.x,
            user_body.y.saturating_add(1),
            user_body.width,
            user_body.height.saturating_sub(1),
        ),
    );
    let agent_card = Rect::new(cards.x, user_card.bottom(), cards.width, agent_height);
    let agent_body = draw_half_cell_card(frame, agent_card, palette().surface_alt, palette().panel);
    let agent_text = message
        .latest_agent_text
        .as_deref()
        .unwrap_or("Waiting for agent output...");
    frame.render_widget(
        Paragraph::new("AGENT").style(
            Style::default()
                .fg(palette().cyan)
                .bg(palette().surface_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(agent_body.x, agent_body.y, agent_body.width, 1),
    );
    frame.render_widget(
        Paragraph::new(agent_text)
            .style(
                Style::default()
                    .fg(if message.latest_agent_text.is_some() {
                        palette().accent
                    } else {
                        palette().faint
                    })
                    .bg(palette().surface_alt),
            )
            .wrap(Wrap { trim: true }),
        Rect::new(
            agent_body.x,
            agent_body.y.saturating_add(1),
            agent_body.width,
            agent_body.height.saturating_sub(1),
        ),
    );
    let activity_start = message.activities.len().saturating_sub(activity_count);
    for (position, activity) in message.activities[activity_start..].iter().enumerate() {
        let activity_card = Rect::new(
            cards.x,
            agent_card
                .bottom()
                .saturating_add(u16::try_from(position).unwrap_or(0).saturating_mul(3)),
            cards.width,
            3,
        );
        let activity_body =
            draw_half_cell_card(frame, activity_card, palette().surface_alt, palette().panel);
        let newest = position + 1 == activity_count;
        let live =
            status == AgentStatus::Working && selected_message == messages.len().saturating_sub(1);
        let spinner = SPINNER_FRAMES[herdr.spinner_frame() % SPINNER_FRAMES.len()];
        let spans = match activity {
            AgentActivityPreview::Reasoning => {
                let active = newest && live && message.reasoning_active;
                let mut spans = Vec::new();
                if active {
                    spans.push(Span::styled(spinner, Style::default().fg(palette().orange)));
                }
                spans.push(Span::styled(
                    if active { "  REASONING" } else { "REASONING" },
                    Style::default()
                        .fg(palette().orange)
                        .add_modifier(Modifier::BOLD),
                ));
                spans
            }
            AgentActivityPreview::Tool {
                name,
                title,
                running,
            } => {
                let active = *running && newest && live;
                let mut spans = Vec::new();
                if active {
                    spans.push(Span::styled(spinner, Style::default().fg(palette().cyan)));
                }
                spans.push(Span::styled(
                    if active { "  TOOL  " } else { "TOOL  " },
                    Style::default()
                        .fg(palette().cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    name.as_str(),
                    Style::default().fg(palette().accent),
                ));
                if let Some(title) = title {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        title.as_str(),
                        Style::default().fg(palette().soft),
                    ));
                }
                spans
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(palette().surface_alt)),
            activity_body,
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                message.request_count.to_string(),
                Style::default().fg(palette().yellow),
            ),
            Span::styled(" REQUESTS", Style::default().fg(palette().muted)),
            Span::styled("  ", Style::default()),
            Span::styled(
                message.tool_call_count.to_string(),
                Style::default().fg(palette().cyan),
            ),
            Span::styled(" TOOLS", Style::default().fg(palette().muted)),
        ]))
        .style(Style::default().bg(palette().panel)),
        Rect::new(
            cards.x,
            agent_card.bottom().saturating_add(activity_height),
            cards.width,
            1,
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
        &mut targets,
    );
    targets
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
        let hovered = hovered == Some(HitTarget::AgentPreviewPickerItem(index));
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
        targets.push((HitTarget::AgentPreviewPickerItem(index), rect));
    }
}

fn draw_half_cell_card(
    frame: &mut Frame<'_>,
    area: Rect,
    background: Color,
    outer_background: Color,
) -> Rect {
    if area.width < 3 || area.height < 3 {
        return area;
    }
    let edge_style = Style::default()
        .fg(background)
        .bg(outer_background)
        .remove_modifier(Modifier::DIM);
    frame.render_widget(
        Paragraph::new("▄".repeat(usize::from(area.width))).style(edge_style),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new("▀".repeat(usize::from(area.width))).style(edge_style),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    frame.render_widget(
        Paragraph::new("▐\n".repeat(usize::from(inner.height))).style(edge_style),
        Rect::new(area.x, inner.y, 1, inner.height),
    );
    frame.render_widget(
        Paragraph::new("▌\n".repeat(usize::from(inner.height))).style(edge_style),
        Rect::new(area.right().saturating_sub(1), inner.y, 1, inner.height),
    );
    fill(frame, inner, background);
    inner
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
    let stats_gap = u16::from(stats_width > 0);
    let session_x = area.x.saturating_add(1);
    let trailing_x = if stats_width > 0 {
        stats_area.x
    } else {
        area.right()
    };
    let session_width = trailing_x
        .saturating_sub(session_x)
        .saturating_sub(stats_gap);
    frame.render_widget(
        Paragraph::new(truncate_width(session, usize::from(session_width)))
            .style(Style::default().fg(palette().muted).bg(background)),
        Rect::new(session_x, area.y, session_width, 1),
    );
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
