use super::*;

pub(crate) struct SettingsView<'a> {
    pub(crate) settings: &'a Settings,
    pub(crate) page: SettingsPage,
    pub(crate) selection: usize,
    pub(crate) shortcut_selection: usize,
    pub(crate) shortcut_scroll: usize,
    pub(crate) shortcut_capture: bool,
    pub(crate) shortcut_error: Option<&'a str>,
}

pub(crate) struct SettingsRegions {
    pub(crate) targets: Vec<(HitTarget, Rect)>,
    pub(crate) shortcut_viewport: Option<usize>,
}

pub(crate) fn draw_settings(
    frame: &mut Frame<'_>,
    view: SettingsView<'_>,
    fetch_running: bool,
) -> SettingsRegions {
    let SettingsView {
        settings,
        page,
        selection,
        shortcut_selection,
        shortcut_scroll,
        shortcut_capture,
        shortcut_error,
    } = view;
    let area = centered_min(frame.area(), 58, 0, 48, 24);
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
        Paragraph::new("SETTINGS").style(
            Style::default()
                .fg(palette().ink)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x.saturating_add(2), area.y.saturating_add(1), 10, 1),
    );

    let shortcuts_tab = Rect::new(
        area.right().saturating_sub(14),
        area.y.saturating_add(1),
        12,
        1,
    );
    let general_tab = Rect::new(shortcuts_tab.x.saturating_sub(10), shortcuts_tab.y, 9, 1);
    let mut targets = vec![
        (HitTarget::Settings(SettingsHitTarget::Overlay), area),
        (
            HitTarget::Settings(SettingsHitTarget::Page(SettingsPage::General)),
            general_tab,
        ),
        (
            HitTarget::Settings(SettingsHitTarget::Page(SettingsPage::Shortcuts)),
            shortcuts_tab,
        ),
    ];
    for (label, rect, active) in [
        (" General ", general_tab, page == SettingsPage::General),
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
        for (row, (index, definition)) in Shortcuts::definitions()
            .enumerate()
            .skip(shortcut_scroll)
            .take(usize::from(body.height))
            .enumerate()
        {
            let rect = Rect::new(body.x, body.y.saturating_add(row as u16), body.width, 1);
            let selected = index == shortcut_selection;
            let binding = if selected && shortcut_capture {
                "press a key...".to_owned()
            } else {
                settings.shortcuts.label(definition.action)
            };
            let marker = if settings.shortcuts.is_overridden(definition.action) {
                "*"
            } else {
                " "
            };
            let prefix = format!("{marker} {} / {}", definition.section, definition.label);
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
        draw_footer(frame, area, footer, shortcut_error.is_some());
        return SettingsRegions {
            targets,
            shortcut_viewport: Some(usize::from(body.height)),
        };
    }

    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(4),
        area.width.saturating_sub(4),
        area.height.saturating_sub(6),
    );
    frame.render_widget(
        Paragraph::new("AUTOMATION").style(
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let compact = inner.height < 15;
    let row_offsets = if compact {
        [1, 2, 3, 5, 6, 7]
    } else {
        [2, 4, 6, 10, 12, 14]
    };
    let rows = row_offsets
        .map(|offset| Rect::new(inner.x, inner.y.saturating_add(offset), inner.width, 1));
    let interval_down = Rect::new(rows[1].right().saturating_sub(15), rows[1].y, 3, 1);
    let interval_up = Rect::new(rows[1].right().saturating_sub(3), rows[1].y, 3, 1);

    draw_setting_row(
        frame,
        rows[0],
        "Auto-fetch remotes",
        if settings.auto_fetch { "On" } else { "Off" },
        selection == 0,
    );
    draw_setting_row(
        frame,
        rows[1],
        "Fetch interval",
        &format!("[-] {:>4} min [+]", settings.fetch_interval_minutes),
        selection == 1,
    );
    draw_setting_row(
        frame,
        rows[2],
        "Format on save",
        if settings.format_on_save { "On" } else { "Off" },
        selection == 2,
    );
    frame.render_widget(
        Paragraph::new("INTERFACE").style(
            Style::default()
                .fg(palette().muted)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            inner.x,
            inner.y.saturating_add(if compact { 4 } else { 8 }),
            inner.width,
            1,
        ),
    );
    draw_setting_row(
        frame,
        rows[3],
        "Media protocol",
        media_preview_protocol_label(settings.media_preview_protocol),
        selection == 3,
    );
    draw_setting_row(
        frame,
        rows[4],
        "Sixel quality",
        settings.sixel_quality.label(),
        selection == 4,
    );
    draw_setting_row(
        frame,
        rows[5],
        "Editor command",
        settings
            .editor_command
            .as_deref()
            .unwrap_or("Not configured"),
        selection == 5,
    );

    let footer = if fetch_running {
        "Fetching remotes now..."
    } else {
        "Space toggle   Left/Right adjust   Enter edit   Esc close"
    };
    draw_footer(frame, area, footer, false);
    targets.extend([
        (HitTarget::Settings(SettingsHitTarget::AutoFetch), rows[0]),
        (
            HitTarget::Settings(SettingsHitTarget::FetchInterval),
            rows[1],
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
            rows[2],
        ),
        (
            HitTarget::Settings(SettingsHitTarget::MediaPreview),
            rows[3],
        ),
        (
            HitTarget::Settings(SettingsHitTarget::SixelQuality),
            rows[4],
        ),
        (HitTarget::Settings(SettingsHitTarget::Editor), rows[5]),
    ]);
    SettingsRegions {
        targets,
        shortcut_viewport: None,
    }
}

fn draw_setting_row(frame: &mut Frame<'_>, area: Rect, label: &str, value: &str, selected: bool) {
    let value = truncate_width(
        value,
        usize::from(area.width).saturating_sub(label.len() + 2),
    );
    let padding = usize::from(area.width)
        .saturating_sub(UnicodeWidthStr::width(label) + UnicodeWidthStr::width(value.as_str()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(label.to_owned(), Style::default().fg(palette().ink)),
            Span::raw(" ".repeat(padding)),
            Span::styled(value, Style::default().fg(palette().accent)),
        ]))
        .style(Style::default().bg(if selected {
            palette().selected
        } else {
            palette().surface_alt
        })),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, footer: &str, error: bool) {
    frame.render_widget(
        Paragraph::new(truncate_width(
            footer,
            usize::from(area.width.saturating_sub(4)),
        ))
        .style(Style::default().fg(if error {
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
