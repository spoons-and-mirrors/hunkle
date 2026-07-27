use super::palette;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

mod markdown;
mod syntax;
pub(super) use markdown::{markdown_prefix_style, styled_markdown};
use syntax::{Language, syntax_spans_for_language};

pub(super) fn styled_source(source: &str, path: &str, width: usize) -> Vec<Line<'static>> {
    styled_source_window(source, path, width, 0, usize::MAX)
}

pub(super) fn styled_source_window(
    source: &str,
    path: &str,
    width: usize,
    start: usize,
    count: usize,
) -> Vec<Line<'static>> {
    let numbered = width >= 72;
    let language = Language::from_path(path);
    source
        .lines()
        .enumerate()
        .skip(start)
        .take(count)
        .map(|(index, line)| {
            let mut spans = if numbered {
                vec![Span::styled(
                    format!("{:>5}  ", index + 1),
                    Style::default().fg(palette().faint),
                )]
            } else {
                Vec::new()
            };
            for span in syntax_spans_for_language(line, language) {
                push_merged_span(&mut spans, span);
            }
            finish_line(spans, width, palette().panel)
        })
        .collect()
}

pub(super) fn styled_diff(
    diff: &str,
    path: &str,
    width: usize,
    show_initial_header: bool,
) -> Vec<Line<'static>> {
    styled_diff_window(diff, path, width, 0, usize::MAX, show_initial_header)
}

pub(super) fn diff_display_line_count(diff: &str, show_initial_header: bool) -> usize {
    let has_hunks = diff.lines().any(|line| line.starts_with("@@"));
    let mut in_hunk = false;
    let mut seen_header = false;
    let mut count = 0;
    for line in diff.lines() {
        let file_header = line.starts_with("diff --git");
        if file_header {
            in_hunk = false;
            if show_initial_header {
                count += usize::from(seen_header);
                count += 1;
                seen_header = true;
                continue;
            }
        }
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header {
            continue;
        }
        if hunk_header {
            count += usize::from(in_hunk);
            in_hunk = true;
        }
        count += 1;
    }
    count
}

pub(super) fn wrapped_preview_line_starts(
    content: &str,
    is_diff: bool,
    width: usize,
    show_initial_diff_header: bool,
) -> Vec<usize> {
    let width = width.max(1);
    let numbered = width >= 72;
    let has_hunks = is_diff && content.lines().any(|line| line.starts_with("@@"));
    let mut in_hunk = false;
    let mut seen_header = false;
    let mut starts = vec![0_usize];
    for line in content.lines() {
        let file_header = is_diff && line.starts_with("diff --git");
        if file_header {
            in_hunk = false;
            if show_initial_diff_header {
                if seen_header {
                    starts.push(starts.last().copied().unwrap_or(0).saturating_add(1));
                }
                seen_header = true;
            } else if has_hunks {
                continue;
            }
        }
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header && !file_header {
            continue;
        }
        if hunk_header {
            if in_hunk {
                starts.push(starts.last().copied().unwrap_or(0).saturating_add(1));
            }
            in_hunk = true;
        }
        let prefix = if !is_diff {
            usize::from(numbered) * 7
        } else if in_hunk
            && !hunk_header
            && !line.starts_with("+++")
            && !line.starts_with("---")
            && (line.starts_with('+') || line.starts_with('-') || line.starts_with(' '))
        {
            usize::from(numbered) * 5 + 1
        } else {
            0
        };
        let payload = if is_diff && prefix > 0 {
            &line[1..]
        } else {
            line
        };
        let content_width_available = width.saturating_sub(prefix).max(1);
        let line_height = word_wrapped_height(payload, content_width_available);
        starts.push(
            starts
                .last()
                .copied()
                .unwrap_or(0)
                .saturating_add(line_height),
        );
    }
    starts
}

pub(super) fn word_wrapped_height(content: &str, width: usize) -> usize {
    let width = width.max(1);
    let mut rows = 1_usize;
    let mut row_width = 0_usize;
    let mut has_word = false;
    let mut pending_whitespace: Option<&str> = None;
    let mut start = 0;
    while let Some((whitespace, end)) = next_wrap_token(content, start) {
        let token = &content[start..end];
        start = end;
        if whitespace && has_word {
            pending_whitespace = Some(token);
            continue;
        }
        let token_width = UnicodeWidthStr::width(token);
        let whitespace_width = pending_whitespace.map_or(0, UnicodeWidthStr::width);
        if !whitespace
            && has_word
            && token_width <= width
            && row_width
                .saturating_add(whitespace_width)
                .saturating_add(token_width)
                > width
        {
            rows = rows.saturating_add(1);
            row_width = 0;
            pending_whitespace = None;
        } else if let Some(spaces) = pending_whitespace.take() {
            add_wrapped_content(spaces, width, &mut rows, &mut row_width);
        }
        add_wrapped_content(token, width, &mut rows, &mut row_width);
        has_word |= !whitespace;
    }
    rows
}

fn next_wrap_token(content: &str, start: usize) -> Option<(bool, usize)> {
    let mut graphemes = content[start..].grapheme_indices(true);
    let (_, first) = graphemes.next()?;
    let whitespace = first.chars().all(char::is_whitespace);
    let end = graphemes
        .find_map(|(offset, grapheme)| {
            (grapheme.chars().all(char::is_whitespace) != whitespace).then_some(start + offset)
        })
        .unwrap_or(content.len());
    Some((whitespace, end))
}

fn add_wrapped_content(content: &str, width: usize, rows: &mut usize, row_width: &mut usize) {
    for grapheme in content.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if *row_width > 0 && row_width.saturating_add(grapheme_width) > width {
            *rows = rows.saturating_add(1);
            *row_width = 0;
        }
        *row_width = row_width.saturating_add(grapheme_width);
    }
}

fn push_merged_span(spans: &mut Vec<Span<'static>>, span: Span<'_>) {
    if let Some(previous) = spans.last_mut()
        && previous.style == span.style
    {
        previous.content.to_mut().push_str(&span.content);
    } else {
        spans.push(Span::styled(span.content.into_owned(), span.style));
    }
}

fn owned_syntax_spans(code: &str, language: Language) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for span in syntax_spans_for_language(code, language) {
        push_merged_span(&mut spans, span);
    }
    spans
}

pub(super) fn styled_diff_window(
    diff: &str,
    path: &str,
    width: usize,
    start: usize,
    count: usize,
    show_initial_header: bool,
) -> Vec<Line<'static>> {
    let numbered = width >= 72;
    let mut old_line = None;
    let mut new_line = None;
    let end = start.saturating_add(count);
    let mut display_index = 0;
    let mut lines = Vec::new();
    let has_hunks = diff.lines().any(|line| line.starts_with("@@"));
    let mut in_hunk = false;
    let mut seen_header = false;
    let language = Language::from_path(path);

    for line in diff.lines() {
        let file_header = line.starts_with("diff --git");
        if file_header {
            in_hunk = false;
            old_line = None;
            new_line = None;
            if show_initial_header {
                if seen_header {
                    if display_index >= start && display_index < end {
                        lines.push(finish_line(Vec::new(), width, palette().panel));
                    }
                    display_index += 1;
                }
                seen_header = true;
            } else if has_hunks {
                continue;
            }
        }
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header && !file_header {
            continue;
        }
        if hunk_header {
            if in_hunk {
                if display_index >= start && display_index < end {
                    lines.push(finish_line(Vec::new(), width, palette().panel));
                }
                display_index += 1;
            }
            in_hunk = true;
        }
        if display_index >= end {
            break;
        }
        if display_index >= start {
            lines.push(styled_diff_line(
                line,
                language,
                width,
                numbered,
                &mut old_line,
                &mut new_line,
            ));
        } else {
            advance_diff_line(line, &mut old_line, &mut new_line);
        }
        display_index += 1;
    }
    lines
}

fn styled_diff_line(
    line: &str,
    language: Language,
    width: usize,
    numbered: bool,
    old_line: &mut Option<u32>,
    new_line: &mut Option<u32>,
) -> Line<'static> {
    if line.starts_with("@@") {
        if let Some((old, new)) = parse_hunk_lines(line) {
            *old_line = Some(old);
            *new_line = Some(new);
        }
        return finish_line(
            vec![Span::styled(
                line.to_owned(),
                Style::default()
                    .fg(palette().cyan)
                    .add_modifier(Modifier::BOLD),
            )],
            width,
            palette().surface_alt,
        );
    }
    if line.starts_with("diff --git") {
        return finish_line(
            vec![Span::styled(
                line.to_owned(),
                Style::default()
                    .fg(palette().accent)
                    .add_modifier(Modifier::BOLD),
            )],
            width,
            palette().panel,
        );
    }
    if line.starts_with("index ") {
        return finish_line(
            vec![Span::styled(
                line.to_owned(),
                Style::default().fg(palette().faint),
            )],
            width,
            palette().panel,
        );
    }
    if line.starts_with("---") || line.starts_with("+++") {
        let color = if line.starts_with("---") {
            palette().red
        } else {
            palette().green
        };
        return finish_line(
            vec![Span::styled(line.to_owned(), Style::default().fg(color))],
            width,
            palette().panel,
        );
    }
    if line.starts_with("\\ No newline") {
        return finish_line(
            vec![Span::styled(
                line.to_owned(),
                Style::default().fg(palette().yellow),
            )],
            width,
            palette().panel,
        );
    }
    if line.starts_with("Untracked file:") || line.starts_with("Binary untracked file") {
        return finish_line(
            vec![Span::styled(
                line.to_owned(),
                Style::default()
                    .fg(palette().yellow)
                    .add_modifier(Modifier::BOLD),
            )],
            width,
            palette().panel,
        );
    }

    let (marker, payload, background, new_number) = if let Some(payload) = line.strip_prefix('+') {
        let number = *new_line;
        *new_line = new_line.map(|value| value + 1);
        ("+", payload, palette().add_bg, number)
    } else if let Some(payload) = line.strip_prefix('-') {
        *old_line = old_line.map(|value| value + 1);
        ("-", payload, palette().remove_bg, None)
    } else if let Some(payload) = line.strip_prefix(' ')
        && old_line.is_some()
    {
        let new = *new_line;
        *old_line = old_line.map(|value| value + 1);
        *new_line = new_line.map(|value| value + 1);
        (" ", payload, palette().panel, new)
    } else {
        return finish_line(owned_syntax_spans(line, language), width, palette().panel);
    };

    let mut spans = if numbered {
        line_number(new_number)
    } else {
        Vec::new()
    };
    spans.push(Span::styled(
        marker.to_owned(),
        Style::default()
            .fg(if marker == "+" {
                palette().green
            } else if marker == "-" {
                palette().red
            } else {
                palette().faint
            })
            .add_modifier(Modifier::BOLD),
    ));
    for span in syntax_spans_for_language(payload, language) {
        push_merged_span(&mut spans, span);
    }
    finish_line(spans, width, background)
}

fn advance_diff_line(line: &str, old_line: &mut Option<u32>, new_line: &mut Option<u32>) {
    if line.starts_with("@@") {
        if let Some((old, new)) = parse_hunk_lines(line) {
            *old_line = Some(old);
            *new_line = Some(new);
        }
    } else if line.starts_with("+++") || line.starts_with("---") {
    } else if line.starts_with('+') {
        *new_line = new_line.map(|value| value + 1);
    } else if line.starts_with('-') {
        *old_line = old_line.map(|value| value + 1);
    } else if line.starts_with(' ') && old_line.is_some() {
        *old_line = old_line.map(|value| value + 1);
        *new_line = new_line.map(|value| value + 1);
    }
}

fn parse_hunk_lines(line: &str) -> Option<(u32, u32)> {
    let mut fields = line.split_whitespace();
    fields.next()?;
    let old = fields
        .next()?
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = fields
        .next()?
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    Some((old, new))
}

fn line_number(new: Option<u32>) -> Vec<Span<'static>> {
    vec![Span::styled(
        format!(
            "{:>4} ",
            new.map_or_else(String::new, |value| value.to_string())
        ),
        Style::default().fg(palette().faint),
    )]
}

fn finish_line(spans: Vec<Span<'static>>, _width: usize, background: Color) -> Line<'static> {
    Line::from(spans).style(Style::default().bg(background))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_source_diff_with_numbers_and_tinted_changes() {
        let lines = styled_diff(
            concat!(
                "diff --git a/src/main.rs b/src/main.rs\n",
                "index 1234567..abcdef0 100644\n",
                "--- a/src/main.rs\n",
                "+++ b/src/main.rs\n",
                "@@ -1 +1 @@\n",
                "-let old_value = 1;\n",
                "+let new_value = 2;",
            ),
            "src/main.rs",
            100,
            false,
        );

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].style.bg, Some(palette().surface_alt));
        assert_eq!(lines[1].style.bg, Some(palette().remove_bg));
        assert_eq!(lines[2].style.bg, Some(palette().add_bg));
        assert!(lines[1].spans[0].content.trim().is_empty());
        assert_eq!(lines[2].spans[0].content.trim(), "1");
        assert!(
            lines[2]
                .spans
                .iter()
                .any(|span| span.content == "let" && span.style.fg == Some(palette().purple))
        );
    }

    #[test]
    fn keeps_the_initial_file_header_for_commit_diffs() {
        let diff = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "index 1234567..abcdef0 100644\n",
            "--- a/src/main.rs\n",
            "+++ b/src/main.rs\n",
            "@@ -1 +1 @@\n",
            "-let old_value = 1;\n",
            "+let new_value = 2;",
        );
        let lines = styled_diff(diff, "", 100, true);

        assert_eq!(lines.len(), 4);
        assert!(lines[0].spans[0].content.starts_with("diff --git"));
        assert_eq!(diff_display_line_count(diff, true), lines.len());
        assert_eq!(
            wrapped_preview_line_starts(diff, true, 100, true).len(),
            lines.len() + 1
        );
    }

    #[test]
    fn separates_commit_files_without_git_metadata() {
        let diff = concat!(
            "diff --git a/first.rs b/first.rs\n",
            "index 1111111..2222222 100644\n",
            "--- a/first.rs\n",
            "+++ b/first.rs\n",
            "@@ -1 +1 @@\n",
            " context\n",
            "diff --git a/second.rs b/second.rs\n",
            "index 3333333..4444444 100644\n",
            "--- a/second.rs\n",
            "+++ b/second.rs\n",
            "\n",
            "@@ -2 +2 @@\n",
            "+change",
        );
        let lines = styled_diff(diff, "", 100, true);
        let text = |index: usize| {
            lines[index]
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };

        assert_eq!(lines.len(), 7);
        assert_eq!(text(0), "diff --git a/first.rs b/first.rs");
        assert!(lines[3].spans.is_empty());
        assert_eq!(text(4), "diff --git a/second.rs b/second.rs");
        assert!(text(5).starts_with("@@"));
        assert!(lines.iter().all(|line| {
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            !text.starts_with("index ") && !text.starts_with("--- ") && !text.starts_with("+++ ")
        }));
        assert_eq!(diff_display_line_count(diff, true), lines.len());
        assert_eq!(
            wrapped_preview_line_starts(diff, true, 100, true).len(),
            lines.len() + 1
        );
    }

    #[test]
    fn wrapped_line_index_matches_styled_diff_heights() {
        let diff = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "@@ -1 +1 @@\n",
            "+a line that wraps\n",
            "@@ -3 +3 @@\n",
            " context\n",
            "diff --git a/very-long-old-name.rs b/very-long-new-name.rs\n",
            "--- a/very-long-old-name.rs\n",
            "+++ b/very-long-new-name.rs\n",
            "@@ -1 +1 @@\n",
            "+emoji 👩‍💻 line",
        );
        let width = 10;
        let lines = styled_diff(diff, "src/main.rs", width, false);
        let starts = wrapped_preview_line_starts(diff, true, width, false);

        assert_eq!(starts.len(), lines.len() + 1);
        for (index, line) in lines.iter().enumerate() {
            let display_width = line
                .spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            let gutter = usize::from(
                line.spans
                    .first()
                    .is_some_and(|span| matches!(span.content.as_ref(), "+" | "-" | " ")),
            );
            let content = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let payload = if gutter > 0 { &content[1..] } else { &content };
            assert_eq!(
                starts[index + 1] - starts[index],
                word_wrapped_height(payload, width.saturating_sub(gutter).max(1)),
                "styled width was {display_width}",
            );
        }
        assert_eq!(word_wrapped_height("word committing", 11), 2);
    }
}
