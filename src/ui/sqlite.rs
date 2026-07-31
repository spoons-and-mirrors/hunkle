use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, HitTarget, SqliteFocus, SqlitePage},
    ui::{palette, truncate_width},
};

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    app.regions.diff_scroll_max = 0;
    app.regions.diff_scrollbar = None;
    app.regions.diff_scroll_thumb = None;
    let Some(browser) = app.changes.sqlite_browser.as_ref() else {
        return;
    };
    let active = browser.active;
    let focus = browser.focus;
    let wide = area.width >= 68;
    let sections = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
    let content = sections[0];
    let footer = sections[1];

    if wide {
        let columns = Layout::horizontal([
            Constraint::Length(content.width.min(26)),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .split(content);
        draw_objects(frame, app, columns[0]);
        frame.render_widget(
            Paragraph::new(
                (0..columns[1].height)
                    .map(|_| Line::styled("│", Style::default().fg(palette().faint)))
                    .collect::<Vec<_>>(),
            ),
            columns[1],
        );
        draw_rows(frame, app, columns[2]);
    } else if !active || focus == SqliteFocus::Objects {
        draw_objects(frame, app, content);
    } else {
        draw_rows(frame, app, content);
    }

    let hint = if !active {
        "Enter explore"
    } else if focus == SqliteFocus::Objects {
        "↑↓ objects   Enter rows   Shift+Tab pane   Esc files"
    } else {
        "↑↓ rows   ←→ columns   PgUp/PgDn page   Shift+Tab pane   Esc files"
    };
    let (has_previous, has_next) = app
        .changes
        .sqlite_browser
        .as_ref()
        .and_then(|browser| browser.page.as_ref())
        .map_or((false, false), |page| (page.key.offset > 0, page.has_next));
    let show_paging =
        active && focus == SqliteFocus::Rows && footer.width >= 40 && (has_previous || has_next);
    let hint_area = Rect::new(
        footer.x,
        footer.y,
        footer
            .width
            .saturating_sub(if show_paging { 22 } else { 0 }),
        1,
    );
    frame.render_widget(
        Paragraph::new(truncate_width(hint, usize::from(hint_area.width)))
            .style(Style::default().fg(palette().muted)),
        hint_area,
    );
    if show_paging {
        let previous = Rect::new(footer.right().saturating_sub(22), footer.y, 10, 1);
        let next = Rect::new(footer.right().saturating_sub(11), footer.y, 11, 1);
        frame.render_widget(
            Paragraph::new("‹ previous").style(Style::default().fg(if has_previous {
                palette().accent
            } else {
                palette().faint
            })),
            previous,
        );
        frame.render_widget(
            Paragraph::new("next ›").style(Style::default().fg(if has_next {
                palette().accent
            } else {
                palette().faint
            })),
            next,
        );
        if has_previous && let Some(target) = app.changes.sqlite_page_target(false) {
            app.regions
                .register_hit_target(HitTarget::Changes(target), previous);
        }
        if has_next && let Some(target) = app.changes.sqlite_page_target(true) {
            app.regions
                .register_hit_target(HitTarget::Changes(target), next);
        }
    }
}

fn draw_objects(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(browser) = app.changes.sqlite_browser.as_ref() else {
        return;
    };
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let count_suffix = if browser.objects_truncated { "+" } else { "" };
    let metadata = format!(
        "OBJECTS  {}{}  {}  v{}",
        browser.objects.len(),
        count_suffix,
        display_file_size(browser.file_size),
        browser.user_version
    );
    let focused = browser.active && browser.focus == SqliteFocus::Objects;
    frame.render_widget(
        Paragraph::new(truncate_width(&metadata, usize::from(parts[0].width))).style(
            Style::default()
                .fg(if focused {
                    palette().accent
                } else {
                    palette().muted
                })
                .add_modifier(Modifier::BOLD),
        ),
        parts[0],
    );
    app.regions.sqlite_objects = Some(parts[1]);
    if let Some(target) = app.changes.sqlite_objects_target() {
        app.regions
            .register_hit_target(HitTarget::Changes(target), parts[1]);
    }

    let start = browser.object_scroll.min(
        browser
            .objects
            .len()
            .saturating_sub(usize::from(parts[1].height)),
    );
    if browser.objects.is_empty() {
        frame.render_widget(
            Paragraph::new("No user tables or views").style(Style::default().fg(palette().faint)),
            parts[1],
        );
        return;
    }
    let lines = browser
        .objects
        .iter()
        .enumerate()
        .skip(start)
        .take(usize::from(parts[1].height))
        .map(|(index, object)| {
            let selected = browser.selected_object == Some(index);
            let kind_width = object.kind.len().saturating_add(2);
            let name_width = usize::from(parts[1].width).saturating_sub(kind_width + 2);
            let line = Line::from(vec![
                Span::styled(
                    if selected { "▌ " } else { "  " },
                    Style::default().fg(palette().accent),
                ),
                Span::styled(
                    truncate_width(&safe_label(&object.name), name_width),
                    Style::default()
                        .fg(palette().ink)
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("  {}", object.kind.to_ascii_uppercase()),
                    Style::default().fg(palette().faint),
                ),
            ]);
            (index, selected, line)
        })
        .collect::<Vec<_>>();
    for (visible, (index, _, _)) in lines.iter().enumerate() {
        if let Some(target) = app.changes.sqlite_object_target(*index) {
            app.regions.register_hit_target(
                HitTarget::Changes(target),
                Rect::new(
                    parts[1].x,
                    parts[1].y.saturating_add(visible as u16),
                    parts[1].width,
                    1,
                ),
            );
        }
    }
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .map(|(_, selected, line)| {
                    if selected {
                        line.style(Style::default().bg(if focused {
                            palette().selected
                        } else {
                            palette().inactive_selected
                        }))
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>(),
        ),
        parts[1],
    );
}

fn draw_rows(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(browser) = app.changes.sqlite_browser.as_ref() else {
        return;
    };
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let focused = browser.active && browser.focus == SqliteFocus::Rows;
    let object = browser.selected_object();
    let mut title = match (object, &browser.page) {
        (Some(object), Some(page)) => {
            let first = page.key.offset.saturating_add(1);
            let last = page.key.offset.saturating_add(page.rows.len());
            format!(
                "{} · {}  rows {first}–{last}",
                safe_label(&object.name),
                object.kind.to_ascii_uppercase()
            )
        }
        (Some(object), None) => format!(
            "{} · {}",
            safe_label(&object.name),
            object.kind.to_ascii_uppercase()
        ),
        (None, _) => "ROWS".to_owned(),
    };
    if browser
        .page
        .as_ref()
        .is_some_and(|page| page.columns_truncated)
    {
        title.push_str(" · 128+ columns");
    }
    frame.render_widget(
        Paragraph::new(truncate_width(&title, usize::from(parts[0].width))).style(
            Style::default()
                .fg(if focused {
                    palette().accent
                } else {
                    palette().muted
                })
                .add_modifier(Modifier::BOLD),
        ),
        parts[0],
    );
    app.regions.sqlite_rows = Some(parts[1]);
    if let Some(target) = app.changes.sqlite_rows_target() {
        app.regions
            .register_hit_target(HitTarget::Changes(target), parts[1]);
    }
    if browser.page_loading {
        frame.render_widget(
            Paragraph::new("Loading rows…").style(Style::default().fg(palette().faint)),
            parts[1],
        );
        return;
    }
    if let Some(error) = &browser.page_error {
        frame.render_widget(
            Paragraph::new(format!("Rows unavailable: {error}"))
                .style(Style::default().fg(palette().red)),
            parts[1],
        );
        return;
    }
    let Some(page) = &browser.page else {
        frame.render_widget(
            Paragraph::new("Select an object").style(Style::default().fg(palette().faint)),
            parts[1],
        );
        return;
    };
    if page.columns.is_empty() {
        frame.render_widget(
            Paragraph::new("No visible columns").style(Style::default().fg(palette().faint)),
            parts[1],
        );
        return;
    }
    if page.rows.is_empty() {
        let columns = page
            .columns
            .iter()
            .skip(browser.column_scroll)
            .map(|column| safe_label(&column.name))
            .collect::<Vec<_>>()
            .join("  │  ");
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(columns, Style::default().fg(palette().cyan)),
                Line::styled("Table has no rows", Style::default().fg(palette().faint)),
            ]),
            parts[1],
        );
        return;
    }

    let data_height = parts[1].height.saturating_sub(1);
    let data_area = Rect::new(
        parts[1].x,
        parts[1].y.saturating_add(1),
        parts[1].width,
        data_height,
    );
    app.regions.sqlite_rows = Some(data_area);
    let visible_columns = visible_columns(page, browser.column_scroll, parts[1].width);
    let widths = std::iter::once(Constraint::Length(5))
        .chain(
            visible_columns
                .iter()
                .map(|(_, width)| Constraint::Length(*width)),
        )
        .collect::<Vec<_>>();
    let header = Row::new(
        std::iter::once(Cell::from("#".to_owned()))
            .chain(visible_columns.iter().map(|(index, width)| {
                let column = &page.columns[*index];
                let label = column_label(&column.name, &column.data_type);
                Cell::from(truncate_width(&label, usize::from(*width)))
            }))
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(palette().cyan)
            .add_modifier(Modifier::BOLD),
    );
    let start = browser
        .row_scroll
        .min(page.rows.len().saturating_sub(usize::from(data_height)));
    let rows = page
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(usize::from(data_height))
        .map(|(index, values)| {
            let cells = std::iter::once(Cell::from(
                page.key
                    .offset
                    .saturating_add(index)
                    .saturating_add(1)
                    .to_string(),
            ))
            .chain(visible_columns.iter().map(|(column, width)| {
                Cell::from(truncate_width(
                    values.get(*column).map_or("", String::as_str),
                    usize::from(*width),
                ))
            }))
            .collect::<Vec<_>>();
            let selected = browser.selected_row == Some(index);
            Row::new(cells).style(if selected {
                Style::default().fg(palette().ink).bg(if focused {
                    palette().selected
                } else {
                    palette().inactive_selected
                })
            } else {
                Style::default().fg(palette().soft)
            })
        })
        .collect::<Vec<_>>();
    for visible in 0..rows.len() {
        let index = start + visible;
        if let Some(target) = app.changes.sqlite_row_target(index) {
            app.regions.register_hit_target(
                HitTarget::Changes(target),
                Rect::new(
                    data_area.x,
                    data_area.y.saturating_add(visible as u16),
                    data_area.width,
                    1,
                ),
            );
        }
    }
    frame.render_widget(
        Table::new(rows, widths).header(header).column_spacing(1),
        parts[1],
    );
}

fn visible_columns(page: &SqlitePage, start: usize, width: u16) -> Vec<(usize, u16)> {
    let mut remaining = width.saturating_sub(6);
    let mut columns = Vec::new();
    for (index, column) in page.columns.iter().enumerate().skip(start) {
        if remaining < 4 {
            break;
        }
        let label = column_label(&column.name, &column.data_type);
        let natural =
            page.rows
                .iter()
                .fold(UnicodeWidthStr::width(label.as_str()), |maximum, row| {
                    maximum.max(
                        row.get(index)
                            .map_or(0, |value| UnicodeWidthStr::width(value.as_str())),
                    )
                });
        let column_width = u16::try_from(natural.clamp(6, 24))
            .unwrap_or(24)
            .min(remaining);
        columns.push((index, column_width));
        remaining = remaining.saturating_sub(column_width.saturating_add(1));
    }
    columns
}

fn column_label(name: &str, data_type: &str) -> String {
    let name = safe_label(name);
    let data_type = safe_label(data_type);
    if data_type.is_empty() {
        name
    } else {
        format!("{name} · {data_type}")
    }
}

fn safe_label(value: &str) -> String {
    let mut safe = String::new();
    for character in value.chars() {
        match character {
            '\n' => safe.push_str("\\n"),
            '\r' => safe.push_str("\\r"),
            '\t' => safe.push_str("\\t"),
            character if character.is_control() => safe.extend(character.escape_default()),
            character => safe.push(character),
        }
    }
    safe
}

fn display_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::safe_label;

    #[test]
    fn escapes_terminal_controls_in_schema_labels() {
        assert_eq!(safe_label("bad\x1b]name\n"), "bad\\u{1b}]name\\n");
    }
}
