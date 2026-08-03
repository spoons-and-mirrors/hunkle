use super::palette;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::repo_path::RepoPath;

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
            push_syntax_spans(&mut spans, line, language);
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
            usize::from(numbered) * 6 + 1
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

#[derive(Clone)]
struct WrappedGrapheme {
    source_start: usize,
    source_end: usize,
    width: usize,
}

pub(super) struct WrappedCursorRow {
    boundaries: Vec<(usize, usize)>,
}

impl WrappedCursorRow {
    fn new(source_column: usize) -> Self {
        Self {
            boundaries: vec![(0, source_column)],
        }
    }

    pub(super) fn width(&self) -> usize {
        self.boundaries.last().map_or(0, |boundary| boundary.0)
    }

    fn push(&mut self, grapheme: &WrappedGrapheme) {
        self.boundaries.push((
            self.width().saturating_add(grapheme.width),
            grapheme.source_end,
        ));
    }

    pub(super) fn source_column_at(&self, column: usize) -> usize {
        self.boundaries
            .iter()
            .min_by_key(|(rendered, _)| rendered.abs_diff(column))
            .map_or(0, |(_, source)| *source)
    }

    pub(super) fn source_start(&self) -> usize {
        self.boundaries.first().map_or(0, |boundary| boundary.1)
    }

    pub(super) fn rendered_column_at(&self, source_column: usize) -> usize {
        self.boundaries
            .iter()
            .min_by_key(|(_, source)| source.abs_diff(source_column))
            .map_or(0, |(rendered, _)| *rendered)
    }

    pub(super) fn columns(&self) -> Vec<(usize, usize)> {
        self.boundaries.clone()
    }
}

pub(super) fn word_wrapped_height(content: &str, width: usize) -> usize {
    wrap_content(content, width, WrappedHeight::default()).rows
}

pub(super) fn word_wrapped_column_at(
    content: &str,
    width: usize,
    row: usize,
    column: usize,
) -> Option<usize> {
    word_wrapped_rows(content, width)
        .get(row)
        .map(|row| row.source_column_at(column))
}

pub(super) fn word_wrapped_rows(content: &str, width: usize) -> Vec<WrappedCursorRow> {
    wrap_content(content, width, CursorRows::default()).finish()
}

#[derive(Clone, Copy)]
struct WrapToken<'a> {
    content: &'a str,
    source_start: usize,
    width: usize,
    whitespace: bool,
}

trait WrapRows {
    fn current_width(&self) -> usize;
    fn break_row(&mut self, source_column: usize);
    fn push(&mut self, grapheme: &WrappedGrapheme);
}

struct CursorRows {
    completed: Vec<WrappedCursorRow>,
    current: WrappedCursorRow,
}

impl Default for CursorRows {
    fn default() -> Self {
        Self {
            completed: Vec::new(),
            current: WrappedCursorRow::new(0),
        }
    }
}

impl CursorRows {
    fn finish(mut self) -> Vec<WrappedCursorRow> {
        self.completed.push(self.current);
        self.completed
    }
}

impl WrapRows for CursorRows {
    fn current_width(&self) -> usize {
        self.current.width()
    }

    fn break_row(&mut self, source_column: usize) {
        let next = WrappedCursorRow::new(source_column);
        self.completed
            .push(std::mem::replace(&mut self.current, next));
    }

    fn push(&mut self, grapheme: &WrappedGrapheme) {
        self.current.push(grapheme);
    }
}

struct WrappedHeight {
    rows: usize,
    current_width: usize,
}

impl Default for WrappedHeight {
    fn default() -> Self {
        Self {
            rows: 1,
            current_width: 0,
        }
    }
}

impl WrapRows for WrappedHeight {
    fn current_width(&self) -> usize {
        self.current_width
    }

    fn break_row(&mut self, _source_column: usize) {
        self.rows = self.rows.saturating_add(1);
        self.current_width = 0;
    }

    fn push(&mut self, grapheme: &WrappedGrapheme) {
        self.current_width = self.current_width.saturating_add(grapheme.width);
    }
}

fn wrap_content<R: WrapRows>(content: &str, width: usize, mut rows: R) -> R {
    let width = width.max(1);
    let mut graphemes = content.grapheme_indices(true).peekable();
    let mut source_column = 0usize;
    let mut has_word = false;
    let mut pending_whitespace = None;
    while let Some(token) = next_wrap_token(content, &mut graphemes, &mut source_column) {
        if token.whitespace && has_word {
            pending_whitespace = Some(token);
            continue;
        }
        let whitespace_width = pending_whitespace.map_or(0, |token: WrapToken<'_>| token.width);
        if !token.whitespace
            && has_word
            && token.width <= width
            && rows
                .current_width()
                .saturating_add(whitespace_width)
                .saturating_add(token.width)
                > width
        {
            rows.break_row(token.source_start);
            pending_whitespace = None;
        } else if let Some(spaces) = pending_whitespace.take() {
            append_wrap_token(spaces, width, &mut rows);
        }
        append_wrap_token(token, width, &mut rows);
        has_word |= !token.whitespace;
    }
    rows
}

fn next_wrap_token<'a>(
    content: &'a str,
    graphemes: &mut std::iter::Peekable<unicode_segmentation::GraphemeIndices<'a>>,
    source_column: &mut usize,
) -> Option<WrapToken<'a>> {
    let (start, first) = graphemes.next()?;
    let whitespace = first.chars().all(char::is_whitespace);
    let source_start = *source_column;
    let first_width = wrap_grapheme_width(first, *source_column);
    *source_column = (*source_column).saturating_add(first_width);
    let mut end = start.saturating_add(first.len());
    while let Some(&(index, grapheme)) = graphemes.peek() {
        if grapheme.chars().all(char::is_whitespace) != whitespace {
            break;
        }
        graphemes.next();
        let grapheme_width = wrap_grapheme_width(grapheme, *source_column);
        *source_column = (*source_column).saturating_add(grapheme_width);
        end = index.saturating_add(grapheme.len());
    }
    Some(WrapToken {
        content: &content[start..end],
        source_start,
        width: (*source_column).saturating_sub(source_start),
        whitespace,
    })
}

fn append_wrap_token(token: WrapToken<'_>, width: usize, rows: &mut impl WrapRows) {
    let mut source_column = token.source_start;
    for content in token.content.graphemes(true) {
        let grapheme_width = wrap_grapheme_width(content, source_column);
        let grapheme = WrappedGrapheme {
            source_start: source_column,
            source_end: source_column.saturating_add(grapheme_width),
            width: grapheme_width,
        };
        if rows.current_width() > 0 && rows.current_width().saturating_add(grapheme.width) > width {
            rows.break_row(grapheme.source_start);
        }
        rows.push(&grapheme);
        source_column = grapheme.source_end;
    }
}

fn wrap_grapheme_width(grapheme: &str, source_column: usize) -> usize {
    if grapheme == "\t" {
        crate::app::TAB_WIDTH - source_column % crate::app::TAB_WIDTH
    } else {
        UnicodeWidthStr::width(grapheme)
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
    push_syntax_spans(&mut spans, code, language);
    spans
}

fn push_syntax_spans(spans: &mut Vec<Span<'static>>, code: &str, language: Language) {
    let mut column = 0usize;
    for span in syntax_spans_for_language(code, language) {
        if span.content.contains('\t') {
            let mut expanded = String::with_capacity(span.content.len());
            for grapheme in span.content.graphemes(true) {
                if grapheme != "\t" {
                    expanded.push_str(grapheme);
                    column = column.saturating_add(UnicodeWidthStr::width(grapheme));
                    continue;
                }
                let width = crate::app::TAB_WIDTH - column % crate::app::TAB_WIDTH;
                expanded.extend(std::iter::repeat_n(' ', width));
                column = column.saturating_add(width);
            }
            push_merged_span(spans, Span::styled(expanded, span.style));
        } else {
            column = column.saturating_add(UnicodeWidthStr::width(span.content.as_ref()));
            push_merged_span(spans, span);
        }
    }
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

    let (marker, payload, background, new_number) = if new_line.is_some()
        && let Some(payload) = line.strip_prefix('+')
    {
        let number = *new_line;
        *new_line = new_line.map(|value| value + 1);
        ("+", payload, palette().add_bg, number)
    } else if old_line.is_some()
        && let Some(payload) = line.strip_prefix('-')
    {
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
    push_syntax_spans(&mut spans, payload, language);
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

pub(super) fn diff_new_line_and_payload_at_display_row(
    diff: &str,
    target: usize,
    show_initial_header: bool,
) -> Option<(usize, &str)> {
    let has_hunks = diff.lines().any(|line| line.starts_with("@@"));
    let mut in_hunk = false;
    let mut seen_header = false;
    let mut display_index = 0;
    let mut old_line = None;
    let mut new_line = None;

    for line in diff.lines() {
        let file_header = line.starts_with("diff --git");
        if file_header {
            in_hunk = false;
            old_line = None;
            new_line = None;
            if show_initial_header {
                if seen_header {
                    if display_index == target {
                        return None;
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
                if display_index == target {
                    return None;
                }
                display_index += 1;
            }
            in_hunk = true;
            if let Some((_, new)) = parse_hunk_lines(line) {
                new_line = Some(new);
            }
        }
        if display_index == target {
            return if in_hunk && (line.starts_with('+') || line.starts_with(' ')) {
                new_line.map(|number| (number.max(1) as usize, &line[1..]))
            } else {
                None
            };
        }
        advance_diff_line(line, &mut old_line, &mut new_line);
        display_index += 1;
    }
    None
}

pub(super) fn diff_file_position_at_display_row(
    diff: &str,
    target: usize,
    show_initial_header: bool,
) -> Option<(RepoPath, usize, &str)> {
    if !show_initial_header {
        return None;
    }
    let lines = diff.lines().collect::<Vec<_>>();
    let has_hunks = lines.iter().any(|line| line.starts_with("@@"));
    let mut display_index = 0;
    let mut in_hunk = false;
    let mut seen_header = false;
    let mut path = None;
    let mut old_line = None;
    let mut new_line = None;

    for (index, line) in lines.iter().copied().enumerate() {
        let file_header = line.starts_with("diff --git");
        if file_header {
            in_hunk = false;
            old_line = None;
            new_line = None;
            path = diff_file_destination(&lines, index).map(|(path, _)| path);
            if seen_header {
                if display_index == target {
                    return None;
                }
                display_index += 1;
            }
            seen_header = true;
        }
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header && !file_header {
            continue;
        }
        if hunk_header {
            if in_hunk {
                if display_index == target {
                    return None;
                }
                display_index += 1;
            }
            in_hunk = true;
            if let Some((old, new)) = parse_hunk_lines(line) {
                old_line = Some(old);
                new_line = Some(new);
            }
        }
        if display_index == target {
            return if in_hunk && (line.starts_with('+') || line.starts_with(' ')) {
                Some((path.clone()?, new_line?.max(1) as usize, &line[1..]))
            } else {
                None
            };
        }
        advance_diff_line(line, &mut old_line, &mut new_line);
        display_index += 1;
    }
    None
}

pub(super) fn diff_file_header_at_display_row(
    diff: &str,
    target: usize,
    show_initial_header: bool,
) -> Option<(RepoPath, usize)> {
    if !show_initial_header {
        return None;
    }
    let lines = diff.lines().collect::<Vec<_>>();
    let has_hunks = lines.iter().any(|line| line.starts_with("@@"));
    let mut display_index = 0;
    let mut in_hunk = false;
    let mut seen_header = false;

    for (index, line) in lines.iter().copied().enumerate() {
        let file_header = line.starts_with("diff --git");
        if file_header {
            in_hunk = false;
            if seen_header {
                if display_index == target {
                    return None;
                }
                display_index += 1;
            }
            seen_header = true;
        }
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header && !file_header {
            continue;
        }
        if hunk_header {
            if in_hunk {
                if display_index == target {
                    return None;
                }
                display_index += 1;
            }
            in_hunk = true;
        }
        if display_index == target {
            return file_header
                .then(|| diff_file_destination(&lines, index))
                .flatten();
        }
        display_index += 1;
    }
    None
}

pub(super) fn diff_new_line_markers(diff: &str, target: &RepoPath) -> Vec<(usize, char)> {
    let lines = diff.lines().collect::<Vec<_>>();
    let has_hunks = lines.iter().any(|line| line.starts_with("@@"));
    if !has_hunks {
        return Vec::new();
    }
    let has_file_headers = lines.iter().any(|line| line.starts_with("diff --git"));
    let mut path = (!has_file_headers).then(|| target.clone());
    let mut in_hunk = false;
    let mut new_line = None;
    let mut deletion_pending = false;
    let mut markers = Vec::new();

    for (index, line) in lines.iter().copied().enumerate() {
        let file_header = line.starts_with("diff --git");
        if file_header {
            path = diff_file_destination(&lines, index).map(|(path, _)| path);
            in_hunk = false;
            new_line = None;
            deletion_pending = false;
        }
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header && !file_header {
            continue;
        }
        if hunk_header {
            in_hunk = true;
            if let Some((_, new)) = parse_hunk_lines(line) {
                new_line = Some(new);
            }
            deletion_pending = false;
            continue;
        }
        if !in_hunk || path.as_ref() != Some(target) {
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            if let Some(line_number) = new_line {
                markers.push((
                    line_number.saturating_sub(1) as usize,
                    if deletion_pending { '~' } else { '+' },
                ));
            }
            new_line = new_line.map(|line| line.saturating_add(1));
            deletion_pending = false;
        } else if line.starts_with('-') {
            deletion_pending = true;
        } else if line.starts_with(' ') {
            if deletion_pending && let Some(line_number) = new_line {
                markers.push((line_number.saturating_sub(1) as usize, '-'));
            }
            new_line = new_line.map(|line| line.saturating_add(1));
            deletion_pending = false;
        }
    }
    if deletion_pending
        && path.as_ref() == Some(target)
        && let Some(line_number) = new_line
    {
        markers.push((line_number.saturating_sub(1) as usize, '-'));
    }
    markers
}

fn diff_file_destination(lines: &[&str], header_index: usize) -> Option<(RepoPath, usize)> {
    let mut destination = None;
    let mut deleted = false;
    let mut first_line = 1;
    for line in lines.iter().copied().skip(header_index + 1) {
        if line.starts_with("diff --git") {
            break;
        }
        if destination.is_none()
            && let Some(path) = line.strip_prefix("+++ ")
        {
            if path == "/dev/null" {
                deleted = true;
            } else {
                destination = parse_git_diff_path(path);
            }
        }
        if line.starts_with("@@") {
            first_line = parse_hunk_lines(line).map_or(1, |(_, line)| line.max(1) as usize);
            break;
        }
    }
    if deleted {
        return None;
    }
    let destination = destination.or_else(|| {
        let header = lines[header_index].strip_prefix("diff --git ")?;
        if let Some((_, path)) = parse_git_diff_tokens(header) {
            return path
                .strip_prefix(b"b/")
                .and_then(|path| RepoPath::from_git_bytes(path).ok());
        }
        if let Some((_, path)) = header.rsplit_once(" b/") {
            return RepoPath::from_git_bytes(path.as_bytes()).ok();
        }
        None
    })?;
    Some((destination, first_line))
}

fn parse_git_diff_path(value: &str) -> Option<RepoPath> {
    let bytes = if value.starts_with('"') {
        parse_git_diff_token(value.as_bytes())?.0
    } else {
        value.as_bytes().to_vec()
    };
    let path = bytes.strip_prefix(b"b/").unwrap_or(bytes.as_slice());
    RepoPath::from_git_bytes(path).ok()
}

fn parse_git_diff_tokens(value: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let bytes = value.as_bytes();
    let (first, consumed) = parse_git_diff_token(bytes)?;
    let remaining = bytes.get(consumed..)?;
    let whitespace = remaining
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())?;
    let (second, _) = parse_git_diff_token(&remaining[whitespace..])?;
    Some((first, second))
}

fn parse_git_diff_token(value: &[u8]) -> Option<(Vec<u8>, usize)> {
    if value.first() != Some(&b'"') {
        let end = value
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(value.len());
        return Some((value[..end].to_vec(), end));
    }
    let mut output = Vec::new();
    let mut index = 1;
    while index < value.len() {
        match value[index] {
            b'"' => return Some((output, index + 1)),
            b'\\' => {
                index += 1;
                let escaped = *value.get(index)?;
                if escaped.is_ascii_digit() && escaped < b'8' {
                    let mut byte = 0_u8;
                    let mut digits = 0;
                    while digits < 3
                        && value
                            .get(index)
                            .is_some_and(|byte| (b'0'..=b'7').contains(byte))
                    {
                        byte = byte.saturating_mul(8).saturating_add(value[index] - b'0');
                        index += 1;
                        digits += 1;
                    }
                    output.push(byte);
                    continue;
                }
                output.push(match escaped {
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 0x0b,
                    b'f' => 0x0c,
                    b'r' => b'\r',
                    other => other,
                });
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    None
}

fn line_number(new: Option<u32>) -> Vec<Span<'static>> {
    vec![Span::styled(
        format!(
            "{:>5} ",
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
    fn maps_only_new_side_diff_rows() {
        let diff =
            "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2,2 +2,3 @@\n context\n-old\n+new\n+more\n";

        let mapped_line =
            |row| diff_new_line_and_payload_at_display_row(diff, row, false).map(|(line, _)| line);
        assert_eq!(mapped_line(0), None);
        assert_eq!(mapped_line(1), Some(2));
        assert_eq!(mapped_line(2), None);
        assert_eq!(mapped_line(3), Some(3));
        assert_eq!(mapped_line(4), Some(4));
    }

    #[test]
    fn diff_and_source_line_numbers_share_the_same_gutter() {
        let diff = "@@ -2 +2 @@\n-old\n+new\n";
        let diff_line = styled_diff(diff, "notes.txt", 100, false)[2]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let source_line = styled_source("old\nnew\n", "notes.txt", 100)[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(diff_line, "    2 +new");
        assert_eq!(source_line, "    2  new");
        assert_eq!(diff_line.find("new"), source_line.find("new"));
    }

    #[test]
    fn maps_commit_diff_file_headers_to_paths_and_first_new_lines() {
        let diff = concat!(
            "diff --git a/src/first.rs b/src/first.rs\n",
            "--- a/src/first.rs\n",
            "+++ b/src/first.rs\n",
            "@@ -10 +12 @@\n",
            "+first\n",
            "diff --git \"a/src/space name.rs\" \"b/src/space name.rs\"\n",
            "--- \"a/src/space name.rs\"\n",
            "+++ \"b/src/space\\040name.rs\"\n",
            "@@ -40 +42 @@\n",
            "+second\n",
        );

        assert_eq!(
            diff_file_header_at_display_row(diff, 0, true),
            Some((RepoPath::from("src/first.rs"), 12))
        );
        assert_eq!(diff_file_header_at_display_row(diff, 3, true), None);
        assert_eq!(
            diff_file_header_at_display_row(diff, 4, true),
            Some((RepoPath::from("src/space name.rs"), 42))
        );
        assert_eq!(diff_file_header_at_display_row(diff, 0, false), None);
        assert_eq!(
            diff_file_position_at_display_row(diff, 2, true),
            Some((RepoPath::from("src/first.rs"), 12, "first"))
        );
        assert_eq!(
            diff_file_position_at_display_row(diff, 6, true),
            Some((RepoPath::from("src/space name.rs"), 42, "second"))
        );
        assert_eq!(diff_file_position_at_display_row(diff, 3, true), None);
    }

    #[test]
    fn maps_quoted_header_only_paths_containing_the_diff_separator() {
        let diff =
            "diff --git \"a/odd b/target\" \"b/odd b/target\"\nold mode 100644\nnew mode 100755\n";

        assert_eq!(
            diff_file_header_at_display_row(diff, 0, true),
            Some((RepoPath::from("odd b/target"), 1))
        );
    }

    #[test]
    fn maps_added_modified_and_removed_lines_for_editor_gutters() {
        let diff = concat!(
            "diff --git a/src/main.rs b/src/main.rs\n",
            "--- a/src/main.rs\n",
            "+++ b/src/main.rs\n",
            "@@ -1,3 +1,4 @@\n",
            " context\n",
            "-old\n",
            "+new\n",
            "+more\n",
        );

        assert_eq!(
            diff_new_line_markers(diff, &RepoPath::from("src/main.rs")),
            vec![(1, '~'), (2, '+')]
        );
    }

    #[test]
    fn maps_wrapped_clicks_to_source_columns() {
        let content = "alpha beta gamma";

        assert_eq!(word_wrapped_column_at(content, 10, 0, 6), Some(6));
        assert_eq!(word_wrapped_column_at(content, 10, 1, 0), Some(11));
        assert_eq!(word_wrapped_column_at(content, 10, 1, 3), Some(14));
    }

    #[test]
    fn maps_tabs_and_wide_graphemes_by_terminal_cells() {
        assert_eq!(word_wrapped_column_at("\tvalue", 4, 1, 2), Some(6));
        assert_eq!(word_wrapped_column_at("界x", 8, 0, 2), Some(2));
    }

    #[test]
    fn height_only_wrapping_matches_cursor_rows_for_mixed_text() {
        let atoms = ["", "a", " ", "\t", "界", "👩‍💻", "e\u{301}"];
        for first in atoms {
            for second in atoms {
                for third in atoms {
                    let content = format!("{first}{second}{third}");
                    for width in 0..=12 {
                        assert_eq!(
                            word_wrapped_height(&content, width),
                            word_wrapped_rows(&content, width).len(),
                            "content={content:?}, width={width}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn batched_syntax_spans_match_grapheme_by_grapheme_rendering() {
        let cases = [
            ("fn\tmain() { let café = \"界\"; }", Language::Rust),
            (
                "def\tvalue(name): # e\u{301}\n  return name",
                Language::Python,
            ),
            ("const\tvalue = `👩‍💻`;", Language::JavaScript),
        ];
        for (content, language) in cases {
            let mut actual = Vec::new();
            push_syntax_spans(&mut actual, content, language);
            assert_eq!(actual, legacy_syntax_spans(content, language));
        }
    }

    fn legacy_syntax_spans(code: &str, language: Language) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut column = 0usize;
        for span in syntax_spans_for_language(code, language) {
            for grapheme in span.content.graphemes(true) {
                if grapheme == "\t" {
                    let width = crate::app::TAB_WIDTH - column % crate::app::TAB_WIDTH;
                    push_merged_span(&mut spans, Span::styled(" ".repeat(width), span.style));
                    column = column.saturating_add(width);
                } else {
                    push_merged_span(&mut spans, Span::styled(grapheme.to_owned(), span.style));
                    column = column.saturating_add(UnicodeWidthStr::width(grapheme));
                }
            }
        }
        spans
    }

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
    fn does_not_style_untracked_source_markers_as_diff_lines() {
        let lines = styled_diff(
            "Untracked file: SESSION.md\n\n- first item\n+ literal plus",
            "SESSION.md",
            100,
            false,
        );

        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2].style.bg, Some(palette().panel));
        assert_eq!(lines[3].style.bg, Some(palette().panel));
        assert_eq!(lines[2].spans[0].content, "-");
        assert_eq!(lines[3].spans[0].content, "+");
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
