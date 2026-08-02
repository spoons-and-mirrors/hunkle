use super::*;

pub(crate) fn draw_help(frame: &mut Frame<'_>, shortcuts: &Shortcuts) {
    let area = centered_min(frame.area(), 72, 0, 58, 24);
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "KEYBOARD",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Quick reference", Style::default().fg(palette().faint)),
        ])),
        Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    let body = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(4),
        area.width.saturating_sub(4),
        area.height.saturating_sub(5),
    );
    let columns = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Length(2),
        Constraint::Percentage(50),
    ])
    .split(body);
    let navigation = vec![
        Line::styled(
            "NAVIGATION",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        shortcut_help(shortcuts, ShortcutAction::TogglePane, "Changes / files"),
        shortcut_help(
            shortcuts,
            ShortcutAction::ToggleGraph,
            "Show / hide Git graph",
        ),
        help_line("j / k", "Move / scroll hunk ×10"),
        help_line("Home / End", "First / last"),
        shortcut_help(shortcuts, ShortcutAction::Refresh, "Refresh"),
        shortcut_help(shortcuts, ShortcutAction::OpenExplorer, "Explorer"),
        shortcut_help(shortcuts, ShortcutAction::OpenSettings, "Settings"),
        shortcut_help(shortcuts, ShortcutAction::OpenActions, "Git actions"),
        shortcut_help(shortcuts, ShortcutAction::OpenGitCommand, "Git command"),
        shortcut_help(
            shortcuts,
            ShortcutAction::OpenHerdr,
            "Send to Herdr pane below",
        ),
        shortcut_pair_help(
            shortcuts,
            ShortcutAction::EditFile,
            ShortcutAction::ConfigureEditor,
            "Edit / configure editor",
        ),
        shortcut_help(
            shortcuts,
            ShortcutAction::ToggleMarkdown,
            "Markdown preview / source",
        ),
        shortcut_help(shortcuts, ShortcutAction::FindFile, "Find repository file"),
        shortcut_help(
            shortcuts,
            ShortcutAction::ToggleWrap,
            "Toggle preview wrapping",
        ),
    ];
    let worktree = vec![
        Line::styled(
            "CHANGES / FILES",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        help_line("← / h", "Tree / exit hunk"),
        help_line("→ / l", "Enter / stage hunk"),
        help_line("Enter", "Toggle folder"),
        shortcut_help(
            shortcuts,
            ShortcutAction::StageSelection,
            "Stage file / hunk",
        ),
        shortcut_help(
            shortcuts,
            ShortcutAction::DiscardChanges,
            "Discard unstaged file changes",
        ),
        shortcut_help(
            shortcuts,
            ShortcutAction::ToggleAgents,
            "Show / hide agents",
        ),
        shortcut_help(shortcuts, ShortcutAction::UnstageAll, "Unstage all"),
        shortcut_help(
            shortcuts,
            ShortcutAction::RenameFile,
            "Rename file / folder / workspace",
        ),
        shortcut_help(shortcuts, ShortcutAction::DeleteFile, "Delete from Files"),
        shortcut_help(
            shortcuts,
            ShortcutAction::SaveOrFormat,
            "Save editor / format file",
        ),
        help_line("Drag", "Move file / folder"),
        shortcut_help(shortcuts, ShortcutAction::FocusCommit, "Commit editor"),
        help_line("Arrow keys", "Commit cursor"),
        help_line("C-A / C-⌫", "Select all / del word"),
        shortcut_help(shortcuts, ShortcutAction::SubmitCommit, "Commit"),
        help_line("Esc", "Close / unfocus"),
        shortcut_help(shortcuts, ShortcutAction::Quit, "Quit"),
    ];
    frame.render_widget(Paragraph::new(navigation), columns[0]);
    frame.render_widget(Paragraph::new(worktree), columns[2]);
    frame.render_widget(
        Paragraph::new(format!(
            "{} / Esc close",
            shortcuts.label(ShortcutAction::OpenHelp)
        ))
        .style(Style::default().fg(palette().muted))
        .alignment(Alignment::Right),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
}

fn shortcut_help(
    shortcuts: &Shortcuts,
    action: ShortcutAction,
    description: &'static str,
) -> Line<'static> {
    help_line_owned(shortcuts.label(action), description)
}

fn shortcut_pair_help(
    shortcuts: &Shortcuts,
    first: ShortcutAction,
    second: ShortcutAction,
    description: &'static str,
) -> Line<'static> {
    help_line_owned(
        format!("{} / {}", shortcuts.label(first), shortcuts.label(second)),
        description,
    )
}

fn help_line_owned(key: String, description: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {key:<12}"),
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description, Style::default().fg(palette().ink)),
    ])
}

fn help_line<'a>(key: &'a str, description: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {key:<12}"),
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description, Style::default().fg(palette().ink)),
    ])
}
