use std::time::Duration;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    AgentDestinationMetadata, AgentEntryState, AgentStatus, HerdrSession, HitTarget,
    LinkedWorktreeCatalog, Settings,
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
    hovered: Option<usize>,
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
        let background = row_background(&state, hovered == Some(index));
        if row_area.y > list.y {
            let previous_background = index.checked_sub(1).map(|previous| {
                row_background(
                    &herdr.agent_entry_state(previous),
                    hovered == Some(previous),
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
            hovered == Some(index),
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
    worktree: &'a str,
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
            worktree: "basetree",
            branch: fallback_branch.unwrap_or("unknown"),
        },
        |destination| AgentCardDestination {
            repository: destination.repository(),
            worktree: destination.worktree(),
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
            destination.worktree,
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
    if let Some(elapsed) = elapsed {
        frame.render_widget(
            Paragraph::new(elapsed)
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().fg(palette().soft).bg(background)),
            Rect::new(
                area.right().saturating_sub(time_width).saturating_sub(1),
                area.y,
                time_width,
                1,
            ),
        );
    }
    let available = area
        .right()
        .saturating_sub(time_width)
        .saturating_sub(2)
        .saturating_sub(area.x);
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
    worktree: &str,
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
    let worktree_width = if worktree == "basetree" {
        0
    } else {
        badge_width(worktree).min(18).min(
            stats_area
                .x
                .saturating_sub(area.x)
                .saturating_sub(stats_gap)
                .saturating_sub(4),
        )
    };
    let worktree_area = Rect::new(
        stats_area
            .x
            .saturating_sub(stats_gap)
            .saturating_sub(worktree_width),
        area.y,
        worktree_width,
        1,
    );
    let session_x = area.x.saturating_add(1);
    let trailing_x = if worktree_width > 0 {
        worktree_area.x
    } else if stats_width > 0 {
        stats_area.x
    } else {
        area.right()
    };
    let session_width = trailing_x
        .saturating_sub(session_x)
        .saturating_sub(u16::from(worktree_width > 0 || stats_width > 0));
    frame.render_widget(
        Paragraph::new(truncate_width(session, usize::from(session_width)))
            .style(Style::default().fg(palette().muted).bg(background)),
        Rect::new(session_x, area.y, session_width, 1),
    );
    if worktree_width > 0 {
        draw_badge(frame, worktree_area, worktree, palette().yellow, background);
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
