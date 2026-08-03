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
        vec![truncate_width(
            &summary
                .files
                .iter()
                .map(RepoPath::display)
                .collect::<Vec<_>>()
                .join("  "),
            available,
        )]
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
    let rows = summary.map_or(1, |summary| {
        wrapped_file_summary(&summary.files, file_width, usize::MAX)
            .len()
            .max(1)
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
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0usize;
    for file in files {
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
            lines.push(std::mem::take(&mut line));
        }
        let mut remaining = file.as_str();
        while UnicodeWidthStr::width(remaining) > width {
            let split = remaining
                .char_indices()
                .take_while(|(index, character)| {
                    UnicodeWidthStr::width(&remaining[..index + character.len_utf8()]) <= width
                })
                .map(|(index, character)| index + character.len_utf8())
                .last()
                .unwrap_or_else(|| remaining.chars().next().map_or(0, char::len_utf8));
            if split == 0 {
                break;
            }
            lines.push(remaining[..split].to_owned());
            remaining = &remaining[split..];
        }
        line.push_str(remaining);
        line_width = UnicodeWidthStr::width(remaining);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    let truncated = lines.len() > maximum_lines;
    lines.truncate(maximum_lines);
    if truncated && let Some(last) = lines.last_mut() {
        *last = format!("{}…", truncate_width(last, width.saturating_sub(1)));
    }
    lines
}
