use std::time::Duration;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    AgentDestinationMetadata, AgentEntryState, AgentStatus, HerdrSession, HitTarget,
    LinkedWorktreeCatalog, Settings,
};

use super::{fill, palette, text::word_wrapped_height, truncate_width};

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
    let title = truncate_width(
        &format!("AGENTS {}", herdr.agents.len()),
        usize::from(header.width),
    );
    let separator_width = usize::from(header.width)
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
    frame.render_widget(Paragraph::new(Line::from(header_spans)), header);
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
        Some(HitTarget::Agent(index)) => Some(index),
        Some(HitTarget::AgentTooltip { agent, .. } | HitTarget::AgentMessage { agent, .. }) => {
            Some(agent)
        }
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
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        if gap.bottom() <= list.bottom() {
            draw_agent_gap(frame, gap, background, palette().panel);
        }
    }
    targets
}

pub(super) fn draw_tooltip(
    frame: &mut Frame<'_>,
    herdr: &HerdrSession,
    index: usize,
    selected_message: Option<usize>,
    anchor: Rect,
) -> Vec<(HitTarget, Rect)> {
    let Some(messages) = herdr
        .agent_user_messages(index)
        .filter(|messages| !messages.is_empty())
    else {
        return Vec::new();
    };
    let selected_message = selected_message
        .unwrap_or_else(|| messages.len().saturating_sub(1))
        .min(messages.len().saturating_sub(1));
    let message = &messages[selected_message];
    let screen = frame.area();
    let desired_width = u16::try_from(UnicodeWidthStr::width(message.as_str()).saturating_add(6))
        .unwrap_or(u16::MAX)
        .clamp(32, 50)
        .min(screen.width.saturating_sub(2));
    if desired_width < 6 || screen.height < 3 {
        return Vec::new();
    }

    let right_space = screen.right().saturating_sub(anchor.right());
    let (x, width) = if right_space >= 28 {
        (anchor.right(), desired_width.min(right_space))
    } else if anchor.x > desired_width {
        (anchor.x.saturating_sub(desired_width), desired_width)
    } else {
        (screen.right().saturating_sub(desired_width), desired_width)
    };
    let content_width = usize::from(width.saturating_sub(4).max(1));
    let y = anchor.y.saturating_sub(1);
    let available_height = screen.bottom().saturating_sub(y);
    let height = u16::try_from(word_wrapped_height(message, content_width).saturating_add(5))
        .unwrap_or(u16::MAX)
        .clamp(4, 10)
        .min(available_height);
    if height < 4 {
        return Vec::new();
    }
    let area = Rect::new(x, y, width, height);
    let panel = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Paragraph::new("▄".repeat(usize::from(area.width))).style(
            Style::default()
                .fg(palette().surface_alt)
                .remove_modifier(Modifier::DIM),
        ),
        Rect::new(area.x, area.y, area.width, 1),
    );
    fill(frame, panel, palette().panel);
    let header = Rect::new(panel.x, panel.y, panel.width, 1);
    fill(frame, header, palette().surface_alt);
    let counter = format!("{} / {}", selected_message + 1, messages.len());
    let counter_width = u16::try_from(UnicodeWidthStr::width(counter.as_str())).unwrap_or(u16::MAX);
    let marker_count = messages.len().min(5);
    let marker_width =
        u16::try_from(marker_count.saturating_mul(2).saturating_sub(1)).unwrap_or(u16::MAX);
    let counter_x = area.right().saturating_sub(counter_width).saturating_sub(2);
    let marker_x = counter_x.saturating_sub(marker_width).saturating_sub(2);
    frame.render_widget(
        Paragraph::new("USER MESSAGE").style(
            Style::default()
                .fg(palette().cyan)
                .bg(palette().surface_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            area.x.saturating_add(2),
            header.y,
            marker_x.saturating_sub(area.x).saturating_sub(3),
            1,
        ),
    );
    let mut targets = vec![(
        HitTarget::AgentTooltip {
            agent: index,
            message: selected_message,
        },
        area,
    )];
    for message_index in 0..marker_count {
        let marker = Rect::new(
            marker_x.saturating_add(u16::try_from(message_index).unwrap_or(0).saturating_mul(2)),
            header.y,
            1,
            1,
        );
        let color = if message_index == selected_message {
            palette().yellow
        } else if message_index + 1 == marker_count {
            palette().cyan
        } else {
            palette().muted
        };
        frame.render_widget(
            Paragraph::new("■").style(Style::default().fg(color).bg(palette().surface_alt)),
            marker,
        );
        targets.push((
            HitTarget::AgentMessage {
                agent: index,
                message: message_index,
            },
            marker,
        ));
    }
    frame.render_widget(
        Paragraph::new(counter)
            .alignment(ratatui::layout::Alignment::Right)
            .style(
                Style::default()
                    .fg(palette().accent)
                    .bg(palette().surface_alt),
            ),
        Rect::new(counter_x, header.y, counter_width, 1),
    );
    let body_offset = u16::from(panel.height >= 5) + 1;
    frame.render_widget(
        Paragraph::new(message.as_str())
            .style(Style::default().fg(palette().soft).bg(palette().panel))
            .wrap(Wrap { trim: true }),
        Rect::new(
            area.x.saturating_add(2),
            panel.y.saturating_add(body_offset),
            area.width.saturating_sub(4),
            panel.height.saturating_sub(body_offset).saturating_sub(1),
        ),
    );
    targets
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
