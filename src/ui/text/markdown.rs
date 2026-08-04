use pulldown_cmark::{
    Alignment as MarkdownAlignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType,
    Options, Parser, Tag, TagEnd,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    super::palette,
    syntax::{Language, syntax_spans_for_language},
};

const MAX_RENDERED_LINES: usize = 30_000;

pub(crate) fn styled_markdown(
    markdown: &str,
    width: usize,
    wrap_tables: bool,
) -> Vec<Line<'static>> {
    styled_markdown_with_breaks(markdown, width, wrap_tables, false, true)
}

pub(crate) fn styled_markdown_preserving_breaks(
    markdown: &str,
    width: usize,
    wrap_tables: bool,
) -> Vec<Line<'static>> {
    styled_markdown_with_breaks(markdown, width, wrap_tables, true, false)
}

fn styled_markdown_with_breaks(
    markdown: &str,
    width: usize,
    wrap_tables: bool,
    preserve_soft_breaks: bool,
    show_code_language: bool,
) -> Vec<Line<'static>> {
    let mut renderer =
        MarkdownRenderer::new(width, wrap_tables, preserve_soft_breaks, show_code_language);
    for event in Parser::new_ext(markdown, markdown_options()) {
        if renderer.at_line_limit() {
            renderer.truncated = true;
            break;
        }
        renderer.event(event);
    }
    renderer.finish()
}

fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT
        | Options::ENABLE_WIKILINKS
}

struct ListState {
    next: Option<u64>,
    marker: Option<String>,
    marker_used: bool,
}

enum BlockState {
    Quote {
        alert: Option<BlockQuoteKind>,
        alert_used: bool,
    },
    List(ListState),
    Footnote {
        label: String,
        label_used: bool,
    },
    Definition {
        marker_used: bool,
    },
}

struct LinkState {
    destination: String,
    show_destination: bool,
}

struct TableState {
    alignments: Vec<MarkdownAlignment>,
    rows: Vec<Vec<Vec<Span<'static>>>>,
    row: Vec<Vec<Span<'static>>>,
    header_rows: usize,
}

struct MarkdownRenderer {
    width: usize,
    wrap_tables: bool,
    preserve_soft_breaks: bool,
    show_code_language: bool,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    blocks: Vec<BlockState>,
    code_block: bool,
    code_language: Language,
    table: Option<TableState>,
    links: Vec<LinkState>,
    metadata_depth: usize,
    truncated: bool,
}

impl MarkdownRenderer {
    fn new(
        width: usize,
        wrap_tables: bool,
        preserve_soft_breaks: bool,
        show_code_language: bool,
    ) -> Self {
        Self {
            width,
            wrap_tables,
            preserve_soft_breaks,
            show_code_language,
            lines: Vec::new(),
            spans: Vec::new(),
            styles: Vec::new(),
            blocks: Vec::new(),
            code_block: false,
            code_language: Language::Generic,
            table: None,
            links: Vec::new(),
            metadata_depth: 0,
            truncated: false,
        }
    }

    fn event(&mut self, event: Event<'_>) {
        if self.metadata_depth > 0 {
            match event {
                Event::Start(Tag::MetadataBlock(_)) => self.metadata_depth += 1,
                Event::End(TagEnd::MetadataBlock(_)) => self.metadata_depth -= 1,
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(code) => self.push_styled(
                &code,
                Style::default()
                    .fg(palette().yellow)
                    .bg(palette().surface_alt),
            ),
            Event::SoftBreak => {
                if self.preserve_soft_breaks {
                    self.finish_line(false);
                } else {
                    self.push_text(" ");
                }
            }
            Event::HardBreak => self.finish_line(false),
            Event::Rule => {
                self.finish_line(false);
                let width = self
                    .width
                    .saturating_sub(self.block_prefix_width())
                    .clamp(1, 48);
                self.push_block_line(vec![Span::styled(
                    "-".repeat(width),
                    Style::default().fg(palette().faint),
                )]);
                self.blank_line();
            }
            Event::Html(html) => self.push_html(&html),
            Event::InlineHtml(html) => self.push_inline_html(&html),
            Event::FootnoteReference(name) => {
                self.push_styled(&format!("[{name}]"), Style::default().fg(palette().accent))
            }
            Event::TaskListMarker(checked) => self.push_styled(
                if checked { "[x] " } else { "[ ] " },
                Style::default().fg(if checked {
                    palette().green
                } else {
                    palette().muted
                }),
            ),
            Event::InlineMath(math) => {
                self.push_styled(&math, Style::default().fg(palette().purple))
            }
            Event::DisplayMath(math) => {
                self.blank_line();
                self.push_styled("  ", Style::default().bg(palette().surface_alt));
                self.push_styled(
                    &math,
                    Style::default()
                        .fg(palette().purple)
                        .bg(palette().surface_alt),
                );
                self.finish_line(false);
                self.blank_line();
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.blank_line();
                self.styles.push(heading_style(level));
            }
            Tag::BlockQuote(alert) => {
                self.finish_line(false);
                self.blocks.push(BlockState::Quote {
                    alert,
                    alert_used: false,
                });
            }
            Tag::CodeBlock(kind) => {
                self.blank_line();
                self.code_block = true;
                self.code_language = match &kind {
                    CodeBlockKind::Fenced(info) => {
                        Language::from_name(info.split_whitespace().next().unwrap_or_default())
                    }
                    CodeBlockKind::Indented => Language::Generic,
                };
                if self.show_code_language
                    && let CodeBlockKind::Fenced(language) = kind
                    && !language.is_empty()
                {
                    self.push_styled(
                        &format!(" {language} "),
                        Style::default()
                            .fg(palette().muted)
                            .bg(palette().surface_alt)
                            .add_modifier(Modifier::BOLD),
                    );
                    self.finish_line(false);
                }
            }
            Tag::List(start) => {
                self.blank_line();
                self.blocks.push(BlockState::List(ListState {
                    next: start,
                    marker: None,
                    marker_used: false,
                }));
            }
            Tag::Item => {
                self.finish_line(false);
                if let Some(list) = self.blocks.iter_mut().rev().find_map(|block| match block {
                    BlockState::List(list) => Some(list),
                    _ => None,
                }) {
                    list.marker = Some(match list.next {
                        Some(number) => {
                            list.next = Some(number.saturating_add(1));
                            format!("{number}. ")
                        }
                        None => "* ".to_owned(),
                    });
                    list.marker_used = false;
                }
            }
            Tag::FootnoteDefinition(name) => {
                self.blank_line();
                self.blocks.push(BlockState::Footnote {
                    label: format!("[{name}] "),
                    label_used: false,
                });
            }
            Tag::Table(alignments) => {
                self.blank_line();
                self.table = Some(TableState {
                    alignments,
                    rows: Vec::new(),
                    row: Vec::new(),
                    header_rows: 0,
                });
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.row.clear();
                }
                self.styles
                    .push(Style::default().add_modifier(Modifier::BOLD));
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.row.clear();
                }
            }
            Tag::TableCell => self.spans.clear(),
            Tag::Emphasis => self
                .styles
                .push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::default().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                self.links.push(LinkState {
                    destination: dest_url.into_string(),
                    show_destination: !matches!(link_type, LinkType::Autolink | LinkType::Email),
                });
                self.styles.push(
                    Style::default()
                        .fg(palette().accent)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Image { dest_url, .. } => {
                self.push_styled("image: ", Style::default().fg(palette().muted));
                self.links.push(LinkState {
                    destination: dest_url.into_string(),
                    show_destination: true,
                });
                self.styles.push(
                    Style::default()
                        .fg(palette().purple)
                        .add_modifier(Modifier::ITALIC),
                );
            }
            Tag::HtmlBlock => {}
            Tag::MetadataBlock(_) => self.metadata_depth = 1,
            Tag::DefinitionList => self.blank_line(),
            Tag::DefinitionListTitle => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::DefinitionListDefinition => {
                self.finish_line(false);
                self.blocks
                    .push(BlockState::Definition { marker_used: false });
            }
            Tag::Superscript => self.styles.push(Style::default().fg(palette().purple)),
            Tag::Subscript => self.styles.push(Style::default().fg(palette().muted)),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.finish_line(false);
                self.blank_line();
            }
            TagEnd::Heading(_) => {
                self.styles.pop();
                self.finish_line(false);
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.finish_line(false);
                self.pop_block(|block| matches!(block, BlockState::Quote { .. }));
                self.blank_line();
            }
            TagEnd::CodeBlock => {
                self.finish_line(false);
                self.code_block = false;
                self.code_language = Language::Generic;
                self.blank_line();
            }
            TagEnd::List(_) => {
                self.finish_line(false);
                self.pop_block(|block| matches!(block, BlockState::List(_)));
                self.blank_line();
            }
            TagEnd::Item => {
                self.finish_line(false);
                if let Some(list) = self.blocks.iter_mut().rev().find_map(|block| match block {
                    BlockState::List(list) => Some(list),
                    _ => None,
                }) {
                    list.marker = None;
                }
            }
            TagEnd::FootnoteDefinition => {
                self.finish_line(false);
                self.pop_block(|block| matches!(block, BlockState::Footnote { .. }));
                self.blank_line();
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
                self.blank_line();
            }
            TagEnd::TableHead => {
                self.styles.pop();
                if let Some(table) = &mut self.table {
                    if !table.row.is_empty() {
                        table.rows.push(std::mem::take(&mut table.row));
                    }
                    table.header_rows = table.rows.len();
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    table.rows.push(std::mem::take(&mut table.row));
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    table.row.push(std::mem::take(&mut self.spans));
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript => {
                self.styles.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                self.styles.pop();
                if let Some(link) = self.links.pop()
                    && link.show_destination
                    && !link.destination.is_empty()
                {
                    self.push_styled(
                        &format!(" ({})", compact_destination(&link.destination, 72)),
                        Style::default().fg(palette().faint),
                    );
                }
            }
            TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) | TagEnd::DefinitionList => {}
            TagEnd::DefinitionListTitle => {
                self.styles.pop();
                self.finish_line(false);
            }
            TagEnd::DefinitionListDefinition => {
                self.finish_line(false);
                self.pop_block(|block| matches!(block, BlockState::Definition { .. }));
            }
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.code_block {
            self.push_code_text(text);
            return;
        }
        let mut parts = text.split('\n').peekable();
        while let Some(part) = parts.next() {
            if !part.is_empty() {
                self.ensure_prefix();
                let style = self.current_style();
                let column = spans_width(&self.spans);
                self.spans
                    .push(Span::styled(expand_tabs(part, column), style));
            }
            if parts.peek().is_some() {
                if self.code_block {
                    self.ensure_prefix();
                }
                self.finish_line(true);
            }
        }
    }

    fn push_code_text(&mut self, text: &str) {
        let mut parts = text.split('\n').peekable();
        while let Some(part) = parts.next() {
            self.ensure_prefix();
            if !part.is_empty() {
                let code = expand_tabs(part, spans_width(&self.spans));
                let base = self.current_style();
                self.spans.extend(
                    syntax_spans_for_language(&code, self.code_language)
                        .into_iter()
                        .map(|span| {
                            Span::styled(span.content.into_owned(), base.patch(span.style))
                        }),
                );
            }
            if parts.peek().is_some() {
                self.finish_line(true);
            }
        }
    }

    fn push_styled(&mut self, text: &str, style: Style) {
        self.ensure_prefix();
        self.spans.push(Span::styled(
            text.to_owned(),
            self.current_style().patch(style),
        ));
    }

    fn current_style(&self) -> Style {
        let mut style = Style::default().fg(palette().ink);
        if self.code_block {
            style = style.bg(palette().surface_alt).fg(palette().ink);
        }
        for nested in &self.styles {
            style = style.patch(*nested);
        }
        style
    }

    fn ensure_prefix(&mut self) {
        if !self.spans.is_empty() {
            return;
        }
        if self.table.is_some() {
            return;
        }
        let mut prefix = String::new();
        for block in &mut self.blocks {
            match block {
                BlockState::Quote { alert, alert_used } => {
                    prefix.push_str("> ");
                    if let Some(alert) = alert.filter(|_| !*alert_used) {
                        *alert_used = true;
                        prefix.push_str(alert_label(alert));
                        prefix.push_str("  ");
                    }
                }
                BlockState::List(list) => {
                    if let Some(marker) = &list.marker {
                        if list.marker_used {
                            prefix.push_str(&" ".repeat(UnicodeWidthStr::width(marker.as_str())));
                        } else {
                            list.marker_used = true;
                            prefix.push_str(marker);
                        }
                    } else {
                        prefix.push_str("  ");
                    }
                }
                BlockState::Footnote { label, label_used } => {
                    if *label_used {
                        prefix.push_str(&" ".repeat(UnicodeWidthStr::width(label.as_str())));
                    } else {
                        *label_used = true;
                        prefix.push_str(label);
                    }
                }
                BlockState::Definition { marker_used } => {
                    prefix.push_str(if *marker_used { "    " } else { "  : " });
                    *marker_used = true;
                }
            }
        }
        if !prefix.is_empty() {
            self.spans
                .push(Span::styled(prefix, markdown_prefix_style()));
        }
        if self.code_block {
            if let Some(prefix) = self
                .spans
                .first_mut()
                .filter(|span| span.style == markdown_prefix_style())
            {
                prefix.content.to_mut().push_str("  ");
            } else {
                self.spans.push(Span::styled("  ", markdown_prefix_style()));
            }
        }
    }

    fn pop_block(&mut self, predicate: impl Fn(&BlockState) -> bool) {
        if let Some(index) = self.blocks.iter().rposition(predicate) {
            self.blocks.remove(index);
        }
    }

    fn push_html(&mut self, html: &str) {
        let text = html_plaintext(html);
        if !text.trim().is_empty() {
            self.push_text(text.trim_matches('\n'));
            self.finish_line(false);
        }
    }

    fn push_inline_html(&mut self, html: &str) {
        let tag = html
            .trim()
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split(|character: char| {
                character.is_whitespace() || character == '>' || character == '/'
            })
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(tag.as_str(), "br" | "p" | "div" | "li" | "tr") {
            self.finish_line(false);
        }
    }

    fn block_prefix_width(&self) -> usize {
        self.blocks
            .iter()
            .map(|block| match block {
                BlockState::Quote { alert, alert_used } => {
                    2 + alert
                        .filter(|_| !*alert_used)
                        .map_or(0, |kind| alert_label(kind).len() + 2)
                }
                BlockState::List(list) => list.marker.as_deref().map_or(2, UnicodeWidthStr::width),
                BlockState::Footnote { label, .. } => UnicodeWidthStr::width(label.as_str()),
                BlockState::Definition { .. } => 4,
            })
            .sum::<usize>()
            .saturating_add(usize::from(self.code_block) * 2)
    }

    fn push_block_line(&mut self, spans: Vec<Span<'static>>) {
        if self.lines.len() >= MAX_RENDERED_LINES {
            self.truncated = true;
            return;
        }
        self.ensure_prefix();
        self.spans.extend(spans);
        self.finish_line(false);
    }

    fn at_line_limit(&self) -> bool {
        self.lines
            .len()
            .saturating_add(self.table.as_ref().map_or(0, |table| table.rows.len()))
            >= MAX_RENDERED_LINES
    }

    fn finish_line(&mut self, preserve_empty: bool) {
        if self.spans.is_empty() && !preserve_empty {
            return;
        }
        let mut line = Line::from(std::mem::take(&mut self.spans));
        if self.code_block {
            line = line.style(Style::default().bg(palette().surface_alt));
        }
        self.lines.push(line);
    }

    fn blank_line(&mut self) {
        self.finish_line(false);
        if self
            .lines
            .last()
            .is_some_and(|line| line.spans.iter().any(|span| !span.content.is_empty()))
        {
            self.lines.push(Line::default());
        }
    }

    fn render_table(&mut self, table: TableState) {
        let column_count = table
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(table.alignments.len());
        if column_count == 0 {
            return;
        }

        let available_width = self.width.saturating_sub(self.block_prefix_width()).max(1);
        let minimum_table_width = column_count.saturating_mul(4).saturating_add(1);
        if available_width < minimum_table_width {
            self.render_stacked_table(&table, column_count, available_width);
            return;
        }

        let mut widths = vec![1_usize; column_count];
        for row in &table.rows {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(spans_width(cell));
            }
        }
        let border_width = column_count.saturating_mul(3).saturating_add(1);
        let content_width = available_width
            .saturating_sub(border_width)
            .max(column_count);
        widths = fit_column_widths(&widths, content_width);

        self.push_table_border(&widths, '┌', '┬', '┐');
        for (row_index, row) in table.rows.iter().enumerate() {
            let cells = (0..column_count)
                .map(|column| {
                    let cell = row.get(column).map_or(&[][..], Vec::as_slice);
                    if self.wrap_tables {
                        wrap_spans(cell, widths[column])
                    } else {
                        vec![truncate_spans(cell, widths[column])]
                    }
                })
                .collect::<Vec<_>>();
            let row_height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for line_index in 0..row_height {
                let mut spans = vec![table_border("│")];
                for column in 0..column_count {
                    let cell = cells[column].get(line_index).map_or(&[][..], Vec::as_slice);
                    spans.push(Span::raw(" "));
                    spans.extend(aligned_cell(
                        cell,
                        widths[column],
                        table
                            .alignments
                            .get(column)
                            .copied()
                            .unwrap_or(MarkdownAlignment::None),
                    ));
                    spans.push(Span::raw(" "));
                    spans.push(table_border("│"));
                }
                self.push_block_line(spans);
            }
            if row_index.saturating_add(1) == table.header_rows {
                self.push_table_border(&widths, '├', '┼', '┤');
            }
        }
        self.push_table_border(&widths, '└', '┴', '┘');
    }

    fn push_table_border(&mut self, widths: &[usize], left: char, join: char, right: char) {
        let mut border = String::new();
        border.push(left);
        for (index, width) in widths.iter().enumerate() {
            border.push_str(&"─".repeat(width.saturating_add(2)));
            border.push(if index.saturating_add(1) == widths.len() {
                right
            } else {
                join
            });
        }
        self.push_block_line(vec![table_border(border)]);
    }

    fn render_stacked_table(
        &mut self,
        table: &TableState,
        column_count: usize,
        available_width: usize,
    ) {
        let header = table.rows.first().filter(|_| table.header_rows > 0);
        let body_start = usize::from(header.is_some());
        let rows = if table.rows.len() > body_start {
            &table.rows[body_start..]
        } else {
            &table.rows[..]
        };

        for (row_index, row) in rows.iter().enumerate() {
            for column in 0..column_count {
                let cell = row.get(column).map_or(&[][..], Vec::as_slice);
                let label = header
                    .and_then(|head| head.get(column))
                    .filter(|label| !label.is_empty());
                let mut spans = Vec::new();
                if let Some(label) = label.filter(|_| available_width >= 4) {
                    let label_budget = spans_width(label)
                        .min(available_width.saturating_sub(2) / 2)
                        .max(1);
                    spans.extend(truncate_spans(label, label_budget));
                    spans.push(Span::styled(": ", Style::default().fg(palette().faint)));
                }
                let used = spans_width(&spans);
                if self.wrap_tables {
                    spans.extend(cell.iter().cloned());
                    for line in wrap_spans(&spans, available_width) {
                        self.push_block_line(line);
                    }
                } else {
                    spans.extend(truncate_spans(cell, available_width.saturating_sub(used)));
                    self.push_block_line(spans);
                }
            }
            if row_index + 1 < rows.len() {
                self.lines.push(Line::default());
            }
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        if let Some(table) = self.table.take() {
            self.spans.clear();
            self.render_table(table);
        } else {
            self.finish_line(false);
        }
        if self.truncated || self.lines.len() > MAX_RENDERED_LINES {
            self.lines.truncate(MAX_RENDERED_LINES.saturating_sub(1));
            self.lines.push(Line::styled(
                "Markdown preview truncated; switch to Source for the complete file.",
                Style::default().fg(palette().muted),
            ));
        }
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        self.lines
    }
}

pub(crate) fn markdown_prefix_style() -> Style {
    Style::default().fg(palette().muted)
}

fn alert_label(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => "NOTE",
        BlockQuoteKind::Tip => "TIP",
        BlockQuoteKind::Important => "IMPORTANT",
        BlockQuoteKind::Warning => "WARNING",
        BlockQuoteKind::Caution => "CAUTION",
    }
}

fn expand_tabs(text: &str, start_column: usize) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut column = start_column;
    for character in text.chars() {
        if character == '\t' {
            let spaces = 4 - column % 4;
            expanded.push_str(&" ".repeat(spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += character.width().unwrap_or(0);
        }
    }
    expanded
}

fn compact_destination(destination: &str, max_chars: usize) -> String {
    let count = destination.chars().count();
    if count <= max_chars {
        return destination.to_owned();
    }
    let side = max_chars.saturating_sub(3) / 2;
    let start = destination.chars().take(side).collect::<String>();
    let end = destination
        .chars()
        .skip(count.saturating_sub(side))
        .collect::<String>();
    format!("{start}...{end}")
}

fn fit_column_widths(natural: &[usize], target: usize) -> Vec<usize> {
    if natural.iter().sum::<usize>() <= target {
        return natural.to_vec();
    }
    let mut low = 1_usize;
    let mut high = natural.iter().copied().max().unwrap_or(1);
    while low < high {
        let cap = low.saturating_add(high).saturating_add(1) / 2;
        if natural.iter().map(|width| (*width).min(cap)).sum::<usize>() <= target {
            low = cap;
        } else {
            high = cap.saturating_sub(1);
        }
    }
    let mut widths = natural
        .iter()
        .map(|width| (*width).min(low))
        .collect::<Vec<_>>();
    let mut spare = target.saturating_sub(widths.iter().sum::<usize>());
    for (width, natural_width) in widths.iter_mut().zip(natural) {
        let increase = natural_width.saturating_sub(*width).min(spare);
        *width += increase;
        spare -= increase;
        if spare == 0 {
            break;
        }
    }
    widths
}

fn html_plaintext(html: &str) -> String {
    let mut output = String::new();
    let mut position = 0;
    let mut suppressed: Option<String> = None;
    while position < html.len() {
        let remaining = &html[position..];
        let Some(open) = remaining.find('<') else {
            if suppressed.is_none() {
                output.push_str(remaining);
            }
            break;
        };
        if suppressed.is_none() {
            output.push_str(&remaining[..open]);
        }
        position += open;
        let remaining = &html[position..];
        if remaining.starts_with("<!--") {
            position += remaining.find("-->").map_or(remaining.len(), |end| end + 3);
            continue;
        }
        let Some(close) = remaining.find('>') else {
            break;
        };
        let raw_tag = remaining[1..close].trim();
        let closing = raw_tag.starts_with('/');
        let name = raw_tag
            .trim_start_matches('/')
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if suppressed.as_deref() == Some(name.as_str()) && closing {
            suppressed = None;
        } else if suppressed.is_none() && !closing && matches!(name.as_str(), "script" | "style") {
            suppressed = Some(name.clone());
        } else if suppressed.is_none()
            && matches!(
                name.as_str(),
                "br" | "p"
                    | "div"
                    | "li"
                    | "tr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "summary"
                    | "details"
                    | "hr"
            )
            && !output.ends_with('\n')
        {
            output.push('\n');
        }
        position += close + 1;
    }
    output
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn aligned_cell(
    spans: &[Span<'static>],
    width: usize,
    alignment: MarkdownAlignment,
) -> Vec<Span<'static>> {
    let mut content = truncate_spans(spans, width);
    let padding = width.saturating_sub(spans_width(&content));
    let left = match alignment {
        MarkdownAlignment::Center => padding / 2,
        MarkdownAlignment::Right => padding,
        MarkdownAlignment::None | MarkdownAlignment::Left => 0,
    };
    let right = padding.saturating_sub(left);
    if left > 0 {
        content.insert(0, Span::raw(" ".repeat(left)));
    }
    if right > 0 {
        content.push(Span::raw(" ".repeat(right)));
    }
    content
}

fn truncate_spans(spans: &[Span<'static>], width: usize) -> Vec<Span<'static>> {
    let mut result = Vec::new();
    let mut used = 0_usize;
    for span in spans {
        let mut content = String::new();
        for grapheme in span.content.graphemes(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            if used.saturating_add(grapheme_width) > width {
                break;
            }
            content.push_str(grapheme);
            used = used.saturating_add(grapheme_width);
        }
        if !content.is_empty() {
            result.push(Span::styled(content, span.style));
        }
        if used >= width {
            break;
        }
    }
    result
}

fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut lines = vec![Vec::new()];
    let mut line_width = 0_usize;
    let mut pending_space = None;

    for span in spans {
        for segment in span.content.split_word_bounds() {
            if segment.chars().all(char::is_whitespace) {
                if line_width > 0 {
                    pending_space = Some(span.style);
                }
                continue;
            }

            let segment_width = UnicodeWidthStr::width(segment);
            if line_width > 0
                && line_width
                    .saturating_add(usize::from(pending_space.is_some()))
                    .saturating_add(segment_width)
                    > width
                && segment_width <= width
            {
                lines.push(Vec::new());
                line_width = 0;
                pending_space = None;
            }
            if line_width > 0
                && let Some(style) = pending_space.take()
            {
                if line_width < width {
                    push_span_content(lines.last_mut().expect("line exists"), " ", style);
                    line_width += 1;
                } else {
                    lines.push(Vec::new());
                    line_width = 0;
                }
            }

            for grapheme in segment.graphemes(true) {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                if line_width > 0 && line_width.saturating_add(grapheme_width) > width {
                    lines.push(Vec::new());
                    line_width = 0;
                }
                if grapheme_width <= width {
                    push_span_content(lines.last_mut().expect("line exists"), grapheme, span.style);
                    line_width = line_width.saturating_add(grapheme_width);
                }
            }
            pending_space = None;
        }
    }
    lines
}

fn push_span_content(spans: &mut Vec<Span<'static>>, content: &str, style: Style) {
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(content);
    } else {
        spans.push(Span::styled(content.to_owned(), style));
    }
}

fn table_border(content: impl Into<String>) -> Span<'static> {
    Span::styled(content.into(), Style::default().fg(palette().faint))
}

fn heading_style(level: HeadingLevel) -> Style {
    let (color, modifier) = match level {
        HeadingLevel::H1 => (palette().accent, Modifier::BOLD),
        HeadingLevel::H2 => (palette().cyan, Modifier::BOLD),
        HeadingLevel::H3 => (palette().green, Modifier::BOLD),
        HeadingLevel::H4 => (palette().yellow, Modifier::BOLD),
        HeadingLevel::H5 => (palette().purple, Modifier::BOLD),
        HeadingLevel::H6 => (palette().muted, Modifier::BOLD | Modifier::ITALIC),
    };
    Style::default().fg(color).add_modifier(modifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered_lines(markdown: &str, width: usize) -> Vec<String> {
        styled_markdown(markdown, width, false)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn renders_markdown_structure_without_source_markers() {
        let lines = styled_markdown(
            "# Title\n\nA **strong** [link](https://example.com).\n\n- one\n- two\n\n```rs\nfn main() {}\n```\n",
            80,
            false,
        );
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.starts_with("Title"));
        assert!(!text.contains("# Title"));
        assert!(text.contains("A strong link (https://example.com)."));
        assert!(text.contains("* one"));
        assert!(text.contains("fn main() {}"));
    }

    #[test]
    fn can_preserve_soft_breaks_while_styling_inline_markdown() {
        let lines = styled_markdown_preserving_breaks("**bold**\n`code`", 80, true);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "bold");
        assert!(lines[0].spans[0].style != Style::default());
        assert!(
            lines[1]
                .spans
                .iter()
                .any(|span| span.style.fg == Some(palette().yellow))
        );
    }

    #[test]
    fn syntax_highlights_fenced_code_without_a_language_caption() {
        let lines = styled_markdown_preserving_breaks(
            "```rust\nfn preview() {\n    println!(\"works\");\n}\n```",
            80,
            false,
        );
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(!text.contains("rust"), "{text:?}");
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| { span.content == "fn" && span.style.fg == Some(palette().purple) })
        );
        assert!(
            lines.iter().flat_map(|line| &line.spans).any(|span| {
                span.content == "preview" && span.style.fg == Some(palette().orange)
            })
        );
        assert!(
            lines.iter().flat_map(|line| &line.spans).any(|span| {
                span.content == "\"works\"" && span.style.fg == Some(palette().green)
            })
        );
    }

    #[test]
    fn renders_tables_as_bordered_aligned_grids() {
        let lines = styled_markdown(
            "| Name | Count |\n| :--- | ---: |\n| apples | 12 |\n| pears | 3 |\n",
            40,
            false,
        );
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(text.first().is_some_and(|line| line.starts_with('┌')));
        assert!(
            text.iter().any(|line| line.contains("│ Name   │ Count │")),
            "{text:#?}"
        );
        assert!(
            text.iter().any(|line| line.contains("│ apples │    12 │")),
            "{text:#?}"
        );
        assert!(text.iter().any(|line| line.starts_with('├')));
        assert!(text.last().is_some_and(|line| line.starts_with('└')));
    }

    #[test]
    fn wraps_table_cells_without_losing_content() {
        let markdown = "| Key | Description |\n| :--- | :--- |\n| alpha | beginning words continue across rows until TAIL |\n";
        let unwrapped = styled_markdown(markdown, 32, false);
        let wrapped = styled_markdown(markdown, 32, true);
        let wrapped_text = wrapped
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            !unwrapped
                .iter()
                .any(|line| line.spans.iter().any(|span| span.content.contains("TAIL")))
        );
        assert!(
            wrapped_text.iter().any(|line| line.contains("TAIL")),
            "{wrapped_text:#?}"
        );
        assert!(wrapped.len() > unwrapped.len());
        assert!(wrapped.iter().all(|line| spans_width(&line.spans) <= 32));
        assert!(
            wrapped_text
                .iter()
                .filter(|line| line.starts_with('│'))
                .all(|line| line.ends_with('│'))
        );
    }

    #[test]
    fn wraps_stacked_table_values_on_narrow_screens() {
        let markdown =
            "| Key | Description |\n| --- | --- |\n| alpha | words continue until TAIL |\n";
        let wrapped = styled_markdown(markdown, 8, true);
        let text = wrapped
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(text.iter().any(|line| line.contains("TAIL")), "{text:#?}");
        assert!(wrapped.iter().all(|line| spans_width(&line.spans) <= 8));
        assert!(text.iter().all(|line| !line.contains('│')));
    }

    #[test]
    fn preserves_nested_block_order_and_extended_structure() {
        let text = rendered_lines(
            "> - quoted list\n>   > nested quote\n\n- item\n  > quote in item\n\n> [!WARNING]\n> Pay attention.\n",
            80,
        );

        assert!(
            text.iter().any(|line| line == "> * quoted list"),
            "{text:#?}"
        );
        assert!(
            text.iter().any(|line| line == ">   > nested quote"),
            "{text:#?}"
        );
        assert!(text.iter().any(|line| line == "* item"), "{text:#?}");
        assert!(
            text.iter().any(|line| line == "  > quote in item"),
            "{text:#?}"
        );
        assert!(
            text.iter().any(|line| line == "> WARNING  Pay attention."),
            "{text:#?}"
        );
    }

    #[test]
    fn renders_safe_document_fallbacks_without_noisy_links() {
        let text = rendered_lines(
            "---\ntitle: hidden metadata\n---\n\n<https://example.com>\n\nbefore<br>after\n\n<div>visible <strong>HTML</strong></div>\n<script>hidden script</script>\n",
            80,
        )
        .join("\n");

        assert!(!text.contains("hidden metadata"), "{text}");
        assert_eq!(text.matches("https://example.com").count(), 1, "{text}");
        assert!(text.contains("before\nafter"), "{text}");
        assert!(text.contains("visible HTML"), "{text}");
        assert!(!text.contains("hidden script"), "{text}");
    }

    #[test]
    fn tables_fit_every_viewport_width() {
        let markdown = "| Very long heading | Count | Status |\n| :--- | ---: | :---: |\n| a very long value that needs truncation | 12345 | ready |\n";
        for width in 1..=50 {
            let lines = styled_markdown(markdown, width, false);
            assert!(
                lines.iter().all(|line| spans_width(&line.spans) <= width),
                "table exceeded width {width}: {:#?}",
                rendered_lines(markdown, width)
            );
        }
    }

    #[test]
    fn code_blocks_preserve_blank_lines_and_expand_tabs() {
        let text = rendered_lines("> ```rust\n> \tlet value = 1;\n>\n> \tvalue\n> ```\n", 80);

        assert!(text.iter().any(|line| line == ">    rust "), "{text:#?}");
        assert!(
            text.iter().any(|line| line == ">       let value = 1;"),
            "{text:#?}"
        );
        assert!(text.iter().any(|line| line == ">   "), "{text:#?}");
        assert!(text.iter().all(|line| !line.contains('\t')));
    }

    #[test]
    fn bounds_rendering_for_pathological_documents() {
        let markdown = "# heading\n\n".repeat(MAX_RENDERED_LINES);
        let lines = styled_markdown(&markdown, 80, false);

        assert!(lines.len() <= MAX_RENDERED_LINES);
        assert!(lines.last().is_some_and(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("preview truncated"))
        }));
    }
}
