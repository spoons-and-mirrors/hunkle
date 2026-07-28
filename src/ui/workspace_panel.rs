use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use std::{path::Path, time::Duration};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    AgentStatus, HitTarget, SPINNER_FRAMES, WorkspaceDropTarget, WorkspacePanel,
    WorkspacePanelHitTarget, WorkspacePanelPlacement, WorkspacePanelRow,
};

use super::{fill, palette, truncate_width};

pub(super) fn draw(
    frame: &mut Frame<'_>,
    panel: &mut WorkspacePanel,
    area: Rect,
    focused: bool,
    hovered: Option<WorkspacePanelHitTarget>,
    show_agent_harness: bool,
    loaded_workspace_path: Option<&Path>,
) -> Vec<(HitTarget, Rect)> {
    fill(frame, area, palette().surface_alt);
    let mut targets = Vec::new();
    if area.width < 4 || area.height == 0 {
        return targets;
    }

    let (workspace_section, agent_section) = section_areas(area);
    let body = Rect::new(
        workspace_section.x,
        workspace_section.y,
        workspace_section.width,
        workspace_section
            .height
            .saturating_add(agent_section.height),
    );
    let footer = Rect::new(
        area.x.saturating_add(1),
        area.bottom().saturating_sub(1),
        area.width.saturating_sub(2),
        1,
    );

    let spinner_frame = panel.spinner_frame();
    let mut create_button = None;
    let mut snapshot_button = None;

    if workspace_section.height > 0 {
        let row_area = Rect::new(
            workspace_section.x,
            workspace_section.y,
            workspace_section.width,
            1,
        );
        let (create, load) = draw_workspace_header(
            frame,
            row_area,
            panel.create_menu_open || hovered == Some(WorkspacePanelHitTarget::CreateMenu),
            panel.snapshot_menu_open || hovered == Some(WorkspacePanelHitTarget::SnapshotMenu),
        );
        create_button = Some(create);
        targets.push((
            HitTarget::WorkspacePanel(WorkspacePanelHitTarget::CreateMenu),
            create,
        ));
        if let Some(load) = load {
            snapshot_button = Some(load);
            targets.push((
                HitTarget::WorkspacePanel(WorkspacePanelHitTarget::SnapshotMenu),
                load,
            ));
        }
        let collapse = Rect::new(row_area.right().saturating_sub(1), row_area.y, 1, 1);
        let collapse_marker = match panel.placement {
            WorkspacePanelPlacement::Right => "›",
            WorkspacePanelPlacement::Off | WorkspacePanelPlacement::Left => "‹",
        };
        frame.render_widget(
            Paragraph::new(collapse_marker).style(Style::default().fg(palette().faint)),
            collapse,
        );
        targets.push((
            HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Collapse),
            collapse,
        ));
    }

    let workspace_list = Rect::new(
        workspace_section.x,
        workspace_section.y.saturating_add(1),
        workspace_section.width,
        workspace_section.height.saturating_sub(1),
    );
    let workspace_rows = panel.workspace_rows();
    let selected_workspace_row = panel.selected.and_then(|selected| {
        workspace_rows.iter().position(
            |row| matches!(row, WorkspacePanelRow::Workspace(index) if *index == selected),
        )
    });
    let workspace_groups = (0..panel.workspaces.len())
        .map(|index| panel.group_for_workspace(index))
        .collect::<Vec<_>>();
    let mut workspace_group_counts = vec![0usize; panel.groups.len()];
    for group in workspace_groups.iter().flatten() {
        workspace_group_counts[*group] += 1;
    }
    keep_section_visible(
        &mut panel.workspace_scroll,
        selected_workspace_row,
        workspace_rows.len(),
        usize::from(workspace_list.height),
    );
    for (visual_row, row) in workspace_rows
        .iter()
        .copied()
        .enumerate()
        .skip(panel.workspace_scroll)
    {
        let screen_row = visual_row.saturating_sub(panel.workspace_scroll);
        if screen_row >= usize::from(workspace_list.height) {
            break;
        }
        let row_area = Rect::new(
            workspace_list.x,
            workspace_list.y + screen_row as u16,
            workspace_list.width,
            1,
        );
        match row {
            WorkspacePanelRow::Group(index) => {
                let group = &panel.groups[index];
                let count = workspace_group_counts[index];
                let marker = if group.expanded { "▾" } else { "▸" };
                let drop_target =
                    panel.workspace_drag_target() == Some(WorkspaceDropTarget::Group(index));
                draw_group(frame, row_area, marker, &group.name, count, drop_target);
                targets.push((
                    HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Group(index)),
                    row_area,
                ));
            }
            WorkspacePanelRow::Workspace(index) => {
                let state = panel.workspace_entry_state(index, focused, loaded_workspace_path);
                let workspace = &panel.workspaces[index];
                let indent = match (
                    workspace_groups[index].is_some(),
                    panel.workspace_is_linked_worktree(index),
                ) {
                    (true, true) => "  ",
                    (true, false) | (false, true) => " ",
                    (false, false) => "",
                };
                let label = format!("{indent}{}", workspace.label);
                let ungrouped_drop = panel.workspace_drag_target()
                    == Some(WorkspaceDropTarget::Ungrouped)
                    && workspace_groups[index].is_none();
                draw_entry(
                    frame,
                    row_area,
                    &label,
                    workspace.branch.as_deref(),
                    EntryPresentation {
                        status: workspace.status,
                        marker_active: state.loaded,
                        label_active: state.active,
                        selected: state.selected || ungrouped_drop,
                        selected_background: palette().selected,
                        active_marker: "• ",
                        active_marker_color: palette().yellow,
                        active_label_color: palette().yellow,
                        spinner_frame,
                    },
                );
                targets.push((
                    HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Workspace(index)),
                    row_area,
                ));
            }
            _ => {}
        }
    }

    if panel.workspaces.is_empty() && workspace_list.height > 0 {
        let state = panel.error.as_deref().unwrap_or(if panel.loading {
            "Loading Herdr…"
        } else {
            "No workspaces"
        });
        frame.render_widget(
            Paragraph::new(truncate_width(state, usize::from(workspace_list.width))).style(
                Style::default().fg(if panel.error.is_some() {
                    palette().red
                } else {
                    palette().faint
                }),
            ),
            Rect::new(workspace_list.x, workspace_list.y, workspace_list.width, 1),
        );
    }

    if agent_section.height > 0 {
        draw_header(
            frame,
            Rect::new(agent_section.x, agent_section.y, agent_section.width, 1),
            "AGENTS",
            panel.agents.len(),
        );
    }
    let agent_list = Rect::new(
        agent_section.x,
        agent_section.y.saturating_add(1),
        agent_section.width,
        agent_section.height.saturating_sub(1),
    );
    let agent_rows = panel.agent_rows();
    let highlighted_agent_row = panel
        .highlighted_agent_index(focused)
        .and_then(|highlighted| {
            agent_rows.iter().position(|row| {
            matches!(row, WorkspacePanelRow::AgentSession(index) if *index == highlighted)
        })
        });
    keep_section_visible(
        &mut panel.agent_scroll,
        highlighted_agent_row,
        agent_rows.len(),
        usize::from(agent_list.height),
    );
    for (visual_row, row) in agent_rows
        .iter()
        .copied()
        .enumerate()
        .skip(panel.agent_scroll)
    {
        let screen_row = visual_row.saturating_sub(panel.agent_scroll);
        if screen_row >= usize::from(agent_list.height) {
            break;
        }
        let row_area = Rect::new(
            agent_list.x,
            agent_list.y + screen_row as u16,
            agent_list.width,
            1,
        );
        match row {
            WorkspacePanelRow::EmptyAgents => {
                frame.render_widget(
                    Paragraph::new("No agents detected")
                        .style(Style::default().fg(palette().faint)),
                    row_area,
                );
            }
            WorkspacePanelRow::Agent(index) => {
                let state = panel.agent_entry_state(index, focused);
                let elapsed = panel.agent_elapsed(index).map(format_duration);
                let agent = &panel.agents[index];
                let workspace = panel
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == agent.workspace_id)
                    .map_or("", |workspace| workspace.label.as_str());
                let label = if workspace.is_empty() {
                    agent.name.clone()
                } else if show_agent_harness {
                    format!("{} / {workspace}", agent.name)
                } else {
                    workspace.to_owned()
                };
                let label = elapsed.map_or(label.clone(), |elapsed| format!("{elapsed} {label}"));
                draw_entry(
                    frame,
                    row_area,
                    &label,
                    None,
                    EntryPresentation {
                        status: agent.status,
                        marker_active: state.active,
                        label_active: state.active,
                        selected: state.selected,
                        selected_background: palette().inactive_selected,
                        active_marker: "› ",
                        active_marker_color: palette().accent,
                        active_label_color: palette().ink,
                        spinner_frame,
                    },
                );
                let target_height = agent_list.bottom().saturating_sub(row_area.y).min(2);
                targets.push((
                    HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(index)),
                    Rect::new(row_area.x, row_area.y, row_area.width, target_height),
                ));
            }
            WorkspacePanelRow::AgentSession(index) => {
                let state = panel.agent_entry_state(index, focused);
                let base = state
                    .selected
                    .then_some(palette().inactive_selected)
                    .map_or_else(Style::default, |color| Style::default().bg(color));
                frame.render_widget(Block::default().style(base), row_area);
                if let Some(session_name) = panel.agents[index].session_name.as_deref() {
                    let session_area = Rect::new(
                        row_area.x.saturating_add(2),
                        row_area.y,
                        row_area.width.saturating_sub(2),
                        1,
                    );
                    frame.render_widget(
                        Paragraph::new(truncate_width(
                            session_name,
                            usize::from(session_area.width),
                        ))
                        .style(base.fg(if state.selected {
                            palette().yellow
                        } else {
                            palette().faint
                        })),
                        session_area,
                    );
                }
                let target = HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Agent(index));
                if !targets.iter().any(|(existing, _)| *existing == target) {
                    targets.push((target, row_area));
                }
            }
            _ => {}
        }
    }

    frame.render_widget(
        Paragraph::new(if panel.snapshot_editing {
            if let Some(error) = panel.snapshot_error.as_deref() {
                error
            } else {
                panel.snapshot_input.text()
            }
        } else if panel.group_editing {
            if let Some(error) = panel.group_error.as_deref() {
                error
            } else {
                panel.group_input.text()
            }
        } else if panel.snapshot_menu_open {
            panel
                .snapshot_error
                .as_deref()
                .unwrap_or("Enter load  Del remove")
        } else if focused {
            "Enter open  g group  Del"
        } else {
            "w move/off"
        })
        .style(Style::default().fg(if focused {
            palette().accent
        } else {
            palette().faint
        })),
        footer,
    );
    if panel.group_editing && panel.group_error.is_none() {
        frame.render_widget(
            Paragraph::new(format!("Group: {}", panel.group_input.text()))
                .style(Style::default().fg(palette().accent)),
            footer,
        );
    }
    if panel.snapshot_editing && panel.snapshot_error.is_none() {
        frame.render_widget(
            Paragraph::new(format!("Preset: {}", panel.snapshot_input.text()))
                .style(Style::default().fg(palette().accent)),
            footer,
        );
    }
    if panel.create_menu_open
        && let Some(anchor) = create_button
    {
        let worktree_enabled = panel.selected_workspace_id().is_some();
        let (workspace, worktree) = draw_create_popover(
            frame,
            body,
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
        && let Some(anchor) = snapshot_button
    {
        for (index, area) in draw_snapshot_popover(frame, panel, body, anchor, hovered) {
            let target = if index == 0 {
                WorkspacePanelHitTarget::SaveSnapshot
            } else {
                WorkspacePanelHitTarget::Snapshot(index - 1)
            };
            targets.push((HitTarget::WorkspacePanel(target), area));
        }
    }
    targets
}

pub(super) fn section_areas(area: Rect) -> (Rect, Rect) {
    let body = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
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
        .min(bounds.height.saturating_sub(1));
    if height == 0 {
        return Vec::new();
    }
    let width = 23.min(bounds.width);
    let x = anchor
        .right()
        .saturating_sub(width)
        .clamp(bounds.x, bounds.right().saturating_sub(width));
    let y = anchor.bottom();
    let overlay = Rect::new(x, y, width, height);
    frame.render_widget(ratatui::widgets::Clear, overlay);
    fill(frame, overlay, palette().raised);

    let visible = usize::from(height);
    let start = panel
        .snapshot_menu_choice
        .saturating_add(1)
        .saturating_sub(visible)
        .min(item_count.saturating_sub(visible));
    let mut areas = Vec::with_capacity(visible);
    for index in start..start + visible {
        let area = Rect::new(x, y + u16::try_from(index - start).unwrap_or(0), width, 1);
        let target = if index == 0 {
            WorkspacePanelHitTarget::SaveSnapshot
        } else {
            WorkspacePanelHitTarget::Snapshot(index - 1)
        };
        let hovered = hovered == Some(target);
        let selected = panel.snapshot_menu_choice == index;
        let label = if index == 0 {
            "Save current...".to_owned()
        } else {
            let snapshot = &panel.snapshots[index - 1];
            format!("{}  {}", snapshot.name, snapshot.workspace_count())
        };
        frame.render_widget(
            Paragraph::new(format!(
                "  {}",
                truncate_width(&label, usize::from(width).saturating_sub(2))
            ))
            .style(
                Style::default()
                    .fg(palette().ink)
                    .bg(if selected || hovered {
                        palette().selected
                    } else {
                        palette().raised
                    }),
            ),
            area,
        );
        areas.push((index, area));
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
    let width = 18.min(bounds.width);
    let x = anchor
        .right()
        .saturating_sub(width)
        .clamp(bounds.x, bounds.right().saturating_sub(width));
    let y = anchor.bottom();
    let workspace = Rect::new(x, y, width, 1);
    let worktree = Rect::new(x, y.saturating_add(1), width, 1);
    let overlay = Rect::new(x, y, width, 2);
    frame.render_widget(ratatui::widgets::Clear, overlay);
    fill(frame, overlay, palette().raised);
    for (index, (label, area, enabled, target)) in [
        (
            "New workspace",
            workspace,
            true,
            WorkspacePanelHitTarget::CreateWorkspace,
        ),
        (
            "New worktree",
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
            area,
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
    drop_target: bool,
) {
    let style = if drop_target {
        Style::default().bg(palette().selected).fg(palette().ink)
    } else {
        Style::default().fg(palette().muted)
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{marker} {}  {count}",
            truncate_width(name, usize::from(area.width).saturating_sub(6))
        ))
        .style(style.add_modifier(Modifier::BOLD)),
        area,
    );
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, label: &str, count: usize) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                label,
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {count}"), Style::default().fg(palette().faint)),
        ])),
        area,
    );
}

fn draw_workspace_header(
    frame: &mut Frame<'_>,
    area: Rect,
    create_hovered: bool,
    snapshot_hovered: bool,
) -> (Rect, Option<Rect>) {
    let compact = area.width < 22;
    let title = if compact { "WS" } else { "WORKSPACES" };
    let title_width = if compact { 2 } else { 10 };
    frame.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x, area.y, title_width.min(area.width), 1),
    );
    let button_x = area.x.saturating_add(title_width + 1);
    let button = Rect::new(
        button_x,
        area.y,
        3.min(area.right().saturating_sub(button_x)),
        1,
    );
    frame.render_widget(
        Paragraph::new(" + ").style(
            Style::default()
                .fg(if create_hovered {
                    palette().canvas
                } else {
                    palette().accent
                })
                .bg(if create_hovered {
                    palette().accent
                } else {
                    palette().raised
                })
                .add_modifier(Modifier::BOLD),
        ),
        button,
    );
    let load_x = button.right().saturating_add(1);
    let available = area.right().saturating_sub(1).saturating_sub(load_x);
    let load = (available >= 6).then(|| Rect::new(load_x, area.y, 6, 1));
    if let Some(load) = load {
        frame.render_widget(
            Paragraph::new(" Load ").style(
                Style::default()
                    .fg(if snapshot_hovered {
                        palette().canvas
                    } else {
                        palette().accent
                    })
                    .bg(if snapshot_hovered {
                        palette().accent
                    } else {
                        palette().raised
                    }),
            ),
            load,
        );
    }
    (button, load)
}

struct EntryPresentation {
    status: AgentStatus,
    marker_active: bool,
    label_active: bool,
    selected: bool,
    selected_background: Color,
    active_marker: &'static str,
    active_marker_color: Color,
    active_label_color: Color,
    spinner_frame: usize,
}

fn draw_entry(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    detail: Option<&str>,
    presentation: EntryPresentation,
) {
    let EntryPresentation {
        status,
        marker_active,
        label_active,
        selected,
        selected_background,
        active_marker,
        active_marker_color,
        active_label_color,
        spinner_frame,
    } = presentation;
    let marker = if marker_active { active_marker } else { "  " };
    let status_marker = status_marker(status, spinner_frame);
    let available = usize::from(area.width).saturating_sub(4);
    let detail = detail
        .filter(|detail| !detail.is_empty())
        .map(|detail| truncate_width(detail, available.saturating_sub(5).min(available / 2)));
    let detail_width = detail
        .as_deref()
        .map(UnicodeWidthStr::width)
        .unwrap_or_default();
    let label = truncate_width(
        label,
        available.saturating_sub(detail_width + usize::from(detail.is_some()) * 2),
    );
    let label_width = UnicodeWidthStr::width(label.as_str());
    let padding = available.saturating_sub(label_width + detail_width);
    let background = selected.then_some(selected_background);
    let base = background.map_or_else(Style::default, |color| Style::default().bg(color));
    frame.render_widget(Block::default().style(base), area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                marker,
                base.fg(if marker_active {
                    active_marker_color
                } else {
                    palette().faint
                }),
            ),
            Span::styled(
                label,
                if label_active {
                    base.fg(active_label_color).add_modifier(Modifier::BOLD)
                } else {
                    base.fg(palette().muted)
                },
            ),
            Span::styled(" ".repeat(padding), base),
            Span::styled(detail.unwrap_or_default(), base.fg(palette().accent)),
            Span::styled(" ", base),
            Span::styled(status_marker, base.fg(status_color(status))),
        ])),
        area,
    );
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let minutes = seconds / 60;
    if minutes < 60 {
        format!("{minutes}:{:02}", seconds % 60)
    } else {
        format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds % 60)
    }
}

fn status_marker(status: AgentStatus, spinner_frame: usize) -> &'static str {
    match status {
        AgentStatus::Idle => "●",
        AgentStatus::Working => SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()],
        AgentStatus::Blocked => "B",
        AgentStatus::Done => "U",
        AgentStatus::Unknown => "?",
    }
}

fn status_color(status: AgentStatus) -> ratatui::style::Color {
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
    fn formats_agent_durations_as_compact_clocks() {
        assert_eq!(format_duration(Duration::ZERO), "0:00");
        assert_eq!(format_duration(Duration::from_secs(62)), "1:02");
        assert_eq!(format_duration(Duration::from_secs(3_661)), "1:01:01");
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
