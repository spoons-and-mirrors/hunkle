use super::*;

pub(super) fn diff_summary_text(
    summary: Option<&DiffSummary>,
    unavailable: bool,
    wrapped: bool,
    width: u16,
    height: u16,
    include_changes: bool,
) -> Text<'static> {
    let Some(summary) = summary else {
        let state = if unavailable {
            "unavailable"
        } else {
            "loading…"
        };
        let mut lines = Vec::new();
        if include_changes {
            lines.push(Line::from(vec![
                Span::styled(
                    "CHANGES  ",
                    Style::default()
                        .fg(palette().muted)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(state, Style::default().fg(palette().faint)),
            ]));
        }
        lines.push(Line::styled(
            format!("FILES  {state}"),
            Style::default().fg(palette().faint),
        ));
        return Text::from(lines);
    };

    let file_count = summary.files.len();
    let displayed_file_count = format!(
        "{}{}",
        file_count,
        if summary.files_truncated { "+" } else { "" }
    );
    let mut lines = Vec::new();
    if include_changes {
        lines.push(Line::from(vec![
            Span::styled(
                "CHANGES  ",
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("+{}", summary.additions),
                Style::default().fg(palette().green),
            ),
            Span::raw("  "),
            Span::styled(
                format!("-{}", summary.deletions),
                Style::default().fg(palette().red),
            ),
            Span::styled(
                format!(
                    "  {displayed_file_count} {}",
                    if file_count == 1 { "file" } else { "files" }
                ),
                Style::default().fg(palette().faint),
            ),
        ]));
    }
    let label = "FILES  ";
    let available = usize::from(width).saturating_sub(label.len());
    let file_lines = if wrapped {
        wrapped_file_summary(
            &summary.files,
            available,
            usize::from(height.saturating_sub(u16::from(include_changes))),
        )
    } else {
        let mut files = String::new();
        for file in &summary.files {
            if !files.is_empty() {
                files.push_str("  ");
            }
            files.push_str(&file.display());
            if UnicodeWidthStr::width(files.as_str()) > available {
                break;
            }
        }
        vec![truncate_width(&files, available)]
    };
    lines.extend(file_lines.into_iter().enumerate().map(|(index, files)| {
        Line::from(vec![
            Span::styled(
                if index == 0 { label } else { "       " },
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(files, Style::default().fg(palette().cyan)),
        ])
    }));
    Text::from(lines)
}

pub(super) fn diff_summary_height(
    summary: Option<&DiffSummary>,
    width: u16,
    wrapped: bool,
    maximum: u16,
) -> u16 {
    if !wrapped {
        return 3.min(maximum);
    }
    let file_width = usize::from(width).saturating_sub("FILES  ".len());
    let maximum_file_rows = usize::from(maximum.saturating_sub(2));
    let rows = summary.map_or(1, |summary| {
        wrapped_file_summary_height(&summary.files, file_width, maximum_file_rows).max(1)
    });
    (rows as u16).saturating_add(2).min(maximum)
}

pub(super) fn wrapped_file_summary(
    files: &[RepoPath],
    width: usize,
    maximum_lines: usize,
) -> Vec<String> {
    if width == 0 || maximum_lines == 0 {
        return Vec::new();
    }
    let mut lines = Vec::with_capacity(maximum_lines.min(files.len()));
    let mut line = String::new();
    let mut line_width = 0usize;
    let mut truncated = false;
    'files: for file in files {
        let file = file.display();
        let file_width = UnicodeWidthStr::width(file.as_str());
        if file_width <= width {
            let separator_width = usize::from(!line.is_empty()) * 2;
            if line_width
                .saturating_add(separator_width)
                .saturating_add(file_width)
                <= width
            {
                if separator_width > 0 {
                    line.push_str("  ");
                }
                line.push_str(&file);
                line_width = line_width
                    .saturating_add(separator_width)
                    .saturating_add(file_width);
                continue;
            }
        }
        if !line.is_empty() {
            if !push_summary_line(&mut lines, std::mem::take(&mut line), maximum_lines) {
                truncated = true;
                break;
            }
        }
        let mut remaining = file.as_str();
        let mut remaining_width = file_width;
        while remaining_width > width {
            let (split, split_width) = width_prefix(remaining, width);
            if split == 0 {
                break;
            }
            if !push_summary_line(&mut lines, remaining[..split].to_owned(), maximum_lines) {
                truncated = true;
                break 'files;
            }
            remaining = &remaining[split..];
            remaining_width = remaining_width.saturating_sub(split_width);
        }
        line.push_str(remaining);
        line_width = remaining_width;
    }
    if !truncated && !line.is_empty() && !push_summary_line(&mut lines, line, maximum_lines) {
        truncated = true;
    }
    if truncated && let Some(last) = lines.last_mut() {
        *last = format!("{}…", truncate_width(last, width.saturating_sub(1)));
    }
    lines
}

fn push_summary_line(lines: &mut Vec<String>, line: String, maximum: usize) -> bool {
    if lines.len() >= maximum {
        return false;
    }
    lines.push(line);
    true
}

fn width_prefix(content: &str, width: usize) -> (usize, usize) {
    let mut end = 0;
    let mut measured = 0usize;
    for (index, character) in content.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if end > 0 && measured.saturating_add(character_width) > width {
            break;
        }
        end = index + character.len_utf8();
        measured = measured.saturating_add(character_width);
        if measured >= width {
            break;
        }
    }
    (end, measured)
}

fn wrapped_file_summary_height(files: &[RepoPath], width: usize, maximum: usize) -> usize {
    if width == 0 || maximum == 0 {
        return 0;
    }
    let mut rows = 0usize;
    let mut line_width = 0usize;
    for file in files {
        let file = file.display();
        let file_width = UnicodeWidthStr::width(file.as_str());
        let separator_width = usize::from(line_width > 0) * 2;
        if file_width <= width
            && line_width
                .saturating_add(separator_width)
                .saturating_add(file_width)
                <= width
        {
            line_width = line_width
                .saturating_add(separator_width)
                .saturating_add(file_width);
            continue;
        }
        if line_width > 0 {
            rows = rows.saturating_add(1);
            if rows >= maximum {
                return maximum;
            }
        }
        let mut segment_width = 0usize;
        for character in file.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if segment_width > 0 && segment_width.saturating_add(character_width) > width {
                rows = rows.saturating_add(1);
                if rows >= maximum {
                    return maximum;
                }
                segment_width = 0;
            }
            segment_width = segment_width.saturating_add(character_width);
        }
        line_width = segment_width;
    }
    rows.saturating_add(usize::from(line_width > 0))
        .min(maximum)
}
