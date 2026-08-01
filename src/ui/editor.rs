use std::collections::BTreeSet;

use ratatui::buffer::Buffer;

use crate::repo_path::RepoPath;

use super::*;

pub(super) fn draw_file_editor(frame: &mut Frame<'_>, app: &mut App) {
    let Some(panel) = app.regions.diff else {
        return;
    };
    let header = Rect::new(
        panel.x.saturating_add(1),
        panel.y.saturating_add(1),
        panel.width.saturating_sub(2),
        1,
    );
    let body = Rect::new(
        header.x,
        header.y.saturating_add(2),
        header.width,
        panel.bottom().saturating_sub(header.y.saturating_add(3)),
    );
    const LINE_NUMBER_WIDTH: u16 = 7;
    let gutter_width = LINE_NUMBER_WIDTH.min(body.width);
    let gutter = Rect::new(body.x, body.y, gutter_width, body.height);
    let editor_body = Rect::new(
        body.x.saturating_add(gutter_width),
        body.y,
        body.width.saturating_sub(gutter_width),
        body.height,
    );
    app.regions.preview_body = Some(editor_body);
    app.regions.diff_scrollbar = None;
    app.regions.diff_scroll_thumb = None;
    app.regions.diff_scroll_max = 0;
    app.regions.editor_rows.clear();
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Block::default().style(Style::default().bg(palette().panel).fg(palette().ink)),
        panel,
    );

    let save_label = app.settings.shortcuts.label(ShortcutAction::SaveOrFormat);
    let wrapped = app.changes.diff_wrap;
    let formatting = app.format_running();
    let Some(editor) = &mut app.file_editor else {
        return;
    };
    let line_markers = editor_line_markers(
        &app.changes.diff,
        editor.path(),
        editor.locally_changed_lines(),
        editor.visible_line_count(),
    );
    let (cursor_line, cursor_column) = editor.cursor_position();
    let path = editor.path().display();
    let dirty = if formatting {
        "formatting"
    } else if editor.dirty() {
        "modified"
    } else {
        "saved"
    };
    let title = format!(
        "EDIT  {path}  {dirty}  Ln {}, Col {}  {save_label} save  ctrl+enter save + close  esc close",
        cursor_line.saturating_add(1),
        cursor_column.saturating_add(1)
    );
    frame.render_widget(
        Paragraph::new(truncate_width(&title, usize::from(header.width))).style(
            Style::default()
                .fg(palette().accent)
                .add_modifier(Modifier::BOLD),
        ),
        header,
    );

    let viewport_height = usize::from(editor_body.height);
    let viewport_width = usize::from(editor_body.width).max(1);
    let (lines, line_numbers, cursor_row, rendered_cursor_column, cursor_visible) = if wrapped {
        let (cursor_row, rendered_cursor_column) =
            wrapped_editor_cursor(editor.text(), viewport_width, cursor_line, cursor_column);
        if let Some(anchor) = app.file_editor_anchor.take() {
            let row = usize::from(anchor.y.saturating_sub(editor_body.y))
                .min(viewport_height.saturating_sub(1));
            editor.anchor_wrapped_cursor_at(cursor_row, row);
        }
        if editor.should_follow_cursor() {
            editor.ensure_wrapped_cursor_visible(cursor_row, viewport_height);
        }
        let (lines, line_numbers, rows) = wrapped_editor_view(
            editor.text(),
            &path,
            viewport_width,
            editor.wrap_scroll_row,
            viewport_height,
            &line_markers,
        );
        app.regions.editor_rows = rows;
        (
            lines,
            line_numbers,
            cursor_row.saturating_sub(editor.wrap_scroll_row),
            rendered_cursor_column,
            cursor_row >= editor.wrap_scroll_row
                && cursor_row < editor.wrap_scroll_row.saturating_add(viewport_height),
        )
    } else {
        if let Some(anchor) = app.file_editor_anchor.take() {
            let row = usize::from(anchor.y.saturating_sub(editor_body.y))
                .min(viewport_height.saturating_sub(1));
            let column = usize::from(anchor.x.saturating_sub(editor_body.x))
                .min(viewport_width.saturating_sub(1));
            editor.anchor_cursor_at(row, column);
        }
        if editor.should_follow_cursor() {
            editor.ensure_cursor_visible(viewport_height, viewport_width);
        }
        let lines = text::styled_source_window(
            editor.text(),
            &path,
            0,
            editor.scroll_line,
            viewport_height,
        );
        let mut lines = editor_visible_lines(lines, editor.scroll_column, viewport_width);
        while lines.len() <= cursor_line.saturating_sub(editor.scroll_line) {
            lines.push(Line::default().style(Style::default().bg(palette().panel)));
        }
        let line_count = editor.visible_line_count();
        let line_numbers = (0..viewport_height)
            .map(|row| {
                let line = editor.scroll_line.saturating_add(row);
                editor_line_number(
                    (line < line_count).then_some(line),
                    line_markers.get(line).copied().flatten(),
                )
            })
            .collect::<Vec<_>>();
        (
            lines,
            line_numbers,
            cursor_line.saturating_sub(editor.scroll_line),
            cursor_column.saturating_sub(editor.scroll_column),
            cursor_line >= editor.scroll_line
                && cursor_line < editor.scroll_line.saturating_add(viewport_height),
        )
    };
    frame.render_widget(Paragraph::new(line_numbers), gutter);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(palette().panel)),
        editor_body,
    );
    render_file_editor_selection(
        frame.buffer_mut(),
        editor.text(),
        editor.selected_range(),
        editor_body,
        wrapped,
        &app.regions.editor_rows,
        editor.scroll_line,
        editor.scroll_column,
    );
    let cursor_x = editor_body
        .x
        .saturating_add(u16::try_from(rendered_cursor_column).unwrap_or(u16::MAX));
    let cursor_y = editor_body
        .y
        .saturating_add(u16::try_from(cursor_row).unwrap_or(u16::MAX));
    if cursor_visible && cursor_x < editor_body.right() && cursor_y < editor_body.bottom() {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn render_file_editor_selection(
    buffer: &mut Buffer,
    source: &str,
    selection: Option<(usize, usize)>,
    body: Rect,
    wrapped: bool,
    rows: &[crate::app::EditorRenderedRow],
    scroll_line: usize,
    scroll_column: usize,
) {
    let Some(selection) = selection else {
        return;
    };
    let selected_style = Style::default().fg(palette().canvas).bg(palette().accent);
    if wrapped {
        for (row_index, row) in rows.iter().enumerate() {
            let Some((start, end)) = selected_display_range(source, row.line, selection) else {
                continue;
            };
            for pair in row.columns.windows(2) {
                let (rendered_start, source_start) = pair[0];
                let (rendered_end, source_end) = pair[1];
                if source_start < end && source_end > start {
                    style_editor_cells(
                        buffer,
                        body,
                        body.y
                            .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
                        rendered_start,
                        rendered_end,
                        selected_style,
                    );
                }
            }
        }
        return;
    }

    for row in 0..usize::from(body.height) {
        let line = scroll_line.saturating_add(row);
        let Some((start, end)) = selected_display_range(source, line, selection) else {
            continue;
        };
        style_editor_cells(
            buffer,
            body,
            body.y
                .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
            start.saturating_sub(scroll_column),
            end.saturating_sub(scroll_column),
            selected_style,
        );
    }
}

fn style_editor_cells(
    buffer: &mut Buffer,
    body: Rect,
    y: u16,
    start: usize,
    end: usize,
    style: Style,
) {
    let left = body
        .x
        .saturating_add(u16::try_from(start).unwrap_or(u16::MAX));
    let right = body
        .x
        .saturating_add(u16::try_from(end).unwrap_or(u16::MAX));
    if y >= body.bottom() || left >= body.right() || right <= left {
        return;
    }
    for x in left..right.min(body.right()) {
        if let Some(cell) = buffer.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
}

pub(super) fn selected_display_range(
    source: &str,
    line_number: usize,
    selection: (usize, usize),
) -> Option<(usize, usize)> {
    let mut line_start = 0usize;
    for (index, raw_line) in source.split('\n').enumerate() {
        let raw_end = line_start.saturating_add(raw_line.len());
        let line_end = raw_end.saturating_add(usize::from(raw_end < source.len()));
        if index == line_number {
            if selection.0 >= line_end || selection.1 <= line_start {
                return None;
            }
            let content = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            let content_end = line_start.saturating_add(content.len());
            let start = selection.0.clamp(line_start, content_end);
            let end = selection.1.clamp(line_start, content_end);
            let start_column = editor_display_width(&source[line_start..start]);
            let mut end_column = editor_display_width(&source[line_start..end]);
            if start == end && selection.1 > content_end {
                end_column = start_column.saturating_add(1);
            }
            return Some((start_column, end_column.max(start_column.saturating_add(1))));
        }
        line_start = line_end;
    }
    None
}

fn editor_display_width(value: &str) -> usize {
    let mut column = 0;
    for grapheme in value.graphemes(true) {
        if grapheme == "\t" {
            column += TAB_WIDTH - (column % TAB_WIDTH);
        } else {
            column += UnicodeWidthStr::width(grapheme);
        }
    }
    column
}

fn editor_line_markers(
    diff: &str,
    path: &RepoPath,
    locally_changed_lines: &BTreeSet<usize>,
    line_count: usize,
) -> Vec<Option<char>> {
    let mut markers = vec![None; line_count];
    for (line, marker) in text::diff_new_line_markers(diff, path) {
        if let Some(slot) = markers.get_mut(line) {
            *slot = Some(marker);
        }
    }
    for line in locally_changed_lines {
        if let Some(slot) = markers.get_mut(*line) {
            *slot = Some('~');
        }
    }
    markers
}

fn editor_line_number(line: Option<usize>, marker: Option<char>) -> Line<'static> {
    line.map_or_else(
        || Line::default().style(Style::default().bg(palette().panel)),
        |line| {
            let number = Span::styled(
                format!("{:>5}", line.saturating_add(1)),
                Style::default().fg(palette().faint).bg(palette().panel),
            );
            let marker = marker.map_or_else(
                || Span::styled(" ", Style::default().bg(palette().panel)),
                |marker| {
                    let color = match marker {
                        '+' => palette().green,
                        '-' => palette().red,
                        _ => palette().yellow,
                    };
                    Span::styled(
                        marker.to_string(),
                        Style::default()
                            .fg(color)
                            .bg(palette().panel)
                            .add_modifier(Modifier::BOLD),
                    )
                },
            );
            Line::from(vec![number, marker, Span::raw(" ")])
        },
    )
}

pub(super) fn wrapped_editor_cursor(
    source: &str,
    width: usize,
    cursor_line: usize,
    cursor_column: usize,
) -> (usize, usize) {
    let mut visual_row = 0usize;
    for (line, content) in editor_source_lines(source).enumerate() {
        let rows = text::word_wrapped_rows(content, width);
        if line == cursor_line {
            let (row, rendered_column) = rows
                .iter()
                .enumerate()
                .min_by_key(|(_, row)| {
                    row.source_column_at(row.rendered_column_at(cursor_column))
                        .abs_diff(cursor_column)
                })
                .map_or((0, 0), |(index, row)| {
                    (index, row.rendered_column_at(cursor_column))
                });
            return (visual_row.saturating_add(row), rendered_column);
        }
        visual_row = visual_row.saturating_add(rows.len());
    }
    (visual_row, 0)
}

fn wrapped_editor_view(
    source: &str,
    path: &str,
    width: usize,
    scroll: usize,
    height: usize,
    line_markers: &[Option<char>],
) -> (
    Vec<Line<'static>>,
    Vec<Line<'static>>,
    Vec<crate::app::EditorRenderedRow>,
) {
    let mut lines = Vec::new();
    let mut line_numbers = Vec::new();
    let mut rendered_rows = Vec::new();
    let mut visual_row = 0usize;
    for (line_number, content) in editor_source_lines(source).enumerate() {
        let rows = text::word_wrapped_rows(content, width);
        let line_end = visual_row.saturating_add(rows.len());
        if line_end > scroll && visual_row < scroll.saturating_add(height) {
            let styled = text::styled_source_window(source, path, 0, line_number, 1)
                .into_iter()
                .next()
                .unwrap_or_default();
            for (index, row) in rows.iter().enumerate() {
                let absolute_row = visual_row.saturating_add(index);
                if absolute_row < scroll || absolute_row >= scroll.saturating_add(height) {
                    continue;
                }
                let rendered =
                    editor_visible_lines(vec![styled.clone()], row.source_start(), row.width())
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                lines.push(rendered);
                line_numbers.push(editor_line_number(
                    (index == 0).then_some(line_number),
                    line_markers.get(line_number).copied().flatten(),
                ));
                rendered_rows.push(crate::app::EditorRenderedRow {
                    line: line_number,
                    columns: row.columns(),
                });
            }
        }
        visual_row = line_end;
        if visual_row >= scroll.saturating_add(height) {
            break;
        }
    }
    while lines.len() < height {
        lines.push(Line::default().style(Style::default().bg(palette().panel)));
        line_numbers.push(editor_line_number(None, None));
    }
    (lines, line_numbers, rendered_rows)
}

fn editor_source_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

fn editor_visible_lines(
    mut lines: Vec<Line<'static>>,
    start: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let end = start.saturating_add(width);
    for line in &mut lines {
        let mut column = 0usize;
        let mut visible_spans = Vec::new();
        for span in std::mem::take(&mut line.spans) {
            let mut visible = String::new();
            for grapheme in span.content.graphemes(true) {
                let grapheme_width = if grapheme == "\t" {
                    TAB_WIDTH - column % TAB_WIDTH
                } else {
                    UnicodeWidthStr::width(grapheme)
                };
                let grapheme_end = column.saturating_add(grapheme_width);
                if grapheme_width == 0 {
                    if column >= start && column < end {
                        visible.push_str(grapheme);
                    }
                } else if column >= start && grapheme_end <= end {
                    if grapheme == "\t" {
                        visible.push_str(&" ".repeat(grapheme_width));
                    } else {
                        visible.push_str(grapheme);
                    }
                } else {
                    let overlap_start = column.max(start);
                    let overlap_end = grapheme_end.min(end);
                    visible.push_str(&" ".repeat(overlap_end.saturating_sub(overlap_start)));
                }
                column = grapheme_end;
                if column >= end {
                    break;
                }
            }
            if !visible.is_empty() {
                visible_spans.push(Span::styled(visible, span.style));
            }
            if column >= end {
                break;
            }
        }
        line.spans = visible_spans;
    }
    lines
}
