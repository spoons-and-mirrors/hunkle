pub(super) use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};

pub(super) use image::DynamicImage;
pub(super) use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Rect, Size},
    style::Style,
    text::{Line, Span},
};
pub(super) use ratatui_image::{
    Resize,
    errors::Errors as ImageError,
    picker::{Picker, ProtocolType},
    protocol::{StatefulProtocol, StatefulProtocolType, kitty::StatefulKitty},
    thread::{ResizeRequest, ResizeResponse, ThreadProtocol},
};
pub(super) use unicode_segmentation::UnicodeSegmentation;
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::media::MediaPreviewProtocol;

pub(super) use super::text::{
    markdown_prefix_style, styled_diff, styled_diff_window, styled_markdown, styled_source,
    styled_source_window, wrapped_source_line_starts,
};

mod diff;
pub(crate) use diff::{DiffDocument, DiffLineKind};
mod wrap;
pub(super) use wrap::hard_wrap_lines as hard_wrap_preview_lines;
use wrap::*;
#[cfg(test)]
mod tests;

const MAX_CACHED_PREVIEW_LINES: usize = 30_000;
const MAX_CACHED_PREVIEW_BYTES: usize = 512 * 1024;
const MARKDOWN_LINE_GUTTER_WIDTH: usize = 7;
const MIN_NUMBERED_MARKDOWN_WIDTH: usize = 12;

const KITTY_DELETE_ALL: &str = "\u{1b}_Ga=d,d=A,q=2\u{1b}\\";
const KITTY_DELETE_PLACEMENTS: &str = "\u{1b}_Ga=d,d=a,q=2\u{1b}\\";

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct KittyTransmission {
    image_id: u32,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct MediaTerminalOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) kitty: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveKittyImage {
    generation: u64,
    image_id: u32,
    area: Rect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveInlineImage {
    generation: u64,
    protocol: MediaPreviewProtocol,
    area: Rect,
}

pub(crate) fn take_kitty_transmission(
    buffer: &mut Buffer,
    area: Rect,
) -> Option<KittyTransmission> {
    const COMBINED_TRANSMIT: &str = ",a=T,U=1,f=32";
    let cell = buffer.cell((area.x, area.y))?;
    let symbol = cell.symbol().to_owned();
    let placeholders = symbol.find("\u{1b}[s")?;
    let transmission = &symbol[..placeholders];
    buffer
        .cell_mut((area.x, area.y))?
        .set_symbol(&symbol[placeholders..]);
    let id_start = transmission.find("_Gq=2,i=")? + "_Gq=2,i=".len();
    let id_end = id_start + transmission[id_start..].find(',')?;
    let image_id = transmission[id_start..id_end].parse().ok()?;
    let superfile_marker = format!(",a=T,U=1,c={},r={},f=32", area.width, area.height);
    let raw_transmission = transmission.replacen(COMBINED_TRANSMIT, &superfile_marker, 1);
    if raw_transmission == transmission {
        return None;
    }
    Some(KittyTransmission {
        image_id,
        bytes: raw_transmission.into_bytes(),
    })
}

pub(crate) fn take_inline_transmission(
    buffer: &mut Buffer,
    area: Rect,
    protocol: MediaPreviewProtocol,
) -> Option<Vec<u8>> {
    let marker = match protocol {
        MediaPreviewProtocol::Iterm2 => "]1337;File=",
        MediaPreviewProtocol::Sixel => "\u{1b}P",
        _ => return None,
    };
    let symbol = buffer.cell((area.x, area.y))?.symbol().to_owned();
    if !symbol.contains(marker) {
        return None;
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_symbol(" ");
                cell.set_diff_option(CellDiffOption::Skip);
            }
        }
    }
    Some(symbol.into_bytes())
}

#[derive(Clone, Copy)]
pub(crate) struct PreviewInput<'a> {
    pub(crate) content: PreviewContent<'a>,
    pub(crate) generation: u64,
    pub(crate) path: &'a str,
    pub(crate) markdown: bool,
    pub(crate) show_initial_diff_header: bool,
    pub(crate) width: usize,
    pub(crate) viewport_height: usize,
    pub(crate) wrapped: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum PreviewContent<'a> {
    Source(&'a str),
    Diff(&'a DiffDocument),
}

impl<'a> PreviewContent<'a> {
    fn as_str(self) -> &'a str {
        match self {
            Self::Source(source) => source,
            Self::Diff(document) => document.as_str(),
        }
    }

    fn is_diff(self) -> bool {
        matches!(self, Self::Diff(_))
    }
}

pub(crate) struct PreparedPreview {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) rendered_height: usize,
    pub(crate) wrapped: bool,
}

pub(crate) struct PreviewPresentation {
    cache: Option<PreviewCache>,
    media_state: Option<ThreadProtocol>,
    media_receiver: Receiver<Result<ResizeResponse, ImageError>>,
    media_worker: Option<JoinHandle<()>>,
    media_picker: Picker,
    allow_auto_kitty: bool,
    media_generation: Option<u64>,
    media_protocol: Option<MediaPreviewProtocol>,
    effective_media_protocol: MediaPreviewProtocol,
    media_size: Size,
    media_error: Option<String>,
    active_kitty_image: Option<ActiveKittyImage>,
    active_inline_image: Option<ActiveInlineImage>,
    pending_terminal_cleanup: Vec<u8>,
    pending_terminal_output: MediaTerminalOutput,
}

struct PreviewCache {
    generation: u64,
    path: String,
    is_diff: bool,
    markdown: bool,
    markdown_wrapped: bool,
    show_initial_diff_header: bool,
    width: usize,
    lines: Vec<Line<'static>>,
    fully_styled: bool,
    window_start: usize,
    display_count: usize,
    wrapped_line_starts: Option<Vec<usize>>,
    wrapped_window: Option<WrappedWindow>,
    unwrapped_hunks: Option<(Vec<(usize, usize)>, usize)>,
    wrapped_hunks: Option<(Vec<(usize, usize)>, usize)>,
}

struct WrappedWindow {
    first: usize,
    end: usize,
    local_scroll: usize,
    viewport_height: usize,
    lines: Vec<Line<'static>>,
}

impl Default for PreviewPresentation {
    fn default() -> Self {
        let (request_sender, request_receiver) = mpsc::channel::<ResizeRequest>();
        let (result_sender, media_receiver) = mpsc::channel();
        let media_worker = thread::spawn(move || {
            while let Ok(request) = request_receiver.recv() {
                if result_sender.send(request.resize_encode()).is_err() {
                    break;
                }
            }
        });
        Self {
            cache: None,
            media_state: Some(ThreadProtocol::new(request_sender, None)),
            media_receiver,
            media_worker: Some(media_worker),
            media_picker: Picker::halfblocks(),
            allow_auto_kitty: false,
            media_generation: None,
            media_protocol: None,
            effective_media_protocol: MediaPreviewProtocol::Halfblocks,
            media_size: Size::default(),
            media_error: None,
            active_kitty_image: None,
            active_inline_image: None,
            pending_terminal_cleanup: Vec::new(),
            pending_terminal_output: MediaTerminalOutput::default(),
        }
    }
}

impl PreviewPresentation {
    pub(crate) fn clear(&mut self) {
        self.cache = None;
        self.hide_media();
    }

    pub(crate) fn source_position_at_rendered_position(
        &self,
        content: &str,
        row: usize,
        column: usize,
        gutter: usize,
    ) -> Option<(usize, usize)> {
        let cache = self.cache.as_ref()?;
        if cache.display_count == 0 && !cache.is_diff && !cache.markdown && row == 0 {
            return Some((1, 0));
        }
        let (display_line, wrapped_row) = self.display_position_at_rendered_row(row)?;
        let line = content.lines().nth(display_line).unwrap_or_default();
        let column = column.saturating_sub(gutter);
        let source_column = if cache.wrapped_line_starts.is_some() {
            super::text::word_wrapped_column_at(
                line,
                cache.width.saturating_sub(gutter).max(1),
                wrapped_row,
                column,
            )?
        } else {
            column
        };
        Some((display_line.saturating_add(1), source_column))
    }

    pub(crate) fn diff_position_at_rendered_position(
        &self,
        diff: &DiffDocument,
        row: usize,
        column: usize,
        gutter: usize,
    ) -> Option<(usize, usize)> {
        let cache = self.cache.as_ref()?;
        let (display_line, wrapped_row) = self.display_position_at_rendered_row(row)?;
        let (source_line, payload) = diff.display_new_position(display_line, false)?;
        let column = column.saturating_sub(gutter);
        let source_column = if cache.wrapped_line_starts.is_some() {
            super::text::word_wrapped_column_at(
                payload,
                cache.width.saturating_sub(gutter).max(1),
                wrapped_row,
                column,
            )?
        } else {
            column
        };
        Some((source_line, source_column))
    }

    pub(crate) fn diff_file_position_at_rendered_position(
        &self,
        diff: &DiffDocument,
        row: usize,
        column: usize,
        gutter: usize,
    ) -> Option<(crate::repo_path::RepoPath, usize, usize)> {
        let cache = self.cache.as_ref()?;
        let (display_line, wrapped_row) = self.display_position_at_rendered_row(row)?;
        let (path, source_line, payload) =
            diff.display_file_position(display_line, cache.show_initial_diff_header)?;
        let column = column.saturating_sub(gutter);
        let source_column = if cache.wrapped_line_starts.is_some() {
            super::text::word_wrapped_column_at(
                payload,
                cache.width.saturating_sub(gutter).max(1),
                wrapped_row,
                column,
            )?
        } else {
            column
        };
        Some((path, source_line, source_column))
    }

    pub(crate) fn diff_file_header_at_rendered_row(
        &self,
        diff: &DiffDocument,
        row: usize,
    ) -> Option<(crate::repo_path::RepoPath, usize)> {
        let cache = self.cache.as_ref()?;
        let (display_line, _) = self.display_position_at_rendered_row(row)?;
        diff.display_file_header(display_line, cache.show_initial_diff_header)
    }

    fn display_position_at_rendered_row(&self, row: usize) -> Option<(usize, usize)> {
        let cache = self.cache.as_ref()?;
        if let Some(starts) = &cache.wrapped_line_starts {
            let line = starts
                .partition_point(|start| *start <= row)
                .checked_sub(1)
                .filter(|line| *line < cache.display_count)?;
            Some((line, row.saturating_sub(starts[line])))
        } else {
            (row < cache.display_count).then_some((row, 0))
        }
    }

    pub(crate) fn rendered_row_for_source_line(&self, line: usize) -> Option<usize> {
        let cache = self.cache.as_ref()?;
        if cache.is_diff || cache.markdown || line == 0 || line > cache.display_count {
            return None;
        }
        let display_line = line - 1;
        Some(
            cache
                .wrapped_line_starts
                .as_ref()
                .map_or(display_line, |starts| starts[display_line]),
        )
    }

    pub(crate) fn hide_media(&mut self) {
        if self.active_kitty_image.take().is_some() {
            self.pending_terminal_cleanup
                .extend_from_slice(KITTY_DELETE_ALL.as_bytes());
        }
        if let Some(active) = self.active_inline_image.take() {
            append_clear_area(&mut self.pending_terminal_cleanup, active.area);
        }
        if self.media_generation.take().is_some() {
            self.media_state
                .as_mut()
                .expect("media worker is running")
                .empty_protocol();
        }
        self.media_protocol = None;
        self.media_size = Size::default();
        self.media_error = None;
    }

    pub(crate) fn poll_media(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.media_receiver.try_recv() {
            match result {
                Ok(response) => {
                    let accepted = self
                        .media_state
                        .as_mut()
                        .expect("media worker is running")
                        .update_resized_protocol(response);
                    if accepted {
                        self.media_error = None;
                    }
                    changed |= accepted;
                }
                Err(error) => {
                    self.media_error = Some(format!("Could not render media preview: {error}"));
                    changed = true;
                }
            }
        }
        changed
    }

    pub(crate) fn media_error(&self) -> Option<&str> {
        self.media_error.as_deref()
    }

    pub(crate) fn queue_kitty_frame(
        &mut self,
        generation: u64,
        area: Rect,
        transmission: Option<KittyTransmission>,
    ) {
        if let Some(active) = self.active_inline_image.take() {
            append_clear_area(&mut self.pending_terminal_cleanup, active.area);
        }
        if let Some(transmission) = transmission {
            self.pending_terminal_cleanup
                .extend_from_slice(KITTY_DELETE_ALL.as_bytes());
            append_positioned_output(
                &mut self.pending_terminal_output.bytes,
                area,
                &transmission.bytes,
            );
            self.pending_terminal_output.kitty = true;
            self.active_kitty_image = Some(ActiveKittyImage {
                generation,
                image_id: transmission.image_id,
                area,
            });
            return;
        }

        let Some(active) = self.active_kitty_image.as_mut() else {
            return;
        };
        if active.generation != generation {
            self.hide_media();
            return;
        }
        if active.area == area {
            return;
        }
        self.pending_terminal_cleanup
            .extend_from_slice(KITTY_DELETE_PLACEMENTS.as_bytes());
        let placement = format!(
            "\u{1b}_Ga=p,i={},c={},r={},C=1,q=2\u{1b}\\",
            active.image_id, area.width, area.height
        );
        append_positioned_output(
            &mut self.pending_terminal_output.bytes,
            area,
            placement.as_bytes(),
        );
        self.pending_terminal_output.kitty = true;
        active.area = area;
    }

    pub(crate) fn queue_inline_frame(
        &mut self,
        generation: u64,
        protocol: MediaPreviewProtocol,
        area: Rect,
        transmission: Option<Vec<u8>>,
    ) {
        let Some(transmission) = transmission else {
            return;
        };
        let next = ActiveInlineImage {
            generation,
            protocol,
            area,
        };
        if self.active_inline_image == Some(next) {
            return;
        }
        if let Some(active) = self.active_inline_image.take() {
            append_clear_area(&mut self.pending_terminal_cleanup, active.area);
        }
        if self.active_kitty_image.take().is_some() {
            self.pending_terminal_cleanup
                .extend_from_slice(KITTY_DELETE_ALL.as_bytes());
        }
        append_positioned_output(&mut self.pending_terminal_output.bytes, area, &transmission);
        self.active_inline_image = Some(next);
    }

    pub(crate) fn take_terminal_cleanup(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_terminal_cleanup)
    }

    pub(crate) fn take_terminal_output(&mut self) -> MediaTerminalOutput {
        std::mem::take(&mut self.pending_terminal_output)
    }

    pub(crate) fn terminal_restarted(&mut self) {
        self.pending_terminal_output = MediaTerminalOutput::default();
        self.pending_terminal_cleanup.clear();
        self.active_kitty_image = None;
        self.active_inline_image = None;
        if self.media_generation.take().is_some() {
            self.media_state
                .as_mut()
                .expect("media worker is running")
                .empty_protocol();
        }
        self.media_protocol = None;
        self.media_size = Size::default();
        self.media_error = None;
    }

    pub(crate) fn configure_media_picker(&mut self, picker: Picker, allow_auto_kitty: bool) {
        self.terminal_restarted();
        self.media_picker = picker;
        self.allow_auto_kitty = allow_auto_kitty;
    }

    fn effective_protocol(&self, requested: MediaPreviewProtocol) -> MediaPreviewProtocol {
        if requested != MediaPreviewProtocol::Auto {
            return requested;
        }
        match self.media_picker.protocol_type() {
            ProtocolType::Kitty if self.allow_auto_kitty => MediaPreviewProtocol::Kitty,
            ProtocolType::Iterm2 => MediaPreviewProtocol::Iterm2,
            ProtocolType::Sixel => MediaPreviewProtocol::Sixel,
            ProtocolType::Halfblocks | ProtocolType::Kitty => MediaPreviewProtocol::Halfblocks,
        }
    }

    pub(crate) fn media_state(
        &mut self,
        generation: u64,
        image: &Arc<DynamicImage>,
        protocol: MediaPreviewProtocol,
        available: Rect,
    ) -> (Rect, MediaPreviewProtocol, &mut ThreadProtocol) {
        let effective_protocol = self.effective_protocol(protocol);
        if self.media_generation != Some(generation) || self.media_protocol != Some(protocol) {
            let mut picker = self.media_picker.clone();
            let state = if effective_protocol == MediaPreviewProtocol::Kitty {
                let image_id = ((generation % 99_999) + 1) as u32;
                StatefulProtocol::new(
                    (**image).clone(),
                    picker.font_size(),
                    None,
                    StatefulProtocolType::Kitty(StatefulKitty::new(image_id, false)),
                )
            } else {
                picker.set_protocol_type(match effective_protocol {
                    MediaPreviewProtocol::Auto | MediaPreviewProtocol::Halfblocks => {
                        ProtocolType::Halfblocks
                    }
                    MediaPreviewProtocol::Kitty => ProtocolType::Kitty,
                    MediaPreviewProtocol::Iterm2 => ProtocolType::Iterm2,
                    MediaPreviewProtocol::Sixel => ProtocolType::Sixel,
                });
                picker.new_resize_protocol((**image).clone())
            };
            self.media_state
                .as_mut()
                .expect("media worker is running")
                .replace_protocol(state);
            self.media_generation = Some(generation);
            self.media_protocol = Some(protocol);
            self.effective_media_protocol = effective_protocol;
            self.media_size = Size::default();
            self.media_error = None;
        }
        if let Some(size) = self
            .media_state
            .as_ref()
            .expect("media worker is running")
            .size_for(Resize::Fit(None), available.into())
        {
            self.media_size = size;
        }
        let width = self.media_size.width.min(available.width);
        let height = self.media_size.height.min(available.height);
        let area = Rect::new(
            available
                .x
                .saturating_add(available.width.saturating_sub(width) / 2),
            available
                .y
                .saturating_add(available.height.saturating_sub(height) / 2),
            width,
            height,
        );
        (
            area,
            self.effective_media_protocol,
            self.media_state.as_mut().expect("media worker is running"),
        )
    }

    pub(crate) fn shutdown(&mut self) {
        self.media_state.take();
        if let Some(worker) = self.media_worker.take() {
            let _ = worker.join();
        }
    }

    pub(crate) fn prepare(
        &mut self,
        input: PreviewInput<'_>,
        scroll: &mut usize,
    ) -> PreparedPreview {
        let raw = input.content.as_str();
        let is_diff = input.content.is_diff();
        let render_markdown = input.markdown && raw.len() <= MAX_CACHED_PREVIEW_BYTES;
        let cache_matches = self.cache.as_ref().is_some_and(|cache| {
            let markdown_wrapped = render_markdown && input.wrapped;
            cache.generation == input.generation
                && cache.path == input.path
                && cache.is_diff == is_diff
                && cache.markdown == render_markdown
                && cache.markdown_wrapped == markdown_wrapped
                && cache.show_initial_diff_header == input.show_initial_diff_header
                && cache.width == input.width
        });
        if !cache_matches {
            let (display_count, fully_styled, lines) = if render_markdown {
                let content_width = markdown_content_width(input.width);
                let lines = numbered_markdown_lines(
                    styled_markdown(raw, content_width, input.wrapped),
                    input.width,
                );
                (lines.len(), true, lines)
            } else {
                let display_count = match input.content {
                    PreviewContent::Diff(document) => {
                        document.display_len(input.show_initial_diff_header)
                    }
                    PreviewContent::Source(source) => source.lines().count(),
                };
                let fully_styled = display_count <= MAX_CACHED_PREVIEW_LINES
                    && raw.len() <= MAX_CACHED_PREVIEW_BYTES;
                let lines = if fully_styled {
                    if let PreviewContent::Diff(document) = input.content {
                        styled_diff(
                            document,
                            input.path,
                            input.width,
                            input.show_initial_diff_header,
                        )
                    } else {
                        styled_source(raw, input.path, input.width)
                    }
                } else {
                    Vec::new()
                };
                (display_count, fully_styled, lines)
            };
            self.cache = Some(PreviewCache {
                generation: input.generation,
                path: input.path.to_owned(),
                is_diff,
                markdown: render_markdown,
                markdown_wrapped: render_markdown && input.wrapped,
                show_initial_diff_header: input.show_initial_diff_header,
                width: input.width,
                lines,
                fully_styled,
                window_start: 0,
                display_count,
                wrapped_line_starts: None,
                wrapped_window: None,
                unwrapped_hunks: None,
                wrapped_hunks: None,
            });
        }

        if input.wrapped {
            if self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.wrapped_line_starts.is_none())
            {
                let starts = if render_markdown {
                    wrapped_styled_line_starts(
                        &self
                            .cache
                            .as_ref()
                            .expect("preview cache was initialized")
                            .lines,
                        input.width,
                    )
                } else {
                    match input.content {
                        PreviewContent::Diff(document) => document
                            .wrapped_line_starts(input.width, input.show_initial_diff_header),
                        PreviewContent::Source(source) => {
                            wrapped_source_line_starts(source, input.width)
                        }
                    }
                };
                self.cache
                    .as_mut()
                    .expect("preview cache was initialized")
                    .wrapped_line_starts = Some(starts);
            }
            let starts = self
                .cache
                .as_ref()
                .and_then(|cache| cache.wrapped_line_starts.as_deref())
                .expect("wrapped line starts were initialized");
            let display_count = starts.len().saturating_sub(1);
            let rendered_height = starts.last().copied().unwrap_or(0);
            let scroll_limit = rendered_height.saturating_sub(1);
            *scroll = (*scroll).min(scroll_limit);
            let first = starts
                .partition_point(|start| *start <= *scroll)
                .saturating_sub(1)
                .min(display_count);
            let visible_end = scroll.saturating_add(input.viewport_height);
            let end = starts
                .partition_point(|start| *start < visible_end)
                .max(first.saturating_add(1))
                .min(display_count);
            let local_scroll = scroll.saturating_sub(starts[first]);
            if let Some(window) = self
                .cache
                .as_ref()
                .and_then(|cache| cache.wrapped_window.as_ref())
                .filter(|window| {
                    window.first == first
                        && window.end == end
                        && window.local_scroll == local_scroll
                        && window.viewport_height == input.viewport_height
                })
            {
                return PreparedPreview {
                    lines: window.lines.clone(),
                    rendered_height,
                    wrapped: true,
                };
            }
            let logical_lines = self.line_window(
                &input,
                first,
                end.saturating_sub(first),
                input.viewport_height,
            );
            let lines = hard_wrap_lines(
                logical_lines,
                input.width,
                local_scroll,
                input.viewport_height,
                is_diff,
                render_markdown,
            );
            self.cache
                .as_mut()
                .expect("preview cache was initialized")
                .wrapped_window = Some(WrappedWindow {
                first,
                end,
                local_scroll,
                viewport_height: input.viewport_height,
                lines: lines.clone(),
            });
            return PreparedPreview {
                lines,
                rendered_height,
                wrapped: true,
            };
        }

        let height = self
            .cache
            .as_ref()
            .expect("preview cache was initialized")
            .display_count;
        let max_scroll = height.saturating_sub(1);
        *scroll = (*scroll).min(max_scroll);
        let lines = self.line_window(
            &input,
            *scroll,
            input.viewport_height,
            input.viewport_height,
        );
        PreparedPreview {
            lines,
            rendered_height: height,
            wrapped: false,
        }
    }

    pub(crate) fn hunk_rows(
        &mut self,
        content: &DiffDocument,
        wrapped: bool,
    ) -> (Vec<(usize, usize)>, usize) {
        if let Some(cache) = &self.cache {
            let cached = if wrapped {
                &cache.wrapped_hunks
            } else {
                &cache.unwrapped_hunks
            };
            if let Some(cached) = cached {
                return cached.clone();
            }
        }
        let cache = self.cache.as_ref();
        let rendered = content.hunk_rows(
            cache.and_then(|cache| cache.wrapped_line_starts.as_deref()),
            wrapped,
            cache.is_some_and(|cache| cache.show_initial_diff_header),
        );
        if let Some(cache) = &mut self.cache {
            if wrapped {
                cache.wrapped_hunks = Some(rendered.clone());
            } else {
                cache.unwrapped_hunks = Some(rendered.clone());
            }
        }
        rendered
    }

    fn line_window(
        &mut self,
        input: &PreviewInput<'_>,
        start: usize,
        count: usize,
        viewport_height: usize,
    ) -> Vec<Line<'static>> {
        let cache = self.cache.as_ref().expect("preview cache was initialized");
        if cache.fully_styled {
            return cache
                .lines
                .iter()
                .skip(start)
                .take(count)
                .cloned()
                .collect();
        }
        let cached_end = cache.window_start.saturating_add(cache.lines.len());
        if start < cache.window_start || start.saturating_add(count) > cached_end {
            let margin = viewport_height.saturating_mul(4).max(256);
            let window_start = start.saturating_sub(margin);
            let window_count = count.saturating_add(margin.saturating_mul(2));
            let lines = if let PreviewContent::Diff(document) = input.content {
                styled_diff_window(
                    document,
                    input.path,
                    input.width,
                    window_start,
                    window_count,
                    input.show_initial_diff_header,
                )
            } else {
                styled_source_window(
                    input.content.as_str(),
                    input.path,
                    input.width,
                    window_start,
                    window_count,
                )
            };
            let cache = self.cache.as_mut().expect("preview cache was initialized");
            cache.window_start = window_start;
            cache.lines = lines;
        }
        let cache = self.cache.as_ref().expect("preview cache was initialized");
        cache
            .lines
            .iter()
            .skip(start.saturating_sub(cache.window_start))
            .take(count)
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn is_windowed(&self) -> bool {
        self.cache
            .as_ref()
            .is_some_and(|cache| !cache.fully_styled && !cache.lines.is_empty())
    }
}

impl Drop for PreviewPresentation {
    fn drop(&mut self) {
        self.shutdown();
    }
}
