use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};

use image::DynamicImage;
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Rect, Size},
    style::Style,
    text::{Line, Span},
};
use ratatui_image::{
    Resize,
    errors::Errors as ImageError,
    picker::{Picker, ProtocolType},
    protocol::{StatefulProtocol, StatefulProtocolType, kitty::StatefulKitty},
    thread::{ResizeRequest, ResizeResponse, ThreadProtocol},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::media::MediaPreviewProtocol;

use super::text::{
    diff_display_line_count, markdown_prefix_style, styled_diff, styled_diff_window,
    styled_markdown, styled_source, styled_source_window, wrapped_preview_line_starts,
};

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

pub(crate) struct PreviewInput<'a> {
    pub(crate) content: &'a str,
    pub(crate) generation: u64,
    pub(crate) path: &'a str,
    pub(crate) is_diff: bool,
    pub(crate) markdown: bool,
    pub(crate) show_initial_diff_header: bool,
    pub(crate) width: usize,
    pub(crate) viewport_height: usize,
    pub(crate) wrapped: bool,
    pub(crate) hunk_selected: bool,
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
        diff: &str,
        row: usize,
        column: usize,
        gutter: usize,
    ) -> Option<(usize, usize)> {
        let cache = self.cache.as_ref()?;
        let (display_line, wrapped_row) = self.display_position_at_rendered_row(row)?;
        let (source_line, payload) =
            super::text::diff_new_line_and_payload_at_display_row(diff, display_line, false)?;
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
        let render_markdown = input.markdown && input.content.len() <= MAX_CACHED_PREVIEW_BYTES;
        let cache_matches = self.cache.as_ref().is_some_and(|cache| {
            let markdown_wrapped = render_markdown && input.wrapped;
            cache.generation == input.generation
                && cache.path == input.path
                && cache.is_diff == input.is_diff
                && cache.markdown == render_markdown
                && cache.markdown_wrapped == markdown_wrapped
                && cache.show_initial_diff_header == input.show_initial_diff_header
                && cache.width == input.width
        });
        if !cache_matches {
            let (display_count, fully_styled, lines) = if render_markdown {
                let content_width = markdown_content_width(input.width);
                let lines = numbered_markdown_lines(
                    styled_markdown(input.content, content_width, input.wrapped),
                    input.width,
                );
                (lines.len(), true, lines)
            } else {
                let display_count = if input.is_diff {
                    diff_display_line_count(input.content, input.show_initial_diff_header)
                } else {
                    input.content.lines().count()
                };
                let fully_styled = display_count <= MAX_CACHED_PREVIEW_LINES
                    && input.content.len() <= MAX_CACHED_PREVIEW_BYTES;
                let lines = if fully_styled {
                    if input.is_diff {
                        styled_diff(
                            input.content,
                            input.path,
                            input.width,
                            input.show_initial_diff_header,
                        )
                    } else {
                        styled_source(input.content, input.path, input.width)
                    }
                } else {
                    Vec::new()
                };
                (display_count, fully_styled, lines)
            };
            self.cache = Some(PreviewCache {
                generation: input.generation,
                path: input.path.to_owned(),
                is_diff: input.is_diff,
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
                    wrapped_preview_line_starts(
                        input.content,
                        input.is_diff,
                        input.width,
                        input.show_initial_diff_header,
                    )
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
            let scroll_limit = if input.hunk_selected {
                rendered_height.saturating_sub(1)
            } else {
                rendered_height.saturating_sub(input.viewport_height)
            };
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
                input.is_diff,
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
        let max_scroll = if input.is_diff && input.hunk_selected {
            height.saturating_sub(1)
        } else {
            height.saturating_sub(input.viewport_height)
        };
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
        content: &str,
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
        let rendered = rendered_hunk_rows(
            content,
            self.cache
                .as_ref()
                .and_then(|cache| cache.wrapped_line_starts.as_deref()),
            wrapped,
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
            let lines = if input.is_diff {
                styled_diff_window(
                    input.content,
                    input.path,
                    input.width,
                    window_start,
                    window_count,
                    input.show_initial_diff_header,
                )
            } else {
                styled_source_window(
                    input.content,
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

fn append_positioned_output(output: &mut Vec<u8>, area: Rect, command: &[u8]) {
    output.extend_from_slice(b"\x1b[s");
    output.extend_from_slice(format!("\x1b[{};{}H", area.y + 1, area.x + 1).as_bytes());
    output.extend_from_slice(command);
    output.extend_from_slice(b"\x1b[u");
}

fn append_clear_area(output: &mut Vec<u8>, area: Rect) {
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

fn wrapped_styled_line_starts(lines: &[Line<'static>], width: usize) -> Vec<usize> {
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

fn markdown_content_width(width: usize) -> usize {
    if width >= MIN_NUMBERED_MARKDOWN_WIDTH {
        width.saturating_sub(MARKDOWN_LINE_GUTTER_WIDTH).max(1)
    } else {
        width.max(1)
    }
}

fn numbered_markdown_lines(mut lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width < MIN_NUMBERED_MARKDOWN_WIDTH {
        return lines;
    }
    for (index, line) in lines.iter_mut().enumerate() {
        line.spans.insert(
            0,
            Span::styled(
                format!("{:>5}  ", index.saturating_add(1)),
                Style::default().fg(super::palette().faint),
            ),
        );
    }
    lines
}

struct StyledChunk {
    content: String,
    style: Style,
    width: usize,
}

type WrapToken = (bool, Vec<StyledChunk>);

fn hard_wrap_lines(
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
fn append_wrap_token(
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

fn emit_wrapped_row(
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

fn start_continuation(
    output_spans: &mut Vec<Span<'static>>,
    output_width: &mut usize,
    gutter: &WrapGutter,
) {
    output_spans.extend(gutter.continuation.iter().cloned());
    *output_width = gutter.width;
}

#[derive(Default)]
struct WrapGutter {
    width: usize,
    span_count: usize,
    continuation: Vec<Span<'static>>,
}

fn line_gutter(line: &Line<'_>, width: usize, is_diff: bool, markdown: bool) -> WrapGutter {
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
            if UnicodeWidthStr::width(number.content.as_ref()) == 5 && marker(marker_span) =>
        {
            (6, 2)
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

fn markdown_continuation_prefix(prefix: &str) -> String {
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

fn rendered_hunk_rows(
    diff: &str,
    wrapped_line_starts: Option<&[usize]>,
    wrapped: bool,
) -> (Vec<(usize, usize)>, usize) {
    let mut rendered_row: usize = 0;
    let mut styled_index = 0;
    let mut hunk_index = 0;
    let mut rows = Vec::new();
    let has_hunks = diff.lines().any(|line| line.starts_with("@@"));
    let mut in_hunk = false;

    for line in diff.lines() {
        let hunk_header = line.starts_with("@@");
        if has_hunks && !in_hunk && !hunk_header {
            continue;
        }
        if hunk_header {
            if hunk_index > 0 {
                if wrapped {
                    let Some(line_height) = wrapped_line_starts.and_then(|starts| {
                        Some(starts.get(styled_index + 1)? - starts.get(styled_index)?)
                    }) else {
                        break;
                    };
                    rendered_row = rendered_row.saturating_add(line_height);
                    styled_index += 1;
                } else {
                    rendered_row += 1;
                }
            }
            in_hunk = true;
            rows.push((hunk_index, rendered_row));
            hunk_index += 1;
        }
        if wrapped {
            let Some(line_height) = wrapped_line_starts
                .and_then(|starts| Some(starts.get(styled_index + 1)? - starts.get(styled_index)?))
            else {
                break;
            };
            rendered_row = rendered_row.saturating_add(line_height);
            styled_index += 1;
        } else {
            rendered_row += 1;
        }
    }
    (rows, rendered_row)
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgba};
    use ratatui_image::ResizeEncodeRender;

    use super::*;

    #[test]
    fn shutdown_joins_the_media_worker_once() {
        let mut preview = PreviewPresentation::default();
        preview.shutdown();
        preview.shutdown();
        assert!(preview.media_worker.is_none());
        assert!(preview.media_state.is_none());
    }

    #[test]
    fn extracts_superfile_style_kitty_transmission_after_placeholders() {
        let command =
            "\u{1b}_Gq=2,i=42,a=T,U=1,f=32,t=d,s=80,v=48,m=0;data\u{1b}\\\u{1b}[splaceholders";
        let area = Rect::new(2, 3, 10, 3);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 10));
        buffer.cell_mut((2, 3)).unwrap().set_symbol(command);
        buffer.cell_mut((3, 3)).unwrap().set_symbol("placeholder");

        let transmission = take_kitty_transmission(&mut buffer, area).unwrap();

        let patched = String::from_utf8(transmission.bytes).unwrap();
        assert_eq!(transmission.image_id, 42);
        assert!(patched.contains("i=42,a=T,U=1,c=10,r=3,f=32,"));
        assert!(!patched.contains("\u{10eeee}"));
        assert_eq!(
            buffer.cell((2, 3)).unwrap().symbol(),
            "\u{1b}[splaceholders"
        );
        assert_eq!(buffer.cell((3, 3)).unwrap().symbol(), "placeholder");

        buffer
            .cell_mut((2, 3))
            .unwrap()
            .set_symbol("\u{1b}[s\u{10eeee}placeholder");
        buffer.cell_mut((3, 3)).unwrap().set_symbol("placeholder");
        assert!(take_kitty_transmission(&mut buffer, area).is_none());
        assert_eq!(
            buffer.cell((2, 3)).unwrap().symbol(),
            "\u{1b}[s\u{10eeee}placeholder"
        );
        assert_eq!(buffer.cell((3, 3)).unwrap().symbol(), "placeholder");
    }

    #[test]
    fn queues_kitty_output_outside_the_ratatui_buffer() {
        let mut preview = PreviewPresentation::default();
        let area = Rect::new(2, 3, 10, 4);
        preview.queue_kitty_frame(
            7,
            area,
            Some(KittyTransmission {
                image_id: 42,
                bytes: b"\x1b_Gq=2,i=42,a=T,U=1,c=10,r=4;data\x1b\\".to_vec(),
            }),
        );

        let output = preview.take_terminal_output();
        assert!(output.kitty);
        let output = String::from_utf8(output.bytes).unwrap();
        assert_eq!(preview.take_terminal_cleanup(), KITTY_DELETE_ALL.as_bytes());
        assert!(output.contains("\x1b[s\x1b[4;3H"));
        assert!(output.contains("i=42,a=T,U=1,c=10,r=4"));
        assert!(output.ends_with("\x1b[u"));

        preview.queue_kitty_frame(7, Rect::new(4, 5, 10, 4), None);
        let reposition = String::from_utf8(preview.take_terminal_output().bytes).unwrap();
        assert_eq!(
            preview.take_terminal_cleanup(),
            KITTY_DELETE_PLACEMENTS.as_bytes()
        );
        assert!(reposition.contains("\x1b[s\x1b[6;5H"));
        assert!(reposition.contains("a=p,i=42,c=10,r=4,C=1,q=2"));

        preview.hide_media();
        assert_eq!(preview.take_terminal_cleanup(), KITTY_DELETE_ALL.as_bytes());
    }

    #[test]
    fn extracts_inline_protocols_for_out_of_band_output() {
        let area = Rect::new(2, 3, 3, 2);
        for (protocol, payload) in [
            (
                MediaPreviewProtocol::Iterm2,
                "clear\u{1b}]1337;File=inline=1:data\u{7}",
            ),
            (MediaPreviewProtocol::Sixel, "clear\u{1b}Pqdata\u{1b}\\"),
        ] {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 10));
            buffer.cell_mut((2, 3)).unwrap().set_symbol(payload);
            buffer.cell_mut((3, 3)).unwrap().set_symbol("covered");

            let extracted = take_inline_transmission(&mut buffer, area, protocol).unwrap();

            assert_eq!(extracted, payload.as_bytes());
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    let cell = buffer.cell((x, y)).unwrap();
                    assert_eq!(cell.symbol(), " ");
                    assert_eq!(cell.diff_option, CellDiffOption::Skip);
                }
            }
        }
    }

    #[test]
    fn real_inline_encoders_produce_extractable_terminal_payloads() {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255])));
        for protocol in [MediaPreviewProtocol::Iterm2, MediaPreviewProtocol::Sixel] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(match protocol {
                MediaPreviewProtocol::Iterm2 => ProtocolType::Iterm2,
                MediaPreviewProtocol::Sixel => ProtocolType::Sixel,
                _ => unreachable!(),
            });
            let mut state = picker.new_resize_protocol(image.clone());
            state.resize_encode(&Resize::Fit(None), Size::new(2, 1));
            state.last_encoding_result().unwrap().unwrap();
            let payload = match state.protocol_type() {
                StatefulProtocolType::ITerm2(encoded) => &encoded.data,
                StatefulProtocolType::Sixel(encoded) => &encoded.data,
                _ => unreachable!(),
            };
            let area = Rect::new(1, 1, 2, 1);
            let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 5));
            buffer.cell_mut((1, 1)).unwrap().set_symbol(payload);
            assert!(take_inline_transmission(&mut buffer, area, protocol).is_some());
        }
    }

    #[test]
    fn queues_inline_output_only_when_placement_changes() {
        let mut preview = PreviewPresentation::default();
        let area = Rect::new(2, 3, 10, 4);
        preview.queue_inline_frame(
            7,
            MediaPreviewProtocol::Iterm2,
            area,
            Some(b"inline-image".to_vec()),
        );
        let output = preview.take_terminal_output();
        assert!(!output.kitty);
        let output = String::from_utf8(output.bytes).unwrap();
        assert!(output.contains("\u{1b}[s\u{1b}[4;3Hinline-image\u{1b}[u"));

        preview.queue_inline_frame(
            7,
            MediaPreviewProtocol::Iterm2,
            area,
            Some(b"inline-image".to_vec()),
        );
        assert!(preview.take_terminal_output().bytes.is_empty());

        preview.queue_inline_frame(
            7,
            MediaPreviewProtocol::Iterm2,
            Rect::new(4, 5, 10, 4),
            Some(b"inline-image".to_vec()),
        );
        let cleanup = String::from_utf8(preview.take_terminal_cleanup()).unwrap();
        assert!(cleanup.starts_with("\u{1b}[s\u{1b}[4;3H\u{1b}[10X"));
        assert!(cleanup.ends_with("\u{1b}[u"));
        assert!(!preview.take_terminal_output().bytes.is_empty());

        preview.hide_media();
        let cleanup = String::from_utf8(preview.take_terminal_cleanup()).unwrap();
        assert!(cleanup.starts_with("\u{1b}[s\u{1b}[6;5H\u{1b}[10X"));
        assert!(cleanup.ends_with("\u{1b}[u"));
    }

    #[test]
    fn auto_uses_detected_protocols_but_requires_a_known_kitty_terminal() {
        let mut preview = PreviewPresentation::default();
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Iterm2);
        preview.configure_media_picker(picker, false);
        assert_eq!(
            preview.effective_protocol(MediaPreviewProtocol::Auto),
            MediaPreviewProtocol::Iterm2
        );

        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        preview.configure_media_picker(picker.clone(), false);
        assert_eq!(
            preview.effective_protocol(MediaPreviewProtocol::Auto),
            MediaPreviewProtocol::Halfblocks
        );
        preview.configure_media_picker(picker, true);
        assert_eq!(
            preview.effective_protocol(MediaPreviewProtocol::Auto),
            MediaPreviewProtocol::Kitty
        );
    }

    #[test]
    fn wrapped_source_continuations_stay_after_the_line_number_gutter() {
        let lines = vec![Line::from(vec![
            Span::raw("    1  "),
            Span::raw("abcdefghijklmnop"),
        ])];

        let wrapped = hard_wrap_lines(lines, 12, 0, 10, false, false);

        assert_eq!(wrapped.len(), 4);
        assert!(wrapped[0].spans[0].content.starts_with("    1  "));
        assert!(
            wrapped[1..]
                .iter()
                .all(|line| line.spans[0].content.starts_with("       "))
        );

        let lines = vec![Line::from(vec![
            Span::raw("    1  "),
            Span::raw("word committing"),
        ])];
        let wrapped = hard_wrap_lines(lines, 18, 0, 10, false, false);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(wrapped[1].spans[0].content, "       committing");
    }

    #[test]
    fn measures_wrapped_markdown_without_unbounded_allocation() {
        let mut presentation = PreviewPresentation::default();
        let mut scroll = 0;

        let preview = presentation.prepare(
            PreviewInput {
                content: "# Heading\n\nA paragraph that wraps across multiple rows.",
                generation: 1,
                path: "README.md",
                is_diff: false,
                markdown: true,
                show_initial_diff_header: false,
                width: 16,
                viewport_height: 8,
                wrapped: true,
                hunk_selected: false,
            },
            &mut scroll,
        );

        assert!(preview.wrapped);
        assert!(preview.rendered_height > 3);
        assert!(!preview.lines.is_empty());
    }

    #[test]
    fn maps_wrapped_preview_cells_to_exact_source_positions() {
        let mut presentation = PreviewPresentation::default();
        let mut scroll = 0;
        presentation.prepare(
            PreviewInput {
                content: "alpha beta gamma",
                generation: 1,
                path: "notes.txt",
                is_diff: false,
                markdown: false,
                show_initial_diff_header: false,
                width: 10,
                viewport_height: 4,
                wrapped: true,
                hunk_selected: false,
            },
            &mut scroll,
        );

        assert_eq!(
            presentation.source_position_at_rendered_position("alpha beta gamma", 1, 3, 0,),
            Some((1, 14))
        );

        let diff = "@@ -1 +1 @@\n+alpha beta gamma";
        presentation.prepare(
            PreviewInput {
                content: diff,
                generation: 2,
                path: "notes.txt",
                is_diff: true,
                markdown: false,
                show_initial_diff_header: false,
                width: 11,
                viewport_height: 4,
                wrapped: true,
                hunk_selected: false,
            },
            &mut scroll,
        );
        assert_eq!(
            presentation.diff_position_at_rendered_position(diff, 2, 4, 1),
            Some((1, 14))
        );
    }

    #[test]
    fn oversized_markdown_uses_the_windowed_source_cache() {
        let content = "x".repeat(MAX_CACHED_PREVIEW_BYTES + 1);
        let mut presentation = PreviewPresentation::default();
        let mut scroll = 0;

        presentation.prepare(
            PreviewInput {
                content: &content,
                generation: 1,
                path: "README.md",
                is_diff: false,
                markdown: true,
                show_initial_diff_header: false,
                width: 80,
                viewport_height: 8,
                wrapped: false,
                hunk_selected: false,
            },
            &mut scroll,
        );

        let cache = presentation.cache.as_ref().unwrap();
        assert!(!cache.markdown);
        assert!(!cache.fully_styled);
    }

    #[test]
    fn numbers_markdown_rows_and_leaves_wrapped_continuations_blank() {
        let lines = numbered_markdown_lines(
            styled_markdown(
                "- This list item contains enough words to wrap across rows.\n",
                markdown_content_width(24),
                false,
            ),
            24,
        );
        let wrapped = hard_wrap_lines(lines, 24, 0, 20, false, true)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            wrapped
                .first()
                .is_some_and(|line| line.starts_with("    1  * "))
        );
        assert!(
            wrapped[1..]
                .iter()
                .all(|line| line.starts_with("         ")),
            "{wrapped:#?}"
        );
    }

    #[test]
    fn wrapped_markdown_uses_hanging_list_and_quote_prefixes() {
        let lines = styled_markdown(
            "- This list item contains enough words to wrap.\n\n> This quote also contains enough words to wrap.\n",
            80,
            false,
        );
        let wrapped = hard_wrap_lines(lines, 18, 0, 20, false, true)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(wrapped.first().is_some_and(|line| line.starts_with("* ")));
        assert!(wrapped.get(1).is_some_and(|line| line.starts_with("  ")));
        let quote = wrapped
            .iter()
            .position(|line| line.starts_with("> This"))
            .expect("quote should be rendered");
        assert!(
            wrapped
                .get(quote + 1)
                .is_some_and(|line| line.starts_with("> "))
        );
    }

    #[test]
    fn markdown_table_cache_tracks_wrap_mode() {
        let content = "| Key | Description |\n| --- | --- |\n| alpha | beginning words continue across rows until TAIL |\n";
        let mut presentation = PreviewPresentation::default();
        let mut scroll = 0;
        let mut prepare = |presentation: &mut PreviewPresentation, wrapped| {
            presentation.prepare(
                PreviewInput {
                    content,
                    generation: 1,
                    path: "README.md",
                    is_diff: false,
                    markdown: true,
                    show_initial_diff_header: false,
                    width: 30,
                    viewport_height: 30,
                    wrapped,
                    hunk_selected: false,
                },
                &mut scroll,
            )
        };
        let contains_tail = |preview: &PreparedPreview| {
            preview
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content.contains("TAIL"))
        };

        let unwrapped = prepare(&mut presentation, false);
        assert!(!contains_tail(&unwrapped));

        let wrapped = prepare(&mut presentation, true);
        assert!(contains_tail(&wrapped));
        assert!(wrapped.rendered_height > unwrapped.rendered_height);
        assert!(wrapped.lines.iter().all(|line| {
            line.spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>()
                <= 30
        }));
        assert!(
            wrapped
                .lines
                .first()
                .and_then(|line| line.spans.first())
                .is_some_and(|span| span.content.starts_with("    1  "))
        );

        let unwrapped_again = prepare(&mut presentation, false);
        assert!(!contains_tail(&unwrapped_again));
    }
}
