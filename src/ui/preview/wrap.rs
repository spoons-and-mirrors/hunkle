use super::*;

pub(super) fn append_positioned_output(output: &mut Vec<u8>, area: Rect, command: &[u8]) {
    output.extend_from_slice(b"\x1b[s");
    output.extend_from_slice(format!("\x1b[{};{}H", area.y + 1, area.x + 1).as_bytes());
    output.extend_from_slice(command);
    output.extend_from_slice(b"\x1b[u");
}

pub(super) fn append_clear_area(output: &mut Vec<u8>, area: Rect) {
    if area.is_empty() {
        return;
    }
    output.extend_from_slice(b"\x1b[s");
    output.extend_from_slice(format!("\x1b[{};{}H", area.y + 1, area.x + 1).as_bytes());
    for row in 0..area.height {
        output.extend_from_slice(format!("\x1b[{}X", area.width).as_bytes());
        if row + 1 < area.height {
            output.extend_from_slice(b"\x1b[1B");
        }
    }
    output.extend_from_slice(b"\x1b[u");
}

pub(super) fn wrapped_styled_line_starts(lines: &[Line<'static>], width: usize) -> Vec<usize> {
    let mut starts: Vec<usize> = Vec::with_capacity(lines.len().saturating_add(1));
    starts.push(0);
    for line in lines {
        let height = hard_wrap_lines(vec![line.clone()], width, 0, usize::MAX, false, true)
            .len()
            .max(1);
        starts.push(starts.last().copied().unwrap_or(0).saturating_add(height));
    }
    starts
}

pub(super) fn markdown_content_width(width: usize) -> usize {
    if width >= MIN_NUMBERED_MARKDOWN_WIDTH {
        width.saturating_sub(MARKDOWN_LINE_GUTTER_WIDTH).max(1)
    } else {
        width.max(1)
    }
}

pub(super) fn numbered_markdown_lines(
    mut lines: Vec<Line<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
    if width < MIN_NUMBERED_MARKDOWN_WIDTH {
        return lines;
    }
    for (index, line) in lines.iter_mut().enumerate() {
        line.spans.insert(
            0,
            Span::styled(
                format!("{:>5}  ", index.saturating_add(1)),
                Style::default().fg(crate::ui::palette().faint),
            ),
        );
    }
    lines
}

pub(super) struct StyledChunk {
    content: String,
    style: Style,
    width: usize,
}

type WrapToken = (bool, Vec<StyledChunk>);

pub(super) fn hard_wrap_lines(
    lines: Vec<Line<'static>>,
    width: usize,
    skip: usize,
    take: usize,
    is_diff: bool,
    markdown: bool,
) -> Vec<Line<'static>> {
    if take == 0 {
        return Vec::new();
    }
    let width = width.max(1);
    let mut wrapped = Vec::new();
    let mut rendered = 0_usize;
    for line in lines {
        let line_style = line.style;
        let gutter = line_gutter(&line, width, is_diff, markdown);
        let mut output_spans = line.spans[..gutter.span_count].to_vec();
        let mut output_width = gutter.width;
        let mut tokens: Vec<WrapToken> = Vec::new();
        for span in &line.spans[gutter.span_count..] {
            for grapheme in span.content.graphemes(true) {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                let whitespace = grapheme.chars().all(char::is_whitespace);
                if tokens.last().is_none_or(|token| token.0 != whitespace) {
                    tokens.push((whitespace, Vec::new()));
                }
                let chunks = &mut tokens.last_mut().expect("token was inserted").1;
                if let Some(chunk) = chunks.last_mut()
                    && chunk.style == span.style
                {
                    chunk.content.push_str(grapheme);
                    chunk.width = chunk.width.saturating_add(grapheme_width);
                } else {
                    chunks.push(StyledChunk {
                        content: grapheme.to_owned(),
                        style: span.style,
                        width: grapheme_width,
                    });
                }
            }
        }

        let mut pending_whitespace = None;
        let mut has_word = false;
        for (whitespace, token) in tokens {
            if whitespace && has_word {
                pending_whitespace = Some(token);
                continue;
            }
            let token_width = token.iter().map(|chunk| chunk.width).sum::<usize>();
            let whitespace_width = pending_whitespace
                .as_ref()
                .map_or(0, |token: &Vec<StyledChunk>| {
                    token.iter().map(|chunk| chunk.width).sum()
                });
            if !whitespace
                && has_word
                && token_width <= width.saturating_sub(gutter.width)
                && output_width
                    .saturating_add(whitespace_width)
                    .saturating_add(token_width)
                    > width
            {
                if emit_wrapped_row(
                    &mut wrapped,
                    &mut rendered,
                    skip,
                    take,
                    &mut output_spans,
                    line_style,
                ) {
                    return wrapped;
                }
                start_continuation(&mut output_spans, &mut output_width, &gutter);
                pending_whitespace = None;
            } else if let Some(whitespace) = pending_whitespace.take()
                && append_wrap_token(
                    whitespace,
                    &mut output_spans,
                    &mut output_width,
                    &gutter,
                    width,
                    line_style,
                    &mut wrapped,
                    &mut rendered,
                    skip,
                    take,
                )
            {
                return wrapped;
            }
            if append_wrap_token(
                token,
                &mut output_spans,
                &mut output_width,
                &gutter,
                width,
                line_style,
                &mut wrapped,
                &mut rendered,
                skip,
                take,
            ) {
                return wrapped;
            }
            has_word |= !whitespace;
        }
        if emit_wrapped_row(
            &mut wrapped,
            &mut rendered,
            skip,
            take,
            &mut output_spans,
            line_style,
        ) {
            return wrapped;
        }
    }
    wrapped
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_wrap_token(
    token: Vec<StyledChunk>,
    output_spans: &mut Vec<Span<'static>>,
    output_width: &mut usize,
    gutter: &WrapGutter,
    width: usize,
    line_style: Style,
    wrapped: &mut Vec<Line<'static>>,
    rendered: &mut usize,
    skip: usize,
    take: usize,
) -> bool {
    for chunk in token {
        for grapheme in chunk.content.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if *output_width > gutter.width && output_width.saturating_add(grapheme_width) > width {
                if emit_wrapped_row(wrapped, rendered, skip, take, output_spans, line_style) {
                    return true;
                }
                start_continuation(output_spans, output_width, gutter);
            }
            if let Some(last) = output_spans.last_mut()
                && last.style == chunk.style
            {
                last.content.to_mut().push_str(grapheme);
            } else {
                output_spans.push(Span::styled(grapheme.to_owned(), chunk.style));
            }
            *output_width = output_width.saturating_add(grapheme_width);
        }
    }
    false
}

pub(super) fn emit_wrapped_row(
    wrapped: &mut Vec<Line<'static>>,
    rendered: &mut usize,
    skip: usize,
    take: usize,
    output_spans: &mut Vec<Span<'static>>,
    line_style: Style,
) -> bool {
    if *rendered >= skip {
        wrapped.push(Line::from(std::mem::take(output_spans)).style(line_style));
    } else {
        output_spans.clear();
    }
    *rendered = rendered.saturating_add(1);
    wrapped.len() == take
}

pub(super) fn start_continuation(
    output_spans: &mut Vec<Span<'static>>,
    output_width: &mut usize,
    gutter: &WrapGutter,
) {
    output_spans.extend(gutter.continuation.iter().cloned());
    *output_width = gutter.width;
}

#[derive(Default)]
pub(super) struct WrapGutter {
    width: usize,
    span_count: usize,
    continuation: Vec<Span<'static>>,
}

pub(super) fn line_gutter(
    line: &Line<'_>,
    width: usize,
    is_diff: bool,
    markdown: bool,
) -> WrapGutter {
    if markdown {
        let mut gutter = 0;
        let mut span_count = 0;
        let mut continuation = Vec::new();
        if let Some(number) = line.spans.first().filter(|span| {
            span.content.strip_suffix("  ").is_some_and(|prefix| {
                prefix.chars().count() >= 5 && prefix.trim().parse::<usize>().is_ok()
            })
        }) {
            gutter = UnicodeWidthStr::width(number.content.as_ref());
            span_count = 1;
            continuation.push(Span::raw(" ".repeat(gutter)));
        }
        if let Some(prefix) = line
            .spans
            .get(span_count)
            .filter(|span| span.style == markdown_prefix_style())
        {
            let prefix_width = UnicodeWidthStr::width(prefix.content.as_ref());
            if width > gutter.saturating_add(prefix_width) {
                gutter = gutter.saturating_add(prefix_width);
                span_count += 1;
                continuation.push(Span::styled(
                    markdown_continuation_prefix(prefix.content.as_ref()),
                    prefix.style,
                ));
            }
        }
        return if width > gutter && gutter > 0 {
            WrapGutter {
                width: gutter,
                span_count,
                continuation,
            }
        } else {
            WrapGutter::default()
        };
    }
    if !is_diff {
        let gutter = line
            .spans
            .first()
            .filter(|span| {
                span.content.strip_suffix("  ").is_some_and(|prefix| {
                    prefix.chars().count() >= 5 && prefix.trim().parse::<usize>().is_ok()
                })
            })
            .map_or(0, |span| UnicodeWidthStr::width(span.content.as_ref()));
        return if width > gutter && gutter > 0 {
            WrapGutter::spaces(gutter, 1)
        } else {
            WrapGutter::default()
        };
    }
    let marker = |span: &Span<'_>| matches!(span.content.as_ref(), "+" | "-" | " ");
    let (gutter, spans) = match line.spans.as_slice() {
        [number, marker_span, ..]
            if UnicodeWidthStr::width(number.content.as_ref()) == 6 && marker(marker_span) =>
        {
            (7, 2)
        }
        [marker_span, ..] if marker(marker_span) => (1, 1),
        _ => (0, 0),
    };
    if width > gutter {
        WrapGutter::spaces(gutter, spans)
    } else {
        WrapGutter::default()
    }
}

impl WrapGutter {
    fn spaces(width: usize, span_count: usize) -> Self {
        Self {
            width,
            span_count,
            continuation: vec![Span::raw(" ".repeat(width))],
        }
    }
}

pub(super) fn markdown_continuation_prefix(prefix: &str) -> String {
    let mut continuation = String::with_capacity(prefix.len());
    let mut remaining = prefix;
    while !remaining.is_empty() {
        if remaining.starts_with("> ") {
            continuation.push_str("> ");
            remaining = &remaining[2..];
        } else {
            let character = remaining.chars().next().expect("prefix is not empty");
            continuation.push_str(
                &" ".repeat(unicode_width::UnicodeWidthChar::width(character).unwrap_or(0)),
            );
            remaining = &remaining[character.len_utf8()..];
        }
    }
    continuation
}
