use super::*;

pub(crate) fn draw_workspace_delete_dialog(frame: &mut Frame<'_>, dialog: &WorkspaceDeleteDialog) {
    let area = centered_min(frame.area(), 66, 0, 54, 12);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    let (title, prompt, detail, warning, action) = match &dialog.kind {
        WorkspaceDeleteKind::Workspace { pane_count } => {
            let noun = if *pane_count == 1 { "pane" } else { "panes" };
            (
                "CLOSE WORKSPACE",
                format!("Close workspace {}?", dialog.label),
                format!("This closes the workspace and all {pane_count} {noun} inside it."),
                "Processes running in those panes will stop.".to_owned(),
                "Close workspace",
            )
        }
        WorkspaceDeleteKind::Worktree { path, .. } => {
            let path = path.as_deref().map_or_else(
                || "its checkout directory".to_owned(),
                |path| path.display().to_string(),
            );
            (
                "DELETE WORKTREE",
                format!("Delete worktree {}?", dialog.label),
                format!("This removes the linked checkout at {path}."),
                "Uncommitted work will block safe deletion.".to_owned(),
                "Delete worktree",
            )
        }
    };
    frame.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(palette().red)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(&prompt, usize::from(inner.width)))
            .style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(&detail, usize::from(inner.width)))
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(warning).style(Style::default().fg(palette().red)),
        Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1),
    );
    let button = Rect::new(
        inner.right().saturating_sub(18),
        area.y.saturating_add(9),
        18,
        1,
    );
    frame.render_widget(
        Paragraph::new(action)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().red).bg(palette().selected)),
        button,
    );
    frame.render_widget(
        Paragraph::new("Enter confirm   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(crate) fn draw_workspace_rename_dialog(frame: &mut Frame<'_>, dialog: &WorkspaceRenameDialog) {
    let area = centered_min(frame.area(), 62, 0, 48, 12);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    let (title, subject) = match &dialog.target {
        WorkspaceRenameTarget::Workspace { .. } => ("RENAME WORKSPACE", "workspace"),
        WorkspaceRenameTarget::Agent { .. } => ("RENAME AGENT", "agent"),
    };
    frame.render_widget(
        Paragraph::new(title).style(
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(
            &format!("Rename {subject} {}", dialog.original_label),
            usize::from(inner.width),
        ))
        .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    let mut input = dialog.input.text().to_owned();
    if dialog.input.cursor_visible() {
        input.insert(dialog.input.cursor(), '▌');
    }
    frame.render_widget(
        Paragraph::new(truncate_start_width(&input, usize::from(inner.width)))
            .style(Style::default().fg(palette().ink).bg(palette().selected)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    if let Some(error) = &dialog.error {
        frame.render_widget(
            Paragraph::new(truncate_width(error, usize::from(inner.width)))
                .style(Style::default().fg(palette().red)),
            Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1),
        );
    }
    frame.render_widget(
        Paragraph::new("Enter rename   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(crate) fn draw_snapshot_load_dialog(frame: &mut Frame<'_>, dialog: &SnapshotLoadDialog) {
    let area = centered_min(frame.area(), 68, 0, 56, 13);
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new("LOAD WORKSPACE PRESET").style(
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(truncate_width(
            &format!("Load workspace preset {}?", dialog.name),
            usize::from(inner.width),
        ))
        .style(Style::default().fg(palette().ink)),
        Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
    );
    let workspace_noun = if dialog.close_count == 1 {
        "workspace"
    } else {
        "workspaces"
    };
    let pane_noun = if dialog.close_pane_count == 1 {
        "pane"
    } else {
        "panes"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Open {}  |  Close {} {} ({} {})  |  Restore {} groups",
            dialog.open_count,
            dialog.close_count,
            workspace_noun,
            dialog.close_pane_count,
            pane_noun,
            dialog.group_count,
        ))
        .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.y.saturating_add(6), inner.width, 1),
    );
    let warning = if dialog.close_count == 0 {
        "Existing workspaces are reused by directory."
    } else {
        "Processes in closed workspace panes will stop."
    };
    frame.render_widget(
        Paragraph::new(warning).style(Style::default().fg(if dialog.close_count == 0 {
            palette().accent
        } else {
            palette().red
        })),
        Rect::new(inner.x, area.y.saturating_add(8), inner.width, 1),
    );
    let button = Rect::new(
        inner.right().saturating_sub(18),
        area.y.saturating_add(10),
        18,
        1,
    );
    frame.render_widget(
        Paragraph::new("Load preset")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().accent).bg(palette().selected)),
        button,
    );
    frame.render_widget(
        Paragraph::new("Enter confirm   Esc cancel")
            .alignment(Alignment::Right)
            .style(Style::default().fg(palette().muted)),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
}

pub(crate) fn draw_workspace_presets(
    frame: &mut Frame<'_>,
    panel: &WorkspacePanel,
    shortcuts: &Shortcuts,
) -> (Rect, Vec<(HitTarget, Rect)>) {
    let item_count = panel.snapshots.len() + 1;
    let desired_height = if panel.snapshot_editing {
        10
    } else {
        u16::try_from(item_count).unwrap_or(u16::MAX).min(7) + 7
    };
    let area = centered_min(frame.area(), 0, 0, 50, desired_height);
    let mut targets = vec![(
        HitTarget::WorkspacePanel(WorkspacePanelHitTarget::PresetOverlay),
        area,
    )];
    frame.render_widget(Clear, area);
    fill(frame, area, palette().panel);
    fill(
        frame,
        Rect::new(area.x, area.y, area.width, 3),
        palette().surface_alt,
    );
    fill(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        palette().surface_alt,
    );
    let inner = area.inner(ratatui::layout::Margin::new(2, 1));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "WORKSPACE PRESETS",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  {} workspaces · {} groups",
                    panel.workspaces.len(),
                    panel.groups.len()
                ),
                Style::default().fg(palette().faint),
            ),
        ])),
        Rect::new(inner.x, area.y.saturating_add(1), inner.width, 1),
    );
    let section_y = area.y.saturating_add(3);

    if panel.snapshot_editing {
        frame.render_widget(
            Paragraph::new("PRESET NAME").style(
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(inner.x, section_y, inner.width, 1),
        );
        let mut input = panel.snapshot_input.text().to_owned();
        if panel.snapshot_input.cursor_visible() {
            input.insert(panel.snapshot_input.cursor(), '▌');
        }
        frame.render_widget(
            Paragraph::new(format!("  {input}"))
                .style(Style::default().fg(palette().ink).bg(palette().selected)),
            Rect::new(inner.x, section_y.saturating_add(2), inner.width, 1),
        );
        if section_y.saturating_add(4) < area.bottom().saturating_sub(1) {
            frame.render_widget(
                Paragraph::new(panel.snapshot_error.as_deref().unwrap_or(
                    "Using an existing name updates that preset with the current setup.",
                ))
                .style(Style::default().fg(if panel.snapshot_error.is_some() {
                    palette().red
                } else {
                    palette().faint
                })),
                Rect::new(inner.x, section_y.saturating_add(4), inner.width, 1),
            );
        }
        frame.render_widget(
            Paragraph::new("Enter save   Esc back")
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette().muted)),
            Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
        );
        return (area, targets);
    }

    frame.render_widget(
        Paragraph::new(format!("SAVED PRESETS  {}", panel.snapshots.len())).style(
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, section_y, inner.width, 1),
    );
    let list_y = section_y.saturating_add(2);
    let list = Rect::new(
        inner.x,
        list_y,
        inner.width,
        area.bottom().saturating_sub(2).saturating_sub(list_y),
    );
    let visible = usize::from(list.height).min(item_count);
    let start = panel
        .snapshot_menu_choice
        .saturating_add(1)
        .saturating_sub(visible)
        .min(item_count.saturating_sub(visible));
    for index in start..start + visible {
        let row = Rect::new(
            list.x,
            list.y + u16::try_from(index - start).unwrap_or(0),
            list.width,
            1,
        );
        let selected = panel.snapshot_menu_choice == index;
        let (label, detail, color, target) = if index == 0 {
            (
                "+  Create preset from current setup".to_owned(),
                String::new(),
                palette().accent,
                WorkspacePanelHitTarget::SaveSnapshot,
            )
        } else {
            let preset = &panel.snapshots[index - 1];
            (
                format!("   {}", preset.name),
                format!(
                    "{} workspaces  ·  {} groups",
                    preset.workspace_count(),
                    preset.group_count()
                ),
                palette().ink,
                WorkspacePanelHitTarget::Snapshot(index - 1),
            )
        };
        let detail_width = UnicodeWidthStr::width(detail.as_str());
        let label = truncate_width(
            &label,
            usize::from(row.width).saturating_sub(detail_width.saturating_add(2)),
        );
        let padding = usize::from(row.width)
            .saturating_sub(UnicodeWidthStr::width(label.as_str()) + detail_width);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, Style::default().fg(color)),
                Span::raw(" ".repeat(padding)),
                Span::styled(detail, Style::default().fg(palette().faint)),
            ]))
            .style(Style::default().bg(if selected {
                palette().selected
            } else {
                palette().panel
            })),
            row,
        );
        targets.push((HitTarget::WorkspacePanel(target), row));
    }
    let status = panel.snapshot_error.clone().unwrap_or_else(|| {
        format!(
            "Enter load  {} new  {} update  {} delete  Esc",
            shortcuts.label(ShortcutAction::PresetCreate),
            shortcuts.label(ShortcutAction::PresetUpdate),
            shortcuts.label(ShortcutAction::PresetDelete)
        )
    });
    frame.render_widget(
        Paragraph::new(status)
            .alignment(Alignment::Right)
            .style(Style::default().fg(if panel.snapshot_error.is_some() {
                palette().accent
            } else {
                palette().muted
            })),
        Rect::new(inner.x, area.bottom().saturating_sub(1), inner.width, 1),
    );
    (area, targets)
}
