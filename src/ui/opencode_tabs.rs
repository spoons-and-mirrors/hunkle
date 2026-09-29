use super::*;

/// Norm-style cards with local grouping; OpenCode owns their identity and active state.
pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 20 || area.height < 5 {
        return;
    }
    draw_groups(frame, app, Rect::new(area.x, area.y, area.width, 1));
    let instance_id = app.opencode_presence.instance_id().map(str::to_owned);
    let groups = &app.opencode_groups;
    let tabs = &mut app.opencode_presence.tabs;
    let visible = groups
        .ordered_tabs(&tabs.items)
        .into_iter()
        .filter(|&index| groups.visible(groups.group_for(&tabs.items[index].session_id)))
        .collect::<Vec<_>>();
    let viewport = Rect::new(area.x + 4, area.y + 2, area.width - 5, 3);
    let divider_y = viewport.bottom();
    if area.height >= 6 {
        frame.render_widget(
            Paragraph::new("─".repeat(usize::from(area.width)))
                .style(Style::default().fg(palette().faint)),
            Rect::new(area.x, divider_y, area.width, 1),
        );
    }
    tabs.reveal_active |= tabs.viewport_width != viewport.width;
    tabs.viewport_width = viewport.width;
    let mut total = 0usize;
    let cards = visible
        .into_iter()
        .map(|index| {
            let tab = &tabs.items[index];
            let project = tab.project.clone().unwrap_or_else(|| {
                tab.directory
                    .as_deref()
                    .map(|path| path.file_name().unwrap_or(path.as_os_str()))
                    .map(|name| {
                        name.to_string_lossy()
                            .chars()
                            .filter(|character| !character.is_control())
                            .collect::<String>()
                    })
                    .unwrap_or_else(|| "Loading…".to_owned())
            });
            let width = (project.width() + 5)
                .max(tab.branch.as_deref().unwrap_or("—").width() + 3)
                .max(tab.title.width() + 2)
                .clamp(10, 24)
                .min(usize::from(viewport.width));
            let start = total;
            total += width + 1;
            (index, project, start, width)
        })
        .collect::<Vec<_>>();
    let total = total.saturating_sub(1);
    let maximum = total.saturating_sub(usize::from(viewport.width));
    if tabs.reveal_active {
        if let Some(&(_, _, left, width)) = cards
            .iter()
            .find(|&&(index, _, _, _)| tabs.items[index].active)
        {
            let right = left + width;
            if left < tabs.scroll {
                tabs.scroll = left;
            } else if right > tabs.scroll + usize::from(viewport.width) {
                tabs.scroll = right - usize::from(viewport.width);
            }
        }
        tabs.reveal_active = false;
    }
    tabs.scroll = tabs.scroll.min(maximum);
    app.regions.register_scroll_target_with_state(
        ScrollTarget::OpenCodeTabs,
        Rect::new(area.x, viewport.y, area.width, 3),
        tabs.scroll,
        maximum,
    );
    frame.render_widget(
        Paragraph::new("OC").style(Style::default().fg(palette().muted)),
        Rect::new(area.x, viewport.y, 2, 1),
    );
    if cards.is_empty() {
        frame.render_widget(
            Paragraph::new(if tabs.items.is_empty() {
                "Waiting for OpenCode tabs…"
            } else {
                "No cards in visible groups"
            })
            .style(Style::default().fg(palette().muted)),
            viewport,
        );
    }
    for (tab_index, project, start, width) in cards {
        let tab = &tabs.items[tab_index];
        let end = start + width;
        let left = start.max(tabs.scroll);
        let right = end.min(tabs.scroll + usize::from(viewport.width));
        if left >= right {
            continue;
        }
        let dropping = groups.drag.as_ref().is_some_and(|drag| {
            drag.moved && drag.card_target.as_deref() == Some(tab.session_id.as_str())
        });
        let background = if dropping {
            palette().selected
        } else {
            header_card_background(tab.active)
        };
        let border = Style::default().fg(group_color(groups.group_for(&tab.session_id)));
        let (indicator, color) = if tab.attention {
            ("◆", palette().yellow)
        } else if tab.unread.as_deref() == Some("error") {
            ("×", palette().red)
        } else if tab.busy {
            ("◌", palette().orange)
        } else if tab.unread.is_some() {
            ("•", palette().cyan)
        } else {
            ("●", palette().green)
        };
        let title = truncate_width(&project, width.saturating_sub(5));
        let title_style = Style::default().fg(if tab.active {
            palette().ink
        } else {
            palette().soft
        });
        let lines = vec![
            Line::from(vec![
                Span::styled("▌", border),
                Span::raw(" "),
                Span::styled(indicator, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(title, title_style.add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("▌", border),
                Span::raw(" "),
                Span::styled(
                    truncate_width(
                        tab.branch.as_deref().unwrap_or("—"),
                        width.saturating_sub(3),
                    ),
                    Style::default().fg(palette().muted),
                ),
            ]),
        ];
        let rect = Rect::new(
            viewport.x + (left - tabs.scroll) as u16,
            viewport.y,
            (right - left) as u16,
            2,
        );
        if let Some(instance_id) = &instance_id {
            app.regions.register_hit_target(
                HitTarget::OpenCodeTab {
                    instance_id: instance_id.clone(),
                    session_id: tab.session_id.clone(),
                },
                Rect::new(rect.x, rect.y, rect.width, 3),
            );
        }
        frame.render_widget(
            Paragraph::new(lines)
                .style(Style::default().bg(background))
                .scroll((0, (left - start) as u16)),
            rect,
        );
        let name_style = if tab.active {
            Style::default()
                .fg(palette().orange)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette().soft)
        };
        frame.render_widget(
            Paragraph::new(format!(
                " {}",
                truncate_width(&tab.title, width.saturating_sub(2))
            ))
            .style(name_style)
            .scroll((0, (left - start) as u16)),
            Rect::new(rect.x, viewport.bottom() - 1, rect.width, 1),
        );
        if tab.active && area.height >= 6 {
            frame.render_widget(
                Paragraph::new("─".repeat(usize::from(rect.width)))
                    .style(Style::default().fg(palette().orange)),
                Rect::new(rect.x, divider_y, rect.width, 1),
            );
        }
    }
    for (x, symbol, visible) in [
        (viewport.x - 1, "‹", tabs.scroll > 0),
        (viewport.right(), "›", tabs.scroll < maximum),
    ] {
        if visible {
            frame.render_widget(
                Paragraph::new(symbol).style(Style::default().fg(palette().accent)),
                Rect::new(x, viewport.y, 1, 1),
            );
        }
    }
}

fn group_color(id: u64) -> Color {
    let colors = [
        palette().accent,
        palette().purple,
        palette().green,
        palette().orange,
        palette().cyan,
        palette().yellow,
    ];
    colors[((id - 1) % colors.len() as u64) as usize]
}

fn draw_groups(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let groups = &mut app.opencode_groups;
    if let Some(edit) = &groups.edit {
        frame.render_widget(
            Paragraph::new("Name ").style(Style::default().fg(palette().muted)),
            Rect::new(area.x, area.y, 5, 1),
        );
        let input = Rect::new(area.x + 5, area.y, area.width - 13, 1);
        let cursor = edit.input.text()[..edit.input.cursor()].width();
        let scroll = cursor.saturating_sub(usize::from(input.width.saturating_sub(1)));
        frame.render_widget(
            Paragraph::new(text_input_lines(&edit.input, true, palette().ink))
                .style(Style::default().bg(palette().raised))
                .scroll((0, scroll.min(u16::MAX as usize) as u16)),
            input,
        );
        app.regions
            .register_hit_target(HitTarget::OpenCodeGroupName, input);
        for (x, text, target) in [
            (area.right() - 7, "[✓]", HitTarget::OpenCodeGroupSave),
            (area.right() - 3, "[×]", HitTarget::OpenCodeGroupCancel),
        ] {
            let rect = Rect::new(x, area.y, 3, 1);
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(palette().accent)),
                rect,
            );
            app.regions.register_hit_target(target, rect);
        }
        return;
    }
    let add = Rect::new(area.x, area.y, 3, 1);
    frame.render_widget(
        Paragraph::new("[+]").style(Style::default().fg(palette().accent)),
        add,
    );
    app.regions
        .register_hit_target(HitTarget::OpenCodeGroupAdd, add);
    let viewport = Rect::new(area.x + 4, area.y, area.width - 9, 1);
    let mut counts = std::collections::BTreeMap::new();
    for tab in &app.opencode_presence.tabs.items {
        *counts
            .entry(groups.group_for(&tab.session_id))
            .or_insert(0usize) += 1;
    }
    let labels = groups
        .catalog
        .groups
        .iter()
        .map(|group| {
            let count = counts.get(&group.id).copied().unwrap_or(0);
            let marker = if groups.catalog.solo == Some(group.id) {
                "◆"
            } else if groups.visible(group.id) {
                "●"
            } else {
                "○"
            };
            format!(" {marker} {} {count} ", truncate_width(&group.name, 16))
        })
        .collect::<Vec<_>>();
    let total = labels
        .iter()
        .map(|label| label.width() + 1)
        .sum::<usize>()
        .saturating_sub(1);
    let maximum = total.saturating_sub(usize::from(viewport.width));
    groups.scroll = groups.scroll.min(maximum);
    app.regions.register_scroll_target_with_state(
        ScrollTarget::OpenCodeGroups,
        viewport,
        groups.scroll,
        maximum,
    );
    let mut start = 0;
    for (group, label) in groups.catalog.groups.iter().zip(labels.iter()) {
        let end = start + label.width();
        let left = start.max(groups.scroll);
        let right = end.min(groups.scroll + usize::from(viewport.width));
        if right > left {
            let dropping = groups
                .drag
                .as_ref()
                .is_some_and(|drag| drag.target == Some(group.id));
            let style = if dropping {
                Style::default()
                    .fg(palette().canvas)
                    .bg(group_color(group.id))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(if groups.visible(group.id) {
                        group_color(group.id)
                    } else {
                        palette().muted
                    })
                    .bg(
                        if groups.catalog.solo == Some(group.id)
                            || app.hovered_hit_target == Some(HitTarget::OpenCodeGroup(group.id))
                        {
                            palette().raised
                        } else {
                            palette().panel
                        },
                    )
            };
            let rect = Rect::new(
                viewport.x + (left - groups.scroll) as u16,
                area.y,
                (right - left) as u16,
                1,
            );
            frame.render_widget(
                Paragraph::new(label.as_str())
                    .style(style)
                    .scroll((0, (left - start) as u16)),
                rect,
            );
            app.regions
                .register_hit_target(HitTarget::OpenCodeGroup(group.id), rect);
        }
        start = end + 1;
    }
    for (x, text, delta, enabled) in [
        (area.right() - 4, "‹ ", -16, groups.scroll > 0),
        (area.right() - 2, " ›", 16, groups.scroll < maximum),
    ] {
        if enabled {
            let rect = Rect::new(x, area.y, 2, 1);
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(palette().accent)),
                rect,
            );
            app.regions
                .register_hit_target(HitTarget::OpenCodeGroupScroll(delta), rect);
        }
    }
}
