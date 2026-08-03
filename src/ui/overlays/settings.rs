use super::*;

pub(crate) struct SettingsView<'a> {
    pub(crate) settings: &'a Settings,
    pub(crate) page: SettingsPage,
    pub(crate) selection: usize,
    pub(crate) shortcut_selection: usize,
    pub(crate) shortcut_scroll: usize,
    pub(crate) shortcut_capture: bool,
    pub(crate) shortcut_error: Option<&'a str>,
    pub(crate) opencode_selection: usize,
    pub(crate) opencode_model_input: Option<&'a str>,
    pub(crate) opencode_error: Option<&'a str>,
}

pub(crate) fn draw_settings(
    frame: &mut Frame<'_>,
    view: SettingsView<'_>,
    fetch_running: bool,
) -> Vec<(HitTarget, Rect)> {
    let SettingsView {
        settings,
        page,
        selection,
        shortcut_selection,
        shortcut_scroll,
        shortcut_capture,
        shortcut_error,
        opencode_selection,
        opencode_model_input,
        opencode_error,
    } = view;
    let area = centered_min(frame.area(), 58, 0, 48, 30);
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
                "SETTINGS",
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  Application preferences",
                Style::default().fg(palette().faint),
            ),
        ])),
        Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(1),
            area.width.saturating_sub(4),
            1,
        ),
    );
    let shortcuts_tab = Rect::new(
        area.right().saturating_sub(14),
        area.y.saturating_add(1),
        12,
        1,
    );
    let opencode_tab = Rect::new(shortcuts_tab.x.saturating_sub(13), shortcuts_tab.y, 12, 1);
    let general_tab = Rect::new(opencode_tab.x.saturating_sub(11), opencode_tab.y, 10, 1);
    let mut targets = vec![
        (HitTarget::Settings(SettingsHitTarget::Overlay), area),
        (
            HitTarget::Settings(SettingsHitTarget::Page(SettingsPage::General)),
            general_tab,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::Page(SettingsPage::OpenCode)),
            opencode_tab,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::Page(SettingsPage::Shortcuts)),
            shortcuts_tab,
        ),
    ];
    for (label, rect, active) in [
        (" General ", general_tab, page == SettingsPage::General),
        (" OpenCode ", opencode_tab, page == SettingsPage::OpenCode),
        (
            " Shortcuts ",
            shortcuts_tab,
            page == SettingsPage::Shortcuts,
        ),
    ] {
        frame.render_widget(
            Paragraph::new(label).style(
                Style::default()
                    .fg(if active {
                        palette().accent
                    } else {
                        palette().muted
                    })
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            rect,
        );
    }
    if page == SettingsPage::Shortcuts {
        let body = Rect::new(
            area.x.saturating_add(2),
            area.y.saturating_add(4),
            area.width.saturating_sub(4),
            area.height.saturating_sub(6),
        );
        let definitions = Shortcuts::definitions();
        for (row, (index, definition)) in definitions
            .iter()
            .enumerate()
            .skip(shortcut_scroll)
            .take(usize::from(body.height))
            .enumerate()
        {
            let rect = Rect::new(body.x, body.y.saturating_add(row as u16), body.width, 1);
            let selected = index == shortcut_selection;
            let binding = if selected && shortcut_capture {
                "press a key…".to_owned()
            } else {
                settings.shortcuts.label(definition.action)
            };
            let marker = if settings.shortcuts.is_overridden(definition.action) {
                "•"
            } else {
                " "
            };
            let prefix = format!("{marker} {} · {}", definition.section, definition.label);
            let padding = usize::from(rect.width)
                .saturating_sub(UnicodeWidthStr::width(prefix.as_str()) + binding.len());
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(palette().ink)),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(
                        binding,
                        Style::default().fg(if selected && shortcut_capture {
                            palette().orange
                        } else {
                            palette().accent
                        }),
                    ),
                ]))
                .style(Style::default().bg(if selected {
                    palette().selected
                } else {
                    palette().surface_alt
                })),
                rect,
            );
            targets.push((
                HitTarget::Settings(SettingsHitTarget::Shortcut(definition.action)),
                rect,
            ));
        }
        let footer = shortcut_error.unwrap_or(if shortcut_capture {
            "Press a key   Esc cancel"
        } else {
            "Enter change   Delete reset   Tab general   Esc close"
        });
        frame.render_widget(
            Paragraph::new(truncate_width(
                footer,
                usize::from(area.width.saturating_sub(4)),
            ))
            .style(Style::default().fg(if shortcut_error.is_some() {
                palette().red
            } else {
                palette().muted
            }))
            .alignment(Alignment::Right),
            Rect::new(
                area.x.saturating_add(2),
                area.bottom().saturating_sub(1),
                area.width.saturating_sub(4),
                1,
            ),
        );
        return targets;
    }
    if page == SettingsPage::OpenCode {
        let inner = Rect::new(
            area.x.saturating_add(2),
            area.y,
            area.width.saturating_sub(4),
            area.height,
        );
        let model_row = Rect::new(inner.x, area.y.saturating_add(7), inner.width, 1);
        let reasoning_row = Rect::new(inner.x, area.y.saturating_add(12), inner.width, 1);
        frame.render_widget(
            Paragraph::new(Line::styled(
                "COMMIT MESSAGE GENERATION",
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            )),
            Rect::new(inner.x, area.y.saturating_add(4), inner.width, 1),
        );
        frame.render_widget(
            Paragraph::new("OpenCode generates commit messages from the current Git diff.")
                .style(Style::default().fg(palette().faint)),
            Rect::new(inner.x, area.y.saturating_add(5), inner.width, 1),
        );

        let editing = opencode_model_input.is_some();
        let model = opencode_model_input.unwrap_or(&settings.opencode_model);
        let model = truncate_width(model, usize::from(model_row.width).saturating_sub(15));
        let model_padding = usize::from(model_row.width)
            .saturating_sub("Model".len() + model.len() + usize::from(editing));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Model", Style::default().fg(palette().ink)),
                Span::raw(" ".repeat(model_padding)),
                Span::styled(model, Style::default().fg(palette().accent)),
                Span::styled(
                    if editing { "▏" } else { "" },
                    Style::default().fg(palette().orange),
                ),
            ]))
            .style(Style::default().bg(if opencode_selection == 0 {
                palette().selected
            } else {
                palette().surface_alt
            })),
            model_row,
        );
        frame.render_widget(
            Paragraph::new("Use any model ID reported by `opencode models`.")
                .style(Style::default().fg(palette().faint)),
            Rect::new(inner.x, area.y.saturating_add(8), inner.width, 1),
        );

        let reasoning = settings.opencode_reasoning.label();
        let reasoning_padding = usize::from(reasoning_row.width)
            .saturating_sub("Reasoning".len() + reasoning.len() + 4);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Reasoning", Style::default().fg(palette().ink)),
                Span::raw(" ".repeat(reasoning_padding)),
                Span::styled("← ", Style::default().fg(palette().muted)),
                Span::styled(reasoning, Style::default().fg(palette().accent)),
                Span::styled(" →", Style::default().fg(palette().muted)),
            ]))
            .style(Style::default().bg(if opencode_selection == 1 {
                palette().selected
            } else {
                palette().surface_alt
            })),
            reasoning_row,
        );
        frame.render_widget(
            Paragraph::new("Default lets the selected model choose its own variant.")
                .style(Style::default().fg(palette().faint)),
            Rect::new(inner.x, area.y.saturating_add(13), inner.width, 1),
        );

        let footer = opencode_error.unwrap_or(if editing {
            "Enter save   Ctrl+U clear   Esc cancel"
        } else {
            "Enter edit   ←/→ reasoning   Tab next   Esc close"
        });
        frame.render_widget(
            Paragraph::new(truncate_width(
                footer,
                usize::from(area.width.saturating_sub(4)),
            ))
            .style(Style::default().fg(if opencode_error.is_some() {
                palette().red
            } else {
                palette().muted
            }))
            .alignment(Alignment::Right),
            Rect::new(
                area.x.saturating_add(2),
                area.bottom().saturating_sub(1),
                area.width.saturating_sub(4),
                1,
            ),
        );
        targets.extend([
            (
                HitTarget::Settings(SettingsHitTarget::OpenCodeModel),
                model_row,
            ),
            (
                HitTarget::Settings(SettingsHitTarget::OpenCodeReasoning),
                reasoning_row,
            ),
        ]);
        return targets;
    }
    frame.render_widget(
        Paragraph::new("Space toggle   ←/→ interval   Enter edit   Esc close")
            .style(Style::default().fg(palette().muted))
            .alignment(Alignment::Right),
        Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(1),
            area.width.saturating_sub(4),
            1,
        ),
    );

    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(4),
        area.height,
    );
    let compact = area.height < 28;
    let automation_header_y = if compact { 3 } else { 4 };
    let auto_y = if compact { 4 } else { 7 };
    let interval_y = if compact { 5 } else { 9 };
    let format_on_save_y = if compact { 6 } else { 11 };
    let interface_header_y = if compact { 8 } else { 14 };
    let cross_workspace_y = if compact { 9 } else { 15 };
    let agent_y = if compact { 10 } else { 17 };
    let agent_time_y = if compact { 11 } else { 19 };
    let clear_timings_y = if compact { 12 } else { 21 };
    let media_y = if compact { 13 } else { 23 };
    let editor_y = if compact { 14 } else { 25 };
    let auto_row = Rect::new(inner.x, area.y.saturating_add(auto_y), inner.width, 1);
    let interval_row = Rect::new(inner.x, area.y.saturating_add(interval_y), inner.width, 1);
    let format_on_save_row = Rect::new(
        inner.x,
        area.y.saturating_add(format_on_save_y),
        inner.width,
        1,
    );
    let cross_workspace_agents_row = Rect::new(
        inner.x,
        area.y.saturating_add(cross_workspace_y),
        inner.width,
        1,
    );
    let agent_harness_row = Rect::new(inner.x, area.y.saturating_add(agent_y), inner.width, 1);
    let agent_time_row = Rect::new(inner.x, area.y.saturating_add(agent_time_y), inner.width, 1);
    let clear_agent_timings_row = Rect::new(
        inner.x,
        area.y.saturating_add(clear_timings_y),
        inner.width,
        1,
    );
    let media_preview_row = Rect::new(inner.x, area.y.saturating_add(media_y), inner.width, 1);
    let editor_row = Rect::new(inner.x, area.y.saturating_add(editor_y), inner.width, 1);
    let interval_down = Rect::new(
        interval_row.right().saturating_sub(15),
        interval_row.y,
        3,
        1,
    );
    let interval_up = Rect::new(interval_row.right().saturating_sub(3), interval_row.y, 3, 1);

    let media_protocol_label = media_preview_protocol_label(settings.media_preview_protocol);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Media protocol", Style::default().fg(palette().ink)),
            Span::raw(
                " ".repeat(
                    usize::from(media_preview_row.width)
                        .saturating_sub("Media protocol".len() + media_protocol_label.len()),
                ),
            ),
            Span::styled(media_protocol_label, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selection == 7 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        media_preview_row,
    );

    frame.render_widget(
        Paragraph::new(Line::styled(
            "AUTOMATION",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(
            inner.x,
            area.y.saturating_add(automation_header_y),
            inner.width,
            1,
        ),
    );
    if !compact {
        let description = truncate_width(
            "Fetch updated remote refs in the background",
            usize::from(inner.width),
        );
        frame.render_widget(
            Paragraph::new(description).style(Style::default().fg(palette().faint)),
            Rect::new(inner.x, area.y.saturating_add(5), inner.width, 1),
        );
    }

    let (auto_switch, auto_switch_color) = settings_toggle(settings.auto_fetch);
    let auto_padding =
        usize::from(auto_row.width).saturating_sub(19 + UnicodeWidthStr::width(auto_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Auto-fetch remotes", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(auto_padding)),
            Span::styled(
                auto_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(auto_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 0 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        auto_row,
    );

    let interval_control = format!("[-] {:>4} min [+]", settings.fetch_interval_minutes);
    let interval_padding = usize::from(interval_row.width)
        .saturating_sub("Fetch interval".len() + interval_control.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Fetch interval", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(interval_padding)),
            Span::styled(interval_control, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selection == 1 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        interval_row,
    );

    let (format_switch, format_switch_color) = settings_toggle(settings.format_on_save);
    let format_padding = usize::from(format_on_save_row.width)
        .saturating_sub(14 + UnicodeWidthStr::width(format_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Format on save", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(format_padding)),
            Span::styled(
                format_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(format_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 2 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        format_on_save_row,
    );

    let status = if fetch_running {
        "Fetching remotes now...".to_owned()
    } else if settings.auto_fetch {
        format!(
            "Enabled · every {} minute{}",
            settings.fetch_interval_minutes,
            if settings.fetch_interval_minutes == 1 {
                ""
            } else {
                "s"
            }
        )
    } else {
        "Disabled".to_owned()
    };
    if !compact {
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(if settings.auto_fetch {
                palette().green
            } else {
                palette().faint
            })),
            Rect::new(inner.x, area.y.saturating_add(12), inner.width, 1),
        );
    }

    frame.render_widget(
        Paragraph::new(Line::styled(
            "INTERFACE",
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(
            inner.x,
            area.y.saturating_add(interface_header_y),
            inner.width,
            1,
        ),
    );
    let (cross_workspace_switch, cross_workspace_switch_color) =
        settings_toggle(settings.cross_workspace_agents);
    let cross_workspace_padding = usize::from(cross_workspace_agents_row.width)
        .saturating_sub(22 + UnicodeWidthStr::width(cross_workspace_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Cross-workspace agents", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(cross_workspace_padding)),
            Span::styled(
                cross_workspace_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(cross_workspace_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 3 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        cross_workspace_agents_row,
    );

    let (agent_harness_switch, agent_harness_switch_color) =
        settings_toggle(settings.show_agent_harness);
    let agent_harness_padding = usize::from(agent_harness_row.width)
        .saturating_sub(14 + UnicodeWidthStr::width(agent_harness_switch));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agent harness", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(agent_harness_padding)),
            Span::styled(
                agent_harness_switch,
                Style::default()
                    .fg(palette().canvas)
                    .bg(agent_harness_switch_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(if selection == 4 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        agent_harness_row,
    );

    let agent_time_label = settings.agent_time_display.label();
    let agent_time_padding = usize::from(agent_time_row.width)
        .saturating_sub("Agent time".len() + agent_time_label.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agent time", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(agent_time_padding)),
            Span::styled(agent_time_label, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selection == 5 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        agent_time_row,
    );

    let clear_label = "Clear";
    let clear_padding = usize::from(clear_agent_timings_row.width)
        .saturating_sub("Agent timing history".len() + clear_label.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Agent timing history", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(clear_padding)),
            Span::styled(clear_label, Style::default().fg(palette().orange)),
        ]))
        .style(Style::default().bg(if selection == 6 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        clear_agent_timings_row,
    );

    let editor = settings
        .editor_command
        .as_deref()
        .unwrap_or("Not configured");
    let editor = truncate_width(editor, usize::from(editor_row.width).saturating_sub(17));
    let editor_padding =
        usize::from(editor_row.width).saturating_sub("Editor command".len() + editor.len());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Editor command", Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(editor_padding)),
            Span::styled(
                editor,
                Style::default().fg(if settings.editor_command.is_some() {
                    palette().accent
                } else {
                    palette().muted
                }),
            ),
        ]))
        .style(Style::default().bg(if selection == 8 {
            palette().selected
        } else {
            palette().surface_alt
        })),
        editor_row,
    );

    targets.extend([
        (HitTarget::Settings(SettingsHitTarget::AutoFetch), auto_row),
        (
            HitTarget::Settings(SettingsHitTarget::FetchInterval),
            interval_row,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::FetchIntervalDown),
            interval_down,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::FetchIntervalUp),
            interval_up,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::FormatOnSave),
            format_on_save_row,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::CrossWorkspaceAgents),
            cross_workspace_agents_row,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::AgentHarness),
            agent_harness_row,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::AgentTime),
            agent_time_row,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::ClearAgentTimings),
            clear_agent_timings_row,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::MediaPreview),
            media_preview_row,
        ),
        (HitTarget::Settings(SettingsHitTarget::Editor), editor_row),
    ]);
    targets
}

fn media_preview_protocol_label(protocol: crate::media::MediaPreviewProtocol) -> &'static str {
    match protocol {
        crate::media::MediaPreviewProtocol::Auto => "Auto",
        crate::media::MediaPreviewProtocol::Halfblocks => "Unicode",
        crate::media::MediaPreviewProtocol::Kitty => "Kitty (Ghostty)",
        crate::media::MediaPreviewProtocol::Iterm2 => "iTerm2 (WezTerm)",
        crate::media::MediaPreviewProtocol::Sixel => "Sixel (Windows Terminal)",
    }
}

fn settings_toggle(enabled: bool) -> (&'static str, Color) {
    if enabled {
        ("   ◼ ", palette().green)
    } else {
        (" ◼   ", palette().faint)
    }
}
