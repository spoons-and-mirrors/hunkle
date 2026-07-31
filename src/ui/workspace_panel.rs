use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use std::{path::Path, time::Duration};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    AgentStatus, HitTarget, Settings, WorkspaceDropTarget, WorkspacePanel,
    WorkspacePanelEntryState, WorkspacePanelHitTarget, WorkspacePanelRow,
};

#[cfg(test)]
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

use super::{fill, palette, truncate_width};

const HEADER_HEIGHT: u16 = 4;
const FOOTER_HEIGHT: u16 = 3;
const DRAWER_MIN_HEIGHT: u16 = 17;
const DRAWER_MAX_HEIGHT: u16 = 20;

fn chrome_heights(area: Rect) -> (u16, u16) {
    if area.width < 82 || area.height < 20 {
        (2, 2)
    } else {
        (HEADER_HEIGHT, FOOTER_HEIGHT)
    }
}

pub(super) fn drawer_area(screen: Rect) -> Rect {
    let height = if screen.height >= DRAWER_MIN_HEIGHT + 5 {
        (screen.height / 2).clamp(DRAWER_MIN_HEIGHT, DRAWER_MAX_HEIGHT)
    } else {
        screen.height.saturating_sub(2).max(1)
    }
    .min(screen.height);
    Rect::new(
        screen.x,
        screen
            .y
            .saturating_add(screen.height.saturating_sub(height)),
        screen.width,
        height,
    )
}

pub(super) fn draw(
    frame: &mut Frame<'_>,
    panel: &mut WorkspacePanel,
    area: Rect,
    hovered: Option<WorkspacePanelHitTarget>,
    settings: &Settings,
    loaded_workspace_path: Option<&Path>,
) -> Vec<(HitTarget, Rect)> {
    let mut targets = Vec::new();
    if area.width < 4 || area.height == 0 {
        return targets;
    }

    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    let (header_height, footer_height) = chrome_heights(area);
    fill(
        frame,
        Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(1),
            area.width.saturating_sub(2),
            header_height.min(area.height.saturating_sub(2)),
        ),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(
            area.x.saturating_add(1),
            area.bottom().saturating_sub(footer_height + 1),
            area.width.saturating_sub(2),
            footer_height.min(area.height.saturating_sub(header_height + 2)),
        ),
        palette().surface_alt,
    );

    draw_header(frame, panel, area, &mut targets, hovered);

    let (workspace_section, agent_section) = section_areas(area);
    draw_workspace_section(
        frame,
        panel,
        workspace_section,
        hovered,
        loaded_workspace_path,
        &mut targets,
    );
    draw_agent_section(frame, panel, agent_section, settings, hovered, &mut targets);

    draw_footer(frame, panel, area);

    if panel.create_menu_open
        && let Some(anchor) = targets.iter().find_map(|(target, rect)| {
            (*target == HitTarget::WorkspacePanel(WorkspacePanelHitTarget::CreateMenu))
                .then_some(*rect)
        })
    {
        let worktree_enabled = panel.selected_workspace_id().is_some();
        let (workspace, worktree) = draw_create_popover(
            frame,
            area,
            anchor,
            panel.create_menu_choice,
            worktree_enabled,
            hovered,
        );
        targets.push((
            HitTarget::WorkspacePanel(WorkspacePanelHitTarget::CreateWorkspace),
            workspace,
        ));
        targets.push((
            HitTarget::WorkspacePanel(WorkspacePanelHitTarget::CreateWorktree),
            worktree,
        ));
    }
    if panel.snapshot_menu_open
        && let Some(anchor) = targets.iter().find_map(|(target, rect)| {
            (*target == HitTarget::WorkspacePanel(WorkspacePanelHitTarget::SnapshotMenu))
                .then_some(*rect)
        })
    {
        for (index, popup_area) in draw_snapshot_popover(frame, panel, area, anchor, hovered) {
            let target = if index == 0 {
                WorkspacePanelHitTarget::SaveSnapshot
            } else {
                WorkspacePanelHitTarget::Snapshot(index - 1)
            };
            targets.push((HitTarget::WorkspacePanel(target), popup_area));
        }
    }
    targets
}

pub(super) fn section_areas(area: Rect) -> (Rect, Rect) {
    let (header_height, footer_height) = chrome_heights(area);
    let body = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(header_height + 1),
        area.width.saturating_sub(4),
        area.height
            .saturating_sub(header_height + footer_height + 2),
    );
    if area.width >= 82 {
        let columns = Layout::horizontal([
            Constraint::Percentage(56),
            Constraint::Length(2),
            Constraint::Min(22),
        ])
        .split(body);
        (columns[0], columns[2])
    } else {
        let workspace_height = body.height.saturating_add(1) / 2;
        (
            Rect::new(body.x, body.y, body.width, workspace_height),
            Rect::new(
                body.x,
                body.y.saturating_add(workspace_height),
                body.width,
                body.height.saturating_sub(workspace_height),
            ),
        )
    }
}

fn draw_header(
    frame: &mut Frame<'_>,
    panel: &WorkspacePanel,
    area: Rect,
    targets: &mut Vec<(HitTarget, Rect)>,
    hovered: Option<WorkspacePanelHitTarget>,
) {
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        chrome_heights(area).0.min(area.height.saturating_sub(2)),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let title_row = Rect::new(inner.x, inner.y, inner.width, 1);
    let close_button = Rect::new(
        title_row.right().saturating_sub(8),
        title_row.y,
        8.min(title_row.width),
        1,
    );
    let title = Line::from(vec![
        Span::styled(
            "WORKSPACE MANAGER",
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  HERDR / CONTROL CENTER",
            Style::default().fg(palette().faint),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title),
        Rect::new(
            title_row.x,
            title_row.y,
            title_row.width.saturating_sub(close_button.width + 1),
            1,
        ),
    );

    let workspace_count = panel.workspaces.len();
    let agent_count = panel.agents.len();
    let active = panel
        .workspaces
        .iter()
        .filter(|workspace| workspace.focused)
        .count();
    let working = panel
        .agents
        .iter()
        .filter(|agent| agent.status == AgentStatus::Working)
        .count();
    draw_button(
        frame,
        close_button,
        "× Close",
        hovered == Some(WorkspacePanelHitTarget::Collapse),
    );

    let metrics = if inner.width < 70 {
        vec![
            (format!("{workspace_count} WS"), palette().orange),
            (format!("{agent_count} AG"), palette().cyan),
            (format!("{working} WORKING"), palette().yellow),
        ]
    } else {
        vec![
            (format!("{workspace_count} WORKSPACES"), palette().orange),
            (format!("{agent_count} AGENTS"), palette().cyan),
            (format!("{active} ACTIVE"), palette().yellow),
            (format!("{working} WORKING"), palette().green),
        ]
    };
    let compact = inner.height < HEADER_HEIGHT;
    if !compact {
        let metric_row = Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 1);
        let mut metric_x = metric_row.x;
        for (label, color) in metrics {
            let width = badge_width(&label);
            if metric_x.saturating_add(width) > metric_row.right() {
                break;
            }
            draw_badge(
                frame,
                Rect::new(metric_x, metric_row.y, width, 1),
                &label,
                color,
                palette().raised,
            );
            metric_x = metric_x.saturating_add(width + 1);
        }
    }

    let row = Rect::new(
        inner.x,
        inner.y.saturating_add(if compact { 1 } else { 3 }),
        inner.width,
        1,
    );
    let new_button = button_area(row, 0, 7);
    let presets_button = button_area(row, 8, 10);
    draw_button(
        frame,
        new_button,
        "+ New",
        hovered == Some(WorkspacePanelHitTarget::CreateMenu) || panel.create_menu_open,
    );
    draw_button(
        frame,
        presets_button,
        "Presets",
        hovered == Some(WorkspacePanelHitTarget::SnapshotMenu) || panel.snapshot_menu_open,
    );
    frame.render_widget(
        Paragraph::new("r Refresh").style(Style::default().fg(palette().muted)),
        Rect::new(
            row.x.saturating_add(20),
            row.y,
            row.width.saturating_sub(20),
            1,
        ),
    );
    targets.push((
        HitTarget::WorkspacePanel(WorkspacePanelHitTarget::CreateMenu),
        new_button,
    ));
    targets.push((
        HitTarget::WorkspacePanel(WorkspacePanelHitTarget::SnapshotMenu),
        presets_button,
    ));
    targets.push((
        HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Collapse),
        close_button,
    ));
}

fn draw_workspace_section(
    frame: &mut Frame<'_>,
    panel: &mut WorkspacePanel,
    section: Rect,
    hovered: Option<WorkspacePanelHitTarget>,
    loaded_workspace_path: Option<&Path>,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if section.width == 0 || section.height == 0 {
        return;
    }
    let list = draw_section_frame(
        frame,
        section,
        "WORKSPACES",
        panel.workspaces.len(),
        palette().orange,
    );
    let rows = panel
        .workspace_rows()
        .into_iter()
        .filter(|row| {
            matches!(
                row,
                WorkspacePanelRow::Group(_) | WorkspacePanelRow::Workspace(_)
            )
        })
        .collect::<Vec<_>>();
    let selected_row = panel.selected.and_then(|selected| {
        rows.iter().position(
            |row| matches!(row, WorkspacePanelRow::Workspace(index) if *index == selected),
        )
    });
    let groups = (0..panel.workspaces.len())
        .map(|index| panel.group_for_workspace(index))
        .collect::<Vec<_>>();
    let mut group_counts = vec![0usize; panel.groups.len()];
    for group in groups.iter().flatten() {
        group_counts[*group] += 1;
    }
    let card_height = if list.height >= 8 { 2 } else { 1 };
    let card_gap = u16::from(card_height > 1);
    let item_step = card_height + card_gap;
    let viewport = usize::from((list.height + card_gap) / item_step).max(1);
    keep_section_visible(
        &mut panel.workspace_scroll,
        panel
            .workspace_scroll_follows_selection
            .then_some(selected_row)
            .flatten(),
        rows.len(),
        viewport,
    );
    for (visual_row, row) in rows
        .iter()
        .copied()
        .enumerate()
        .skip(panel.workspace_scroll)
    {
        let screen_row = visual_row.saturating_sub(panel.workspace_scroll);
        if screen_row >= viewport {
            break;
        }
        let row_area = Rect::new(
            list.x,
            list.y + u16::try_from(screen_row).unwrap_or(0) * item_step,
            list.width,
            card_height.min(
                list.height
                    .saturating_sub(u16::try_from(screen_row).unwrap_or(0) * item_step),
            ),
        );
        match row {
            WorkspacePanelRow::Group(index) => {
                let group = &panel.groups[index];
                let target = WorkspacePanelHitTarget::Group(index);
                draw_group(
                    frame,
                    row_area,
                    if group.expanded { "▾" } else { "▸" },
                    &group.name,
                    group_counts[index],
                    hovered == Some(target)
                        || panel.workspace_drag_target() == Some(WorkspaceDropTarget::Group(index)),
                );
                targets.push((HitTarget::WorkspacePanel(target), row_area));
            }
            WorkspacePanelRow::Workspace(index) => {
                let state = panel.workspace_entry_state(index, true, loaded_workspace_path);
                let workspace = &panel.workspaces[index];
                let indent = match (
                    groups[index].is_some(),
                    panel.workspace_is_linked_worktree(index),
                ) {
                    (true, true) => "  └ ",
                    (true, false) => "  ",
                    (false, true) => "└ ",
                    (false, false) => "",
                };
                let target = WorkspacePanelHitTarget::Workspace(index);
                let metadata = format_workspace_metadata(
                    workspace.branch.as_deref(),
                    workspace.pane_count,
                    panel.workspace_is_linked_worktree(index),
                );
                draw_workspace_card(
                    frame,
                    row_area,
                    &format!("{indent}{}", workspace.label),
                    &metadata,
                    workspace.status,
                    state,
                    hovered == Some(target)
                        || (panel.workspace_drag_target() == Some(WorkspaceDropTarget::Ungrouped)
                            && groups[index].is_none()),
                );
                targets.push((HitTarget::WorkspacePanel(target), row_area));
            }
            _ => {}
        }
    }
    if panel.workspaces.is_empty() && list.height > 0 {
        let message = panel.error.as_deref().unwrap_or(if panel.loading {
            "Loading Herdr workspaces…"
        } else {
            "No workspaces detected"
        });
        frame.render_widget(
            Paragraph::new(format!(
                "  {}",
                truncate_width(message, usize::from(list.width).saturating_sub(2))
            ))
            .style(
                Style::default()
                    .fg(if panel.error.is_some() {
                        palette().red
                    } else {
                        palette().faint
                    })
                    .bg(palette().surface_alt),
            ),
            list,
        );
    }
}

fn draw_agent_section(
    frame: &mut Frame<'_>,
    panel: &mut WorkspacePanel,
    section: Rect,
    settings: &Settings,
    hovered: Option<WorkspacePanelHitTarget>,
    targets: &mut Vec<(HitTarget, Rect)>,
) {
    if section.width == 0 || section.height == 0 {
        return;
    }
    let list = draw_section_frame(
        frame,
        section,
        "AGENT ACTIVITY",
        panel.agents.len(),
        palette().cyan,
    );
    let rows = panel
        .agent_rows()
        .into_iter()
        .filter(|row| {
            matches!(
                row,
                WorkspacePanelRow::Agent(_) | WorkspacePanelRow::EmptyAgents
            )
        })
        .collect::<Vec<_>>();
    let selected_row = panel
        .selected
        .and_then(|selected| selected.checked_sub(panel.workspaces.len()))
        .and_then(|selected| {
            rows.iter().position(
                |row| matches!(row, WorkspacePanelRow::Agent(index) if *index == selected),
            )
        });
    let card_height = if list.height >= 8 { 2 } else { 1 };
    let card_gap = 1;
    let card_padding = u16::from(list.height >= card_height + 2);
    let card_list = Rect::new(
        list.x,
        list.y.saturating_add(card_padding),
        list.width,
        list.height.saturating_sub(card_padding.saturating_mul(2)),
    );
    let item_step = card_height + card_gap;
    let viewport = usize::from((card_list.height + card_gap) / item_step).max(1);
    keep_section_visible(
        &mut panel.agent_scroll,
        panel
            .agent_scroll_follows_selection
            .then_some(selected_row)
            .flatten(),
        rows.len(),
        viewport,
    );
    let mut last_card = None;
    for (visual_row, row) in rows.iter().copied().enumerate().skip(panel.agent_scroll) {
        let screen_row = visual_row.saturating_sub(panel.agent_scroll);
        if screen_row >= viewport {
            break;
        }
        let row_area = Rect::new(
            card_list.x,
            card_list.y + u16::try_from(screen_row).unwrap_or(0) * item_step,
            card_list.width,
            card_height.min(
                card_list
                    .height
                    .saturating_sub(u16::try_from(screen_row).unwrap_or(0) * item_step),
            ),
        );
        match row {
            WorkspacePanelRow::Agent(index) => {
                let state = panel.agent_entry_state(index, true);
                let agent = &panel.agents[index];
                let workspace = panel
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == agent.workspace_id)
                    .map_or("unassigned", |workspace| workspace.label.as_str());
                let elapsed = panel
                    .agent_elapsed(index, settings.agent_time_display)
                    .map(format_duration);
                let session = panel
                    .agent_display_name(index)
                    .unwrap_or("terminal session");
                let target = WorkspacePanelHitTarget::Agent(index);
                let current_background = agent_card_background(&state, hovered == Some(target));
                let previous_background = visual_row.checked_sub(1).and_then(|previous| match rows
                    .get(previous)
                    .copied()
                {
                    Some(WorkspacePanelRow::Agent(index)) => {
                        let previous_state = panel.agent_entry_state(index, true);
                        Some(agent_card_background(
                            &previous_state,
                            hovered == Some(WorkspacePanelHitTarget::Agent(index)),
                        ))
                    }
                    _ => None,
                });
                if row_area.y > list.y {
                    draw_agent_gap(
                        frame,
                        Rect::new(row_area.x, row_area.y - 1, row_area.width, 1),
                        previous_background.unwrap_or(palette().panel),
                        current_background,
                    );
                }
                draw_agent_card(
                    frame,
                    row_area,
                    workspace,
                    session,
                    elapsed.as_deref(),
                    agent.status,
                    state,
                    hovered == Some(target),
                );
                last_card = Some((row_area, current_background));
                targets.push((HitTarget::WorkspacePanel(target), row_area));
            }
            WorkspacePanelRow::EmptyAgents => {
                let message = panel.error.as_deref().unwrap_or(if panel.loading {
                    "Loading agent activity…"
                } else {
                    "No agents detected"
                });
                frame.render_widget(
                    Paragraph::new(format!(
                        "  {}",
                        truncate_width(message, usize::from(list.width).saturating_sub(2))
                    ))
                    .style(
                        Style::default()
                            .fg(if panel.error.is_some() {
                                palette().red
                            } else {
                                palette().faint
                            })
                            .bg(palette().surface_alt),
                    ),
                    list,
                );
            }
            _ => {}
        }
    }
    if let Some((card, background)) = last_card {
        let gap = Rect::new(card.x, card.bottom(), card.width, 1);
        if gap.bottom() <= list.bottom() {
            draw_agent_gap(frame, gap, background, palette().panel);
        }
    }
    if panel.agents.is_empty() && list.height > 0 && !panel.loading && panel.error.is_none() {
        frame.render_widget(
            Paragraph::new("  No agents detected").style(
                Style::default()
                    .fg(palette().faint)
                    .bg(palette().surface_alt),
            ),
            list,
        );
    }
}

fn draw_agent_gap(frame: &mut Frame<'_>, gap: Rect, above: Color, below: Color) {
    if gap.width == 0 || gap.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("▀".repeat(usize::from(gap.width)))
            .style(Style::default().fg(above).bg(below)),
        gap,
    );
}

fn agent_card_background(state: &WorkspacePanelEntryState, hovered: bool) -> Color {
    let highlighted = state.selected || hovered;
    if highlighted {
        palette().inactive_selected
    } else {
        palette().surface_alt
    }
}

fn draw_footer(frame: &mut Frame<'_>, panel: &WorkspacePanel, area: Rect) {
    let (header_height, footer_height) = chrome_heights(area);
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.bottom().saturating_sub(footer_height + 1),
        area.width.saturating_sub(4),
        footer_height.min(area.height.saturating_sub(header_height + 2)),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let message = if panel.group_editing {
        format!("Group: {}", panel.group_input.text())
    } else if panel.snapshot_editing {
        format!("Preset: {}", panel.snapshot_input.text())
    } else if panel.snapshot_menu_open {
        panel
            .snapshot_error
            .as_deref()
            .unwrap_or("Enter load  Del remove")
            .to_owned()
    } else if panel.create_menu_open {
        "↑/↓ choose  Enter create  Esc cancel".to_owned()
    } else {
        "Enter focus Herdr  Click open in Hunkle  g group  F2 rename  Del remove".to_owned()
    };
    frame.render_widget(
        Paragraph::new(message).style(Style::default().fg(
            if panel.group_editing || panel.snapshot_editing {
                palette().accent
            } else {
                palette().muted
            },
        )),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new("j/k navigate  r refresh  p presets  w / Esc close")
            .alignment(ratatui::layout::Alignment::Right)
            .style(Style::default().fg(palette().faint)),
        Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 1),
    );
    if inner.height > 2 {
        frame.render_widget(
            Paragraph::new(if panel.error.is_some() {
                "Inventory unavailable; r retries the Herdr snapshot"
            } else {
                "Click a card to open it in Hunkle"
            })
            .style(Style::default().fg(if panel.error.is_some() {
                palette().red
            } else {
                palette().faint
            })),
            Rect::new(inner.x, inner.y.saturating_add(2), inner.width, 1),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_agents_pane(
    frame: &mut Frame<'_>,
    panel: &mut WorkspacePanel,
    settings: &Settings,
    header: Rect,
    list: Rect,
    dragging: bool,
    enabled: bool,
    hovered: Option<usize>,
) -> Vec<(HitTarget, Rect)> {
    let mut targets = Vec::new();
    if header.width == 0 || header.height == 0 {
        return targets;
    }
    fill(
        frame,
        header,
        if dragging {
            palette().selected
        } else {
            palette().surface_alt
        },
    );
    let title = format!("AGENTS  {}", panel.agents.len());
    let hint = if enabled {
        "click  focus"
    } else {
        "w  workspace manager"
    };
    let padding = usize::from(header.width)
        .saturating_sub(UnicodeWidthStr::width(title.as_str()) + UnicodeWidthStr::width(hint) + 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                truncate_width(
                    &title,
                    usize::from(header.width)
                        .saturating_sub(UnicodeWidthStr::width(hint) + 1),
                ),
                Style::default()
                    .fg(palette().cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(padding)),
            Span::styled(hint, Style::default().fg(palette().faint)),
        ])),
        header,
    );
    if !enabled {
        if list.height > 0 {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    "  Workspace manager is disabled in settings",
                    Style::default().fg(palette().faint),
                )),
                list,
            );
        }
        return targets;
    }
    if panel.agents.is_empty() {
        let message = panel.error.as_deref().unwrap_or(if panel.loading {
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
                        .fg(if panel.error.is_some() {
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
    let viewport = usize::from((list.height + 1) / card_height).max(1);
    let scroll = panel
        .agent_scroll
        .min(panel.agents.len().saturating_sub(viewport));
    for (screen_row, index) in (scroll..panel.agents.len()).enumerate() {
        if screen_row >= viewport {
            break;
        }
        let offset = u16::try_from(screen_row).unwrap_or(0) * card_height;
        let row_area = Rect::new(
            list.x,
            list.y.saturating_add(offset),
            list.width,
            card_height.min(list.height.saturating_sub(offset)),
        );
        let agent = &panel.agents[index];
        let state = panel.agent_entry_state(index, false);
        let in_active_workspace = panel.agent_is_in_active_workspace(index);
        let workspace = panel
            .workspaces
            .iter()
            .find(|workspace| workspace.id == agent.workspace_id)
            .map_or("unassigned", |workspace| workspace.label.as_str());
        let session = panel
            .agent_display_name(index)
            .unwrap_or("terminal session");
        let elapsed = panel
            .agent_elapsed(index, settings.agent_time_display)
            .map(format_duration);
        draw_agents_pane_row(
            frame,
            row_area,
            workspace,
            session,
            elapsed.as_deref(),
            agent.status,
            state,
            in_active_workspace,
            hovered == Some(index),
        );
        targets.push((
            HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(index)),
            row_area,
        ));
    }
    targets
}

#[allow(clippy::too_many_arguments)]
fn draw_agents_pane_row(
    frame: &mut Frame<'_>,
    area: Rect,
    workspace: &str,
    session: &str,
    elapsed: Option<&str>,
    status: AgentStatus,
    state: WorkspacePanelEntryState,
    in_active_workspace: bool,
    hovered: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let highlighted = state.selected || hovered;
    let background = if highlighted {
        palette().selected
    } else {
        palette().surface_alt
    };
    fill(frame, area, background);
    let content = Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let status_text = status_label(status);
    let badge = badge_width(status_text);
    let badge_area = Rect::new(
        content.right().saturating_sub(badge),
        content.y,
        badge.min(content.width),
        1,
    );
    draw_badge(
        frame,
        badge_area,
        status_text,
        status_color(status),
        palette().raised,
    );
    let name_area = Rect::new(
        area.x,
        content.y,
        badge_area.x.saturating_sub(area.x.saturating_add(1)),
        1,
    );
    let name_color = if in_active_workspace {
        palette().yellow
    } else {
        palette().ink
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("●", Style::default().fg(status_color(status))),
            Span::raw(" "),
            Span::styled(
                truncate_width(workspace, usize::from(name_area.width).saturating_sub(2)),
                Style::default()
                    .fg(name_color)
                    .bg(background)
                    .add_modifier(if highlighted { Modifier::BOLD } else { Modifier::empty() }),
            ),
        ])),
        name_area,
    );
    if area.height > 1 {
        let time_width = elapsed
            .map(|time| u16::try_from(UnicodeWidthStr::width(time)).unwrap_or(u16::MAX))
            .unwrap_or(0)
            .min(content.width);
        let session_width = content
            .width
            .saturating_sub(time_width.saturating_add(u16::from(time_width > 0)));
        frame.render_widget(
            Paragraph::new(truncate_width(session, usize::from(session_width)))
                .style(Style::default().fg(palette().muted).bg(background)),
            Rect::new(content.x, content.y.saturating_add(1), session_width, 1),
        );
        if let Some(elapsed) = elapsed {
            frame.render_widget(
                Paragraph::new(elapsed)
                    .alignment(ratatui::layout::Alignment::Right)
                    .style(Style::default().fg(palette().soft).bg(background)),
                Rect::new(
                    content.right().saturating_sub(time_width),
                    content.y.saturating_add(1),
                    time_width,
                    1,
                ),
            );
        }
    }
}

fn draw_section_frame(
    frame: &mut Frame<'_>,
    section: Rect,
    label: &str,
    count: usize,
    accent: Color,
) -> Rect {
    fill(frame, section, palette().panel);
    let header = Rect::new(
        section.x.saturating_add(1),
        section.y.saturating_add(1),
        section.width.saturating_sub(2),
        1.min(section.height.saturating_sub(2)),
    );
    if header.width > 0 && header.height > 0 {
        let count_label = count.to_string();
        let count_area = Rect::new(
            header.right().saturating_sub(badge_width(&count_label)),
            header.y,
            badge_width(&count_label).min(header.width),
            1,
        );
        frame.render_widget(
            Paragraph::new(label).style(Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Rect::new(
                header.x,
                header.y,
                count_area.x.saturating_sub(header.x.saturating_add(1)),
                1,
            ),
        );
        draw_badge(frame, count_area, &count_label, accent, palette().raised);
    }
    Rect::new(
        section.x.saturating_add(1),
        section.y.saturating_add(2),
        section.width.saturating_sub(2),
        section.height.saturating_sub(3),
    )
}

fn draw_workspace_card(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    metadata: &str,
    status: AgentStatus,
    state: WorkspacePanelEntryState,
    hovered: bool,
) {
    let highlighted = state.selected || hovered;
    let background = if highlighted {
        palette().selected
    } else {
        palette().surface_alt
    };
    fill(frame, area, background);
    let rail = if highlighted {
        palette().accent
    } else if state.active {
        palette().yellow
    } else if state.loaded {
        palette().green
    } else {
        palette().raised
    };
    fill(frame, Rect::new(area.x, area.y, 1, area.height), rail);

    let state_label = if state.active && state.loaded {
        "ACTIVE · OPEN"
    } else if state.active {
        "ACTIVE"
    } else if state.loaded {
        "OPEN"
    } else {
        status_label(status)
    };
    let state_color = if state.active {
        palette().yellow
    } else if state.loaded {
        palette().green
    } else {
        status_color(status)
    };
    let badge = badge_width(state_label);
    let content = Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(3),
        area.height,
    );
    let badge_area = Rect::new(
        content.right().saturating_sub(badge),
        content.y,
        badge.min(content.width),
        1,
    );
    draw_badge(
        frame,
        badge_area,
        state_label,
        state_color,
        palette().raised,
    );
    let label_area = Rect::new(
        content.x,
        content.y,
        badge_area.x.saturating_sub(content.x.saturating_add(1)),
        1,
    );
    frame.render_widget(
        Paragraph::new(truncate_width(label, usize::from(label_area.width))).style(
            Style::default()
                .fg(if state.active {
                    palette().yellow
                } else {
                    palette().ink
                })
                .bg(background)
                .add_modifier(if highlighted {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        label_area,
    );
    if area.height > 1 {
        frame.render_widget(
            Paragraph::new(format!(
                "  {}",
                truncate_width(metadata, usize::from(content.width).saturating_sub(2))
            ))
            .style(Style::default().fg(palette().muted).bg(background)),
            Rect::new(content.x, content.y.saturating_add(1), content.width, 1),
        );
    }
}

fn format_workspace_metadata(
    branch: Option<&str>,
    pane_count: usize,
    linked_worktree: bool,
) -> String {
    let branch = branch.unwrap_or("detached");
    let panes = if pane_count == 1 { "pane" } else { "panes" };
    if linked_worktree {
        format!("{branch}  ·  linked worktree  ·  {pane_count} {panes}")
    } else {
        format!("{branch}  ·  {pane_count} {panes}")
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_agent_card(
    frame: &mut Frame<'_>,
    area: Rect,
    repository: &str,
    session: &str,
    elapsed: Option<&str>,
    status: AgentStatus,
    state: WorkspacePanelEntryState,
    hovered: bool,
) {
    let highlighted = state.selected || hovered;
    let background = agent_card_background(&state, hovered);
    fill(frame, area, background);
    let content = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let status_text = status_label(status);
    let badge = badge_width(status_text);
    let badge_area = Rect::new(
        content.right().saturating_sub(badge),
        content.y,
        badge.min(content.width),
        1,
    );
    draw_badge(
        frame,
        badge_area,
        status_text,
        status_color(status),
        palette().raised,
    );
    let name_area = Rect::new(
        content.x,
        content.y,
        badge_area.x.saturating_sub(content.x.saturating_add(1)),
        1,
    );
    frame.render_widget(
        Paragraph::new(truncate_width(repository, usize::from(name_area.width))).style(
            Style::default()
                .fg(if state.active {
                    palette().accent
                } else {
                    palette().ink
                })
                .bg(background)
                .add_modifier(if highlighted || state.active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        name_area,
    );
    if area.height > 1 {
        let time_width = elapsed
            .map(|time| u16::try_from(UnicodeWidthStr::width(time)).unwrap_or(u16::MAX))
            .unwrap_or(0)
            .min(content.width);
        let session_width = content
            .width
            .saturating_sub(time_width.saturating_add(u16::from(time_width > 0)));
        frame.render_widget(
            Paragraph::new(truncate_width(session, usize::from(session_width)))
                .style(Style::default().fg(palette().muted).bg(background)),
            Rect::new(content.x, content.y.saturating_add(1), session_width, 1),
        );
        if let Some(elapsed) = elapsed {
            frame.render_widget(
                Paragraph::new(elapsed)
                    .alignment(ratatui::layout::Alignment::Right)
                    .style(Style::default().fg(palette().soft).bg(background)),
                Rect::new(
                    content.right().saturating_sub(time_width),
                    content.y.saturating_add(1),
                    time_width,
                    1,
                ),
            );
        }
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
    background: Color,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(format!(
            " {} ",
            truncate_width(label, usize::from(area.width).saturating_sub(2))
        ))
        .style(
            Style::default()
                .fg(foreground)
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
}

fn button_area(row: Rect, offset: u16, width: u16) -> Rect {
    Rect::new(
        row.x.saturating_add(offset),
        row.y,
        width.min(row.width.saturating_sub(offset)),
        1,
    )
}

fn draw_button(frame: &mut Frame<'_>, area: Rect, label: &str, active: bool) {
    frame.render_widget(
        Paragraph::new(format!(
            " {} ",
            truncate_width(label, usize::from(area.width).saturating_sub(2))
        ))
        .style(
            Style::default()
                .fg(if active {
                    palette().canvas
                } else {
                    palette().accent
                })
                .bg(if active {
                    palette().accent
                } else {
                    palette().raised
                })
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
}

fn draw_snapshot_popover(
    frame: &mut Frame<'_>,
    panel: &WorkspacePanel,
    bounds: Rect,
    anchor: Rect,
    hovered: Option<WorkspacePanelHitTarget>,
) -> Vec<(usize, Rect)> {
    let item_count = panel.snapshots.len() + 1;
    let height = u16::try_from(item_count)
        .unwrap_or(u16::MAX)
        .min(bounds.height.saturating_sub(2));
    if height == 0 {
        return Vec::new();
    }
    let width = 28.min(bounds.width.saturating_sub(2));
    let x = anchor.x.clamp(
        bounds.x.saturating_add(1),
        bounds.right().saturating_sub(width).saturating_sub(1),
    );
    let y = anchor
        .bottom()
        .min(bounds.bottom().saturating_sub(height).saturating_sub(1));
    let overlay = Rect::new(x, y, width, height);
    frame.render_widget(Clear, overlay);
    fill(frame, overlay, palette().raised);
    let visible = usize::from(height);
    let start = panel
        .snapshot_menu_choice
        .saturating_add(1)
        .saturating_sub(visible)
        .min(item_count.saturating_sub(visible));
    let mut areas = Vec::with_capacity(visible);
    for index in start..start + visible {
        let row = Rect::new(x, y + u16::try_from(index - start).unwrap_or(0), width, 1);
        let target = if index == 0 {
            WorkspacePanelHitTarget::SaveSnapshot
        } else {
            WorkspacePanelHitTarget::Snapshot(index - 1)
        };
        let selected = panel.snapshot_menu_choice == index || hovered == Some(target);
        let label = if index == 0 {
            "Save current preset…".to_owned()
        } else {
            let snapshot = &panel.snapshots[index - 1];
            format!(
                "{}  {} workspaces",
                snapshot.name,
                snapshot.workspace_count()
            )
        };
        frame.render_widget(
            Paragraph::new(format!(
                "  {}",
                truncate_width(&label, usize::from(width).saturating_sub(2))
            ))
            .style(
                Style::default()
                    .fg(if selected {
                        palette().ink
                    } else {
                        palette().muted
                    })
                    .bg(if selected {
                        palette().selected
                    } else {
                        palette().raised
                    }),
            ),
            row,
        );
        areas.push((index, row));
    }
    areas
}

fn draw_create_popover(
    frame: &mut Frame<'_>,
    bounds: Rect,
    anchor: Rect,
    selection: usize,
    worktree_enabled: bool,
    hovered: Option<WorkspacePanelHitTarget>,
) -> (Rect, Rect) {
    let width = 24.min(bounds.width.saturating_sub(2));
    let x = anchor.x.clamp(
        bounds.x.saturating_add(1),
        bounds.right().saturating_sub(width).saturating_sub(1),
    );
    let y = anchor.bottom().min(bounds.bottom().saturating_sub(3));
    let workspace = Rect::new(x, y, width, 1);
    let worktree = Rect::new(x, y.saturating_add(1), width, 1);
    let overlay = Rect::new(x, y, width, 2);
    frame.render_widget(Clear, overlay);
    fill(frame, overlay, palette().raised);
    for (index, (label, row, enabled, target)) in [
        (
            "New workspace",
            workspace,
            true,
            WorkspacePanelHitTarget::CreateWorkspace,
        ),
        (
            "New linked worktree",
            worktree,
            worktree_enabled,
            WorkspacePanelHitTarget::CreateWorktree,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let active = enabled && (selection == index || hovered == Some(target));
        frame.render_widget(
            Paragraph::new(format!("  {label}")).style(
                Style::default()
                    .fg(if enabled {
                        palette().ink
                    } else {
                        palette().faint
                    })
                    .bg(if active {
                        palette().selected
                    } else {
                        palette().raised
                    }),
            ),
            row,
        );
    }
    (workspace, worktree)
}

fn draw_group(
    frame: &mut Frame<'_>,
    area: Rect,
    marker: &str,
    name: &str,
    count: usize,
    highlighted: bool,
) {
    let background = if highlighted {
        palette().selected
    } else {
        palette().surface_alt
    };
    fill(frame, area, background);
    fill(
        frame,
        Rect::new(area.x, area.y, 1, area.height),
        if highlighted {
            palette().accent
        } else {
            palette().orange
        },
    );
    frame.render_widget(
        Paragraph::new(format!(
            "  {marker} {}",
            truncate_width(name, usize::from(area.width).saturating_sub(8))
        ))
        .style(
            Style::default()
                .fg(if highlighted {
                    palette().ink
                } else {
                    palette().orange
                })
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height > 1 {
        frame.render_widget(
            Paragraph::new(format!(
                "    {count} workspace{}  ·  click to {}",
                if count == 1 { "" } else { "s" },
                if marker == "▾" {
                    "collapse"
                } else {
                    "expand"
                },
            ))
            .style(Style::default().fg(palette().muted).bg(background)),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
    }
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

#[cfg(test)]
fn status_marker(status: AgentStatus, spinner_frame: usize) -> &'static str {
    match status {
        AgentStatus::Idle => "●",
        AgentStatus::Working => SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()],
        AgentStatus::Blocked => "B",
        AgentStatus::Done => "U",
        AgentStatus::Unknown => "?",
    }
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "IDLE",
        AgentStatus::Working => "WORKING",
        AgentStatus::Blocked => "BLOCKED",
        AgentStatus::Done => "DONE",
        AgentStatus::Unknown => "UNKNOWN",
    }
}

fn status_color(status: AgentStatus) -> Color {
    match status {
        AgentStatus::Idle => palette().cyan,
        AgentStatus::Working => palette().yellow,
        AgentStatus::Blocked => palette().red,
        AgentStatus::Done => palette().green,
        AgentStatus::Unknown => palette().faint,
    }
}

fn keep_section_visible(
    scroll: &mut usize,
    selected: Option<usize>,
    row_count: usize,
    viewport: usize,
) {
    if viewport == 0 {
        return;
    }
    let Some(selected) = selected else {
        *scroll = (*scroll).min(row_count.saturating_sub(viewport));
        return;
    };
    if selected < *scroll {
        *scroll = selected;
    } else if selected >= (*scroll).saturating_add(viewport) {
        *scroll = selected.saturating_add(1).saturating_sub(viewport);
    }
    *scroll = (*scroll).min(row_count.saturating_sub(viewport));
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

    #[test]
    fn status_indicators_distinguish_attention_states() {
        assert_eq!(status_marker(AgentStatus::Idle, 0), "●");
        assert_eq!(status_color(AgentStatus::Idle), palette().cyan);
        assert_eq!(status_marker(AgentStatus::Working, 0), "⠋");
        assert_eq!(status_color(AgentStatus::Working), palette().yellow);
        assert_eq!(status_marker(AgentStatus::Blocked, 0), "B");
        assert_eq!(status_color(AgentStatus::Blocked), palette().red);
        assert_eq!(status_marker(AgentStatus::Done, 0), "U");
        assert_eq!(status_color(AgentStatus::Done), palette().green);
        assert_eq!(status_marker(AgentStatus::Unknown, 0), "?");
        assert_eq!(status_color(AgentStatus::Unknown), palette().faint);
    }
}
