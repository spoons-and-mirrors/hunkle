pub(super) use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
pub(super) use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Position, Rect, Size},
    style::Style,
    text::{Line, Span},
};
pub(super) use ratatui_image::{
    FontSize, Resize, ResizeEncodeRender,
    errors::Errors as ImageError,
    picker::{Picker, ProtocolType},
    protocol::{
        Protocol, StatefulProtocol, StatefulProtocolType, halfblocks::Halfblocks,
        kitty::StatefulKitty, sixel::Sixel,
    },
};
pub(super) use unicode_segmentation::UnicodeSegmentation;
pub(super) use unicode_width::UnicodeWidthStr;

pub(super) use crate::{
    media::{MediaPreviewProtocol, SixelQuality},
    repo_path::RepoPath,
};

pub(super) use super::text::{
    markdown_prefix_style, styled_diff, styled_diff_window, styled_editor_source_window_from,
    styled_markdown, styled_source, styled_source_window_from, wrapped_source_line_starts,
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
const SOURCE_LINE_CHECKPOINT_STRIDE: usize = 256;
const MEDIA_ZOOM_STEP: f64 = 1.25;
const MAX_MEDIA_ZOOM_LEVEL: u8 = 10;
const MEDIA_INTERACTION_SETTLE: Duration = Duration::from_millis(120);
const FAST_SIXEL_PREVIEW_GRACE: Duration = Duration::from_millis(10);
const MAX_CACHED_SIXEL_FRAMES: usize = 32;
const MAX_CACHED_SIXEL_BYTES: usize = 48 * 1024 * 1024;
const PREVIEW_SIXEL_MAX_COLORS: u16 = 64;
const FINAL_SIXEL_MAX_COLORS: u16 = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SixelEncoding {
    Preview,
    Fast,
    Quality,
}

impl SixelEncoding {
    fn options(self) -> icy_sixel::EncodeOptions {
        icy_sixel::EncodeOptions {
            max_colors: match self {
                Self::Preview => PREVIEW_SIXEL_MAX_COLORS,
                Self::Fast | Self::Quality => FINAL_SIXEL_MAX_COLORS,
            },
            diffusion: if self == Self::Quality { 0.875 } else { 0.0 },
            quantize_method: icy_sixel::QuantizeMethod::Wu,
        }
    }

    fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Preview => "sixel-preview",
            Self::Fast => "sixel-fast-final",
            Self::Quality => "sixel-quality-final",
        }
    }

    fn is_final(self) -> bool {
        self != Self::Preview
    }
}

fn media_background_color() -> Rgba<u8> {
    let ratatui::style::Color::Rgb(red, green, blue) = super::palette().panel else {
        unreachable!("theme colors are resolved to RGB")
    };
    Rgba([red, green, blue, 255])
}

fn interaction_preview(
    image: &DynamicImage,
    crop_x: u32,
    crop_y: u32,
    crop_width: u32,
    crop_height: u32,
    size: Size,
) -> Result<Protocol, ImageError> {
    let width = u32::from(size.width);
    let height = u32::from(size.height) * 2;
    let sampled = RgbaImage::from_fn(width, height, |x, y| {
        let source_x = crop_x + sample_coordinate(crop_width, x, width);
        let source_y = crop_y + sample_coordinate(crop_height, y, height);
        image.get_pixel(source_x, source_y)
    });
    Halfblocks::new(DynamicImage::ImageRgba8(sampled), size).map(Protocol::Halfblocks)
}

fn sample_coordinate(extent: u32, index: u32, samples: u32) -> u32 {
    let centered = (u64::from(index) * 2 + 1) * u64::from(extent);
    ((centered / (u64::from(samples) * 2)) as u32).min(extent - 1)
}

fn sixel_cache_key(
    image: &DynamicImage,
    size: Size,
    font_size: FontSize,
    is_tmux: bool,
    encoding: SixelEncoding,
) -> SixelCacheKey {
    let mut hasher = DefaultHasher::new();
    image.as_bytes().hash(&mut hasher);
    SixelCacheKey {
        source_hash: hasher.finish(),
        source_width: image.width(),
        source_height: image.height(),
        cell_width: size.width,
        cell_height: size.height,
        font_width: font_size.width,
        font_height: font_size.height,
        is_tmux,
        encoding,
    }
}

fn encode_sixel(
    image: &DynamicImage,
    size: Size,
    is_tmux: bool,
    encoding: SixelEncoding,
) -> Result<Sixel, ImageError> {
    let rgba = image
        .as_rgba8()
        .map_or_else(|| Cow::Owned(image.to_rgba8()), Cow::Borrowed);
    let options = encoding.options();
    let mut sixel = icy_sixel::sixel_encode(
        rgba.as_raw(),
        rgba.width() as usize,
        rgba.height() as usize,
        &options,
    )
    .map_err(|error| ImageError::Sixel(format!("sixel encoding error: {error}")))?;
    let header_end = sixel
        .find('q')
        .map(|index| index + 1)
        .ok_or_else(|| ImageError::Sixel("sixel header did not end with q".to_owned()))?;
    // icy_sixel writes six-pixel bands, so declare the true raster to avoid a trailing cell row.
    sixel.insert_str(
        header_end,
        &format!("\"1;1;{};{}", rgba.width(), rgba.height()),
    );
    let (start, escape, end) = if is_tmux {
        ("\u{1b}Ptmux;", "\u{1b}\u{1b}", "\u{1b}\\")
    } else {
        ("", "\u{1b}", "")
    };
    let mut data = String::with_capacity(sixel.len().saturating_add(256));
    data.push_str(start);
    append_sixel_clear(&mut data, escape, size);
    if is_tmux {
        let Some(sixel) = sixel.strip_prefix('\u{1b}') else {
            return Err(ImageError::Tmux("sixel string did not start with escape"));
        };
        data.push_str(escape);
        data.push_str(sixel);
    } else {
        data.push_str(&sixel);
    }
    data.push_str(end);
    Ok(Sixel {
        data,
        size,
        is_tmux,
    })
}

fn append_sixel_clear(output: &mut String, escape: &str, size: Size) {
    use std::fmt::Write;

    if size.height == 1 {
        write!(output, "{escape}[{}X", size.width).unwrap();
        return;
    }
    for _ in 0..size.height {
        write!(output, "{escape}[{}X{escape}[1B", size.width).unwrap();
    }
    write!(output, "{escape}[{}A", size.height).unwrap();
}

fn picker_uses_tmux(picker: &Picker) -> bool {
    let mut picker = picker.clone();
    picker.set_protocol_type(ProtocolType::Sixel);
    let probe = picker.new_resize_protocol(DynamicImage::new_rgba8(1, 1));
    matches!(
        probe.protocol_type(),
        StatefulProtocolType::Sixel(Sixel { is_tmux: true, .. })
    )
}

fn protocol_kind(protocol: &Protocol) -> MediaPreviewProtocol {
    match protocol {
        Protocol::Halfblocks(_) => MediaPreviewProtocol::Halfblocks,
        Protocol::Kitty(_) => MediaPreviewProtocol::Kitty,
        Protocol::ITerm2(_) => MediaPreviewProtocol::Iterm2,
        Protocol::Sixel(_) => MediaPreviewProtocol::Sixel,
    }
}

fn stateful_payload_len(protocol: &StatefulProtocol) -> usize {
    match protocol.protocol_type() {
        StatefulProtocolType::ITerm2(encoded) => encoded.data.len(),
        StatefulProtocolType::Sixel(encoded) => encoded.data.len(),
        StatefulProtocolType::Halfblocks(_) | StatefulProtocolType::Kitty(_) => 0,
    }
}

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

pub(crate) enum MediaRenderState<'a> {
    Immediate(&'a Protocol),
    Threaded(&'a mut StatefulProtocol),
    Empty,
}

struct MediaProtocolState {
    protocol: Option<StatefulProtocol>,
    request_sender: Sender<MediaWorkerRequest>,
    latest_request: Arc<AtomicU64>,
    request_id: u64,
    final_applied: bool,
}

enum MediaWorkerRequest {
    Protocol {
        id: u64,
        protocol: Box<StatefulProtocol>,
        resize: Resize,
        size: Size,
        kind: MediaPreviewProtocol,
    },
    ProgressiveSixel {
        id: u64,
        request: ProgressiveSixelRequest,
    },
}

struct ProgressiveSixelRequest {
    image: Arc<DynamicImage>,
    font_size: FontSize,
    size: Size,
    background: Rgba<u8>,
    is_tmux: bool,
    preview_key: Option<SixelCacheKey>,
    final_key: SixelCacheKey,
}

struct SixelEncodeRequest {
    id: u64,
    image: Arc<DynamicImage>,
    size: Size,
    is_tmux: bool,
    key: SixelCacheKey,
    started: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaWorkerStage {
    Preview,
    Final,
}

struct MediaWorkerCompletion {
    id: u64,
    kind: MediaPreviewProtocol,
    stage: MediaWorkerStage,
    elapsed: Duration,
    result: Result<Option<MediaWorkerOutput>, ImageError>,
}

enum MediaWorkerOutput {
    Protocol(StatefulProtocol),
    Sixel { sixel: Sixel, key: SixelCacheKey },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SixelCacheKey {
    source_hash: u64,
    source_width: u32,
    source_height: u32,
    cell_width: u16,
    cell_height: u16,
    font_width: u16,
    font_height: u16,
    is_tmux: bool,
    encoding: SixelEncoding,
}

struct SixelCacheEntry {
    key: SixelCacheKey,
    sixel: Sixel,
}

struct DeferredSixelPreview {
    sixel: Sixel,
    release_at: Instant,
}

#[derive(Default)]
struct MediaPending {
    preview: bool,
    final_frame: bool,
    final_encoding: Option<SixelEncoding>,
}

impl MediaPending {
    fn any(&self) -> bool {
        self.preview || self.final_frame
    }

    fn complete(&mut self, stage: MediaWorkerStage) {
        match stage {
            MediaWorkerStage::Preview => self.preview = false,
            MediaWorkerStage::Final => self.final_frame = false,
        }
    }
}

impl MediaProtocolState {
    fn new(request_sender: Sender<MediaWorkerRequest>, latest_request: Arc<AtomicU64>) -> Self {
        Self {
            protocol: None,
            request_sender,
            latest_request,
            request_id: 0,
            final_applied: false,
        }
    }

    fn next_request_id(&mut self) -> u64 {
        self.request_id = self.request_id.wrapping_add(1);
        self.final_applied = false;
        self.latest_request
            .store(self.request_id, Ordering::Release);
        self.request_id
    }

    fn request_protocol(
        &mut self,
        protocol: StatefulProtocol,
        resize: Resize,
        size: Size,
        kind: MediaPreviewProtocol,
    ) -> bool {
        let id = self.next_request_id();
        self.protocol = None;
        self.request_sender
            .send(MediaWorkerRequest::Protocol {
                id,
                protocol: Box::new(protocol),
                resize,
                size,
                kind,
            })
            .is_ok()
    }

    fn request_progressive_sixel(&mut self, request: ProgressiveSixelRequest) -> bool {
        let id = self.next_request_id();
        self.protocol = None;
        self.request_sender
            .send(MediaWorkerRequest::ProgressiveSixel { id, request })
            .is_ok()
    }

    fn accepts(&self, id: u64) -> bool {
        self.request_id == id
    }

    fn final_applied(&self) -> bool {
        self.final_applied
    }

    fn mark_final_applied(&mut self) {
        self.final_applied = true;
    }

    fn set_protocol(&mut self, protocol: StatefulProtocol) {
        self.protocol = Some(protocol);
    }

    fn protocol_mut(&mut self) -> Option<&mut StatefulProtocol> {
        self.protocol.as_mut()
    }

    fn empty_protocol(&mut self) {
        self.next_request_id();
        self.protocol = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveKittyImage {
    revision: u64,
    image_id: u32,
    area: Rect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveInlineImage {
    revision: u64,
    protocol: MediaPreviewProtocol,
    area: Rect,
}

#[derive(Clone, Copy, Debug)]
struct MediaViewMetrics {
    scaled_width: f64,
    scaled_height: f64,
    viewport: Rect,
    area: Rect,
}

impl MediaViewMetrics {
    fn zoomed(mut self, zoom_ratio: f64) -> Self {
        self.scaled_width *= zoom_ratio;
        self.scaled_height *= zoom_ratio;
        let width = self.scaled_width.ceil().min(f64::from(self.viewport.width)) as u16;
        let height = self
            .scaled_height
            .ceil()
            .min(f64::from(self.viewport.height)) as u16;
        self.area = Rect::new(
            self.viewport.x + (self.viewport.width - width) / 2,
            self.viewport.y + (self.viewport.height - height) / 2,
            width,
            height,
        );
        self
    }

    fn anchored_center(
        self,
        center_x: f64,
        center_y: f64,
        pointer: Position,
        zoom_ratio: f64,
    ) -> (f64, f64) {
        if !self.area.contains(pointer) {
            return (center_x, center_y);
        }
        let axis = |center: f64,
                    scaled: f64,
                    viewport_start: u16,
                    viewport_extent: u16,
                    area_start: u16,
                    area_extent: u16,
                    pointer: u16| {
            let pointer = f64::from(pointer) + 0.5;
            let position =
                ((pointer - f64::from(area_start)) / f64::from(area_extent)).clamp(0.0, 1.0);
            let visible = (f64::from(viewport_extent) / scaled).min(1.0);
            let source_at_pointer = center + (position - 0.5) * visible;

            let next_scaled = scaled * zoom_ratio;
            let next_area_extent = next_scaled.ceil().min(f64::from(viewport_extent)) as u16;
            let next_area_start = viewport_start + (viewport_extent - next_area_extent) / 2;
            let next_position = ((pointer - f64::from(next_area_start))
                / f64::from(next_area_extent))
            .clamp(0.0, 1.0);
            let next_visible = (f64::from(viewport_extent) / next_scaled).min(1.0);
            if next_visible == 1.0 {
                0.5
            } else {
                let half_visible = next_visible / 2.0;
                (source_at_pointer - (next_position - 0.5) * next_visible)
                    .clamp(half_visible, 1.0 - half_visible)
            }
        };
        (
            axis(
                center_x,
                self.scaled_width,
                self.viewport.x,
                self.viewport.width,
                self.area.x,
                self.area.width,
                pointer.x,
            ),
            axis(
                center_y,
                self.scaled_height,
                self.viewport.y,
                self.viewport.height,
                self.area.y,
                self.area.height,
                pointer.y,
            ),
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct MediaView {
    zoom_level: u8,
    center_x: f64,
    center_y: f64,
    revision: u64,
    drag_position: Option<Position>,
    preview_until: Option<Instant>,
    metrics: Option<MediaViewMetrics>,
}

impl Default for MediaView {
    fn default() -> Self {
        Self {
            zoom_level: 0,
            center_x: 0.5,
            center_y: 0.5,
            revision: 0,
            drag_position: None,
            preview_until: None,
            metrics: None,
        }
    }
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

#[derive(Clone, Copy)]
pub(crate) struct EditorPreviewInput<'a> {
    pub(crate) source: &'a str,
    pub(crate) line_starts: &'a [usize],
    pub(crate) revision: u64,
    pub(crate) revision_changed_from_line: usize,
    pub(crate) repo_path: &'a RepoPath,
    pub(crate) path: &'a str,
    pub(crate) width: usize,
    pub(crate) viewport_height: usize,
    pub(crate) wrapped: bool,
}

pub(crate) struct PreparedEditorPreview {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) rows: Vec<crate::app::EditorRenderedRow>,
}

pub(crate) struct PreviewPresentation {
    cache: Option<PreviewCache>,
    leading_markdown: Option<LeadingMarkdownCache>,
    editor_cache: Option<EditorPreviewCache>,
    editor_markers: Option<EditorMarkerCache>,
    media_state: Option<MediaProtocolState>,
    media_receiver: Receiver<MediaWorkerCompletion>,
    media_workers: Vec<JoinHandle<()>>,
    media_picker: Picker,
    media_sixel_tmux: bool,
    allow_auto_kitty: bool,
    media_generation: Option<u64>,
    media_protocol: Option<MediaPreviewProtocol>,
    media_sixel_quality: Option<SixelQuality>,
    media_available: Size,
    media_applied_view_revision: u64,
    media_applied_immediate: bool,
    media_frame_revision: u64,
    media_view: MediaView,
    media_preview: Option<Protocol>,
    media_pending: MediaPending,
    deferred_sixel_preview: Option<DeferredSixelPreview>,
    sixel_cache: VecDeque<SixelCacheEntry>,
    sixel_cache_bytes: usize,
    effective_media_protocol: MediaPreviewProtocol,
    media_size: Size,
    media_error: Option<String>,
    active_kitty_image: Option<ActiveKittyImage>,
    active_inline_image: Option<ActiveInlineImage>,
    pending_terminal_cleanup: Vec<u8>,
    pending_terminal_output: MediaTerminalOutput,
}

struct LeadingMarkdownCache {
    generation: u64,
    width: usize,
    wrapped: bool,
    lines: Arc<[Line<'static>]>,
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
    source_lines: Option<SourceLineIndex>,
    wrapped_line_starts: Option<Vec<usize>>,
    wrapped_window: Option<WrappedWindow>,
    unwrapped_hunks: Option<(Vec<(usize, usize)>, usize)>,
    wrapped_hunks: Option<(Vec<(usize, usize)>, usize)>,
}

struct SourceLineIndex {
    count: usize,
    checkpoints: Vec<usize>,
}

impl SourceLineIndex {
    fn new(source: &str) -> Self {
        let mut checkpoints = Vec::new();
        let mut count = 0_usize;
        let mut byte_offset = 0_usize;
        for line in source.split_inclusive('\n') {
            if count.is_multiple_of(SOURCE_LINE_CHECKPOINT_STRIDE) {
                checkpoints.push(byte_offset);
            }
            count = count.saturating_add(1);
            byte_offset = byte_offset.saturating_add(line.len());
        }
        Self { count, checkpoints }
    }

    fn checkpoint(&self, line: usize) -> Option<(usize, usize)> {
        (line < self.count).then(|| {
            let checkpoint_line =
                line / SOURCE_LINE_CHECKPOINT_STRIDE * SOURCE_LINE_CHECKPOINT_STRIDE;
            let byte_offset = self.checkpoints[line / SOURCE_LINE_CHECKPOINT_STRIDE];
            (checkpoint_line, byte_offset)
        })
    }

    fn line<'a>(&self, source: &'a str, line: usize) -> Option<&'a str> {
        let (checkpoint_line, byte_offset) = self.checkpoint(line)?;
        source
            .get(byte_offset..)?
            .lines()
            .nth(line.saturating_sub(checkpoint_line))
    }
}

struct WrappedWindow {
    first: usize,
    end: usize,
    local_scroll: usize,
    viewport_height: usize,
    lines: Vec<Line<'static>>,
}

struct EditorPreviewCache {
    revision: u64,
    path: RepoPath,
    width: usize,
    wrapped: bool,
    wrapped_line_starts: Vec<usize>,
    window: Option<EditorPreviewWindow>,
    #[cfg(test)]
    wrapped_lines_computed: usize,
    #[cfg(test)]
    window_builds: usize,
}

struct EditorPreviewWindow {
    scroll: usize,
    viewport_height: usize,
    lines: Vec<Line<'static>>,
    rows: Vec<crate::app::EditorRenderedRow>,
}

struct EditorMarkerCache {
    generation: u64,
    path: RepoPath,
    markers: BTreeMap<usize, char>,
}

fn spawn_sixel_encoder(
    request_receiver: Receiver<SixelEncodeRequest>,
    result_sender: Sender<MediaWorkerCompletion>,
    latest_request: Arc<AtomicU64>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(mut request) = request_receiver.recv() {
            while let Ok(newer_request) = request_receiver.try_recv() {
                request = newer_request;
            }
            let stage = if request.key.encoding.is_final() {
                MediaWorkerStage::Final
            } else {
                MediaWorkerStage::Preview
            };
            let result = if latest_request.load(Ordering::Acquire) != request.id {
                Ok(None)
            } else {
                encode_sixel(
                    &request.image,
                    request.size,
                    request.is_tmux,
                    request.key.encoding,
                )
                .map(|sixel| {
                    Some(MediaWorkerOutput::Sixel {
                        sixel,
                        key: request.key,
                    })
                })
            };
            if result_sender
                .send(MediaWorkerCompletion {
                    id: request.id,
                    kind: MediaPreviewProtocol::Sixel,
                    stage,
                    elapsed: request.started.elapsed(),
                    result,
                })
                .is_err()
            {
                break;
            }
        }
    })
}

fn spawn_media_worker(
    request_receiver: Receiver<MediaWorkerRequest>,
    preview_sender: Sender<SixelEncodeRequest>,
    final_sender: Sender<SixelEncodeRequest>,
    result_sender: Sender<MediaWorkerCompletion>,
    latest_request: Arc<AtomicU64>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(mut request) = request_receiver.recv() {
            while let Ok(newer_request) = request_receiver.try_recv() {
                request = newer_request;
            }
            let started = Instant::now();
            match request {
                MediaWorkerRequest::Protocol {
                    id,
                    mut protocol,
                    resize,
                    size,
                    kind,
                } => {
                    let result = if latest_request.load(Ordering::Acquire) != id {
                        Ok(None)
                    } else {
                        protocol.resize_encode(&resize, size);
                        match protocol
                            .last_encoding_result()
                            .expect("media encoding just completed")
                        {
                            Ok(()) => Ok(Some(MediaWorkerOutput::Protocol(*protocol))),
                            Err(error) => Err(error),
                        }
                    };
                    if result_sender
                        .send(MediaWorkerCompletion {
                            id,
                            kind,
                            stage: MediaWorkerStage::Final,
                            elapsed: started.elapsed(),
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                MediaWorkerRequest::ProgressiveSixel { id, request } => {
                    if latest_request.load(Ordering::Acquire) != id {
                        continue;
                    }
                    let image = Arc::new(Resize::Scale(None).resize(
                        &request.image,
                        request.font_size,
                        request.size,
                        Some(request.background),
                    ));
                    if latest_request.load(Ordering::Acquire) != id {
                        continue;
                    }
                    let final_request = SixelEncodeRequest {
                        id,
                        image: Arc::clone(&image),
                        size: request.size,
                        is_tmux: request.is_tmux,
                        key: request.final_key,
                        started,
                    };
                    if final_sender.send(final_request).is_err()
                        && result_sender
                            .send(MediaWorkerCompletion {
                                id,
                                kind: MediaPreviewProtocol::Sixel,
                                stage: MediaWorkerStage::Final,
                                elapsed: started.elapsed(),
                                result: Err(ImageError::Sixel(
                                    "final SIXEL worker stopped".to_owned(),
                                )),
                            })
                            .is_err()
                    {
                        break;
                    }
                    if let Some(key) = request.preview_key {
                        let preview_request = SixelEncodeRequest {
                            id,
                            image,
                            size: request.size,
                            is_tmux: request.is_tmux,
                            key,
                            started,
                        };
                        if preview_sender.send(preview_request).is_err()
                            && result_sender
                                .send(MediaWorkerCompletion {
                                    id,
                                    kind: MediaPreviewProtocol::Sixel,
                                    stage: MediaWorkerStage::Preview,
                                    elapsed: started.elapsed(),
                                    result: Err(ImageError::Sixel(
                                        "preview SIXEL worker stopped".to_owned(),
                                    )),
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }
    })
}

impl Default for PreviewPresentation {
    fn default() -> Self {
        let (request_sender, request_receiver) = mpsc::channel::<MediaWorkerRequest>();
        let (result_sender, media_receiver) = mpsc::channel();
        let (preview_sender, preview_receiver) = mpsc::channel();
        let (final_sender, final_receiver) = mpsc::channel();
        let latest_request = Arc::new(AtomicU64::new(0));
        let media_worker = spawn_media_worker(
            request_receiver,
            preview_sender,
            final_sender,
            result_sender.clone(),
            Arc::clone(&latest_request),
        );
        let preview_worker = spawn_sixel_encoder(
            preview_receiver,
            result_sender.clone(),
            Arc::clone(&latest_request),
        );
        let final_worker =
            spawn_sixel_encoder(final_receiver, result_sender, Arc::clone(&latest_request));
        let mut media_picker = Picker::halfblocks();
        media_picker.set_background_color(Some(media_background_color()));
        let media_sixel_tmux = picker_uses_tmux(&media_picker);
        Self {
            cache: None,
            leading_markdown: None,
            editor_cache: None,
            editor_markers: None,
            media_state: Some(MediaProtocolState::new(request_sender, latest_request)),
            media_receiver,
            media_workers: vec![media_worker, preview_worker, final_worker],
            media_picker,
            media_sixel_tmux,
            allow_auto_kitty: false,
            media_generation: None,
            media_protocol: None,
            media_sixel_quality: None,
            media_available: Size::default(),
            media_applied_view_revision: 0,
            media_applied_immediate: false,
            media_frame_revision: 0,
            media_view: MediaView::default(),
            media_preview: None,
            media_pending: MediaPending::default(),
            deferred_sixel_preview: None,
            sixel_cache: VecDeque::new(),
            sixel_cache_bytes: 0,
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
        self.leading_markdown = None;
        self.editor_cache = None;
        self.editor_markers = None;
        self.hide_media();
        self.media_view = MediaView::default();
    }

    fn cached_sixel(&mut self, key: SixelCacheKey) -> Option<Sixel> {
        let index = self.sixel_cache.iter().position(|entry| entry.key == key)?;
        let entry = self.sixel_cache.remove(index)?;
        let sixel = entry.sixel.clone();
        self.sixel_cache.push_back(entry);
        Some(sixel)
    }

    fn cache_sixel(&mut self, key: SixelCacheKey, sixel: Sixel) {
        let bytes = sixel.data.len();
        if bytes > MAX_CACHED_SIXEL_BYTES {
            return;
        }
        if let Some(index) = self.sixel_cache.iter().position(|entry| entry.key == key)
            && let Some(previous) = self.sixel_cache.remove(index)
        {
            self.sixel_cache_bytes = self
                .sixel_cache_bytes
                .saturating_sub(previous.sixel.data.len());
        }
        while self.sixel_cache.len() >= MAX_CACHED_SIXEL_FRAMES
            || self.sixel_cache_bytes.saturating_add(bytes) > MAX_CACHED_SIXEL_BYTES
        {
            let Some(previous) = self.sixel_cache.pop_front() else {
                break;
            };
            self.sixel_cache_bytes = self
                .sixel_cache_bytes
                .saturating_sub(previous.sixel.data.len());
        }
        self.sixel_cache_bytes = self.sixel_cache_bytes.saturating_add(bytes);
        self.sixel_cache.push_back(SixelCacheEntry { key, sixel });
    }

    pub(crate) fn leading_markdown(
        &mut self,
        generation: u64,
        markdown: &str,
        width: usize,
        wrapped: bool,
    ) -> Arc<[Line<'static>]> {
        let matches = self.leading_markdown.as_ref().is_some_and(|cache| {
            cache.generation == generation && cache.width == width && cache.wrapped == wrapped
        });
        if !matches {
            let lines = styled_markdown(markdown, width, wrapped);
            let lines = if wrapped {
                hard_wrap_preview_lines(lines, width, 0, usize::MAX, false, true)
            } else {
                lines
            };
            self.leading_markdown = Some(LeadingMarkdownCache {
                generation,
                width,
                wrapped,
                lines: Arc::from(lines),
            });
        }
        Arc::clone(
            &self
                .leading_markdown
                .as_ref()
                .expect("leading Markdown was prepared")
                .lines,
        )
    }

    pub(crate) fn editor_line_markers(
        &mut self,
        generation: u64,
        diff: Option<&DiffDocument>,
        path: &RepoPath,
        locally_changed_lines: &BTreeSet<usize>,
    ) -> BTreeMap<usize, char> {
        let matches = self
            .editor_markers
            .as_ref()
            .is_some_and(|cache| cache.generation == generation && cache.path == *path);
        if !matches {
            self.editor_markers = Some(EditorMarkerCache {
                generation,
                path: path.clone(),
                markers: diff
                    .map(|diff| diff.new_line_markers(path).into_iter().collect())
                    .unwrap_or_default(),
            });
        }
        let mut markers = self
            .editor_markers
            .as_ref()
            .map(|cache| cache.markers.clone())
            .unwrap_or_default();
        for line in locally_changed_lines {
            markers.insert(*line, '~');
        }
        markers
    }

    pub(crate) fn editor_rendered_position(
        &mut self,
        input: EditorPreviewInput<'_>,
        line: usize,
        column: usize,
    ) -> (usize, usize) {
        self.ensure_editor_cache(input);
        if !input.wrapped {
            return (line, column);
        }

        let line = line.min(input.line_starts.len().saturating_sub(1));
        let cache = self
            .editor_cache
            .as_mut()
            .expect("editor preview cache was initialized");
        extend_editor_wrapped_lines(cache, input, line.saturating_add(1));
        let visual_row = cache
            .wrapped_line_starts
            .get(line)
            .copied()
            .unwrap_or_default();
        let rows = super::text::word_wrapped_rows(editor_source_line(input, line), input.width);
        let (row, rendered_column) = rows
            .iter()
            .enumerate()
            .min_by_key(|(_, row)| {
                row.source_column_at(row.rendered_column_at(column))
                    .abs_diff(column)
            })
            .map_or((0, 0), |(index, row)| {
                (index, row.rendered_column_at(column))
            });
        (visual_row.saturating_add(row), rendered_column)
    }

    pub(crate) fn prepare_editor(
        &mut self,
        input: EditorPreviewInput<'_>,
        scroll: &mut usize,
    ) -> PreparedEditorPreview {
        self.ensure_editor_cache(input);
        let line_count = input.line_starts.len();
        if line_count == 0 {
            return PreparedEditorPreview {
                lines: Vec::new(),
                rows: Vec::new(),
            };
        }

        if input.wrapped {
            let cache = self
                .editor_cache
                .as_mut()
                .expect("editor preview cache was initialized");
            let viewport_end = scroll.saturating_add(input.viewport_height.max(1));
            while cache
                .wrapped_line_starts
                .last()
                .copied()
                .unwrap_or_default()
                <= viewport_end
                && cache.wrapped_line_starts.len() <= line_count
            {
                let next_line = cache.wrapped_line_starts.len().saturating_sub(1);
                extend_editor_wrapped_lines(cache, input, next_line.saturating_add(1));
            }
            if cache.wrapped_line_starts.len() == line_count.saturating_add(1) {
                let rendered_height = cache
                    .wrapped_line_starts
                    .last()
                    .copied()
                    .unwrap_or_default();
                *scroll =
                    (*scroll).min(rendered_height.saturating_sub(input.viewport_height.max(1)));
            }
        } else {
            *scroll = (*scroll).min(line_count.saturating_sub(input.viewport_height.max(1)));
        }

        let cache = self
            .editor_cache
            .as_ref()
            .expect("editor preview cache was initialized");
        if let Some(window) = cache.window.as_ref().filter(|window| {
            window.scroll == *scroll && window.viewport_height == input.viewport_height
        }) {
            return PreparedEditorPreview {
                lines: window.lines.clone(),
                rows: window.rows.clone(),
            };
        }

        let (first, end, local_scroll) = if input.wrapped {
            let starts = &cache.wrapped_line_starts;
            let first = starts
                .partition_point(|start| *start <= *scroll)
                .saturating_sub(1)
                .min(line_count.saturating_sub(1));
            let viewport_end = scroll.saturating_add(input.viewport_height);
            let end = starts
                .partition_point(|start| *start < viewport_end)
                .max(first.saturating_add(1))
                .min(line_count);
            (first, end, scroll.saturating_sub(starts[first]))
        } else {
            let first = (*scroll).min(line_count.saturating_sub(1));
            (
                first,
                first.saturating_add(input.viewport_height).min(line_count),
                0,
            )
        };
        let byte_start = input.line_starts[first];
        let byte_end = input
            .line_starts
            .get(end)
            .copied()
            .unwrap_or(input.source.len());
        let logical_lines = styled_editor_source_window_from(
            &input.source[byte_start..byte_end],
            input.path,
            first,
            end.saturating_sub(first),
        );
        let (lines, rows) = if input.wrapped {
            let lines = hard_wrap_lines(
                logical_lines,
                input.width,
                local_scroll,
                input.viewport_height,
                false,
                false,
            );
            let starts = &cache.wrapped_line_starts;
            let viewport_end = scroll.saturating_add(input.viewport_height);
            let mut rows = Vec::with_capacity(lines.len());
            for (line, &line_start) in starts.iter().enumerate().take(end).skip(first) {
                for (row_index, row) in
                    super::text::word_wrapped_rows(editor_source_line(input, line), input.width)
                        .into_iter()
                        .enumerate()
                {
                    let rendered_row = line_start.saturating_add(row_index);
                    if rendered_row >= *scroll && rendered_row < viewport_end {
                        rows.push(crate::app::EditorRenderedRow {
                            line,
                            columns: row.columns(),
                        });
                    }
                }
            }
            (lines, rows)
        } else {
            (logical_lines, Vec::new())
        };
        self.editor_cache
            .as_mut()
            .expect("editor preview cache was initialized")
            .window = Some(EditorPreviewWindow {
            scroll: *scroll,
            viewport_height: input.viewport_height,
            lines: lines.clone(),
            rows: rows.clone(),
        });
        #[cfg(test)]
        {
            self.editor_cache
                .as_mut()
                .expect("editor preview cache was initialized")
                .window_builds += 1;
        }
        PreparedEditorPreview { lines, rows }
    }

    fn ensure_editor_cache(&mut self, input: EditorPreviewInput<'_>) {
        let matches = self.editor_cache.as_ref().is_some_and(|cache| {
            cache.revision == input.revision
                && cache.path == *input.repo_path
                && cache.width == input.width
                && cache.wrapped == input.wrapped
        });
        if !matches {
            let can_reuse_wrapping = self.editor_cache.as_ref().is_some_and(|cache| {
                cache.path == *input.repo_path
                    && cache.width == input.width
                    && cache.wrapped == input.wrapped
            });
            if can_reuse_wrapping {
                let cache = self.editor_cache.as_mut().expect("editor cache exists");
                cache.revision = input.revision;
                cache
                    .wrapped_line_starts
                    .truncate(input.revision_changed_from_line.saturating_add(1));
                if cache.wrapped_line_starts.is_empty() {
                    cache.wrapped_line_starts.push(0);
                }
                cache.window = None;
            } else {
                self.editor_cache = Some(EditorPreviewCache {
                    revision: input.revision,
                    path: input.repo_path.clone(),
                    width: input.width,
                    wrapped: input.wrapped,
                    wrapped_line_starts: vec![0],
                    window: None,
                    #[cfg(test)]
                    wrapped_lines_computed: 0,
                    #[cfg(test)]
                    window_builds: 0,
                });
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn editor_cache_metrics(&self) -> (usize, usize) {
        self.editor_cache.as_ref().map_or((0, 0), |cache| {
            (cache.wrapped_lines_computed, cache.window_builds)
        })
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
        let line = self.source_line(content, display_line).unwrap_or_default();
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

    pub(crate) fn source_line<'a>(&self, content: &'a str, line: usize) -> Option<&'a str> {
        let cache = self.cache.as_ref()?;
        if let Some(lines) = &cache.source_lines {
            lines.line(content, line)
        } else {
            content.lines().nth(line)
        }
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
        self.clear_active_terminal_media();
        if self.media_generation.take().is_some() {
            self.media_state
                .as_mut()
                .expect("media worker is running")
                .empty_protocol();
        }
        self.media_protocol = None;
        self.media_sixel_quality = None;
        self.media_available = Size::default();
        self.media_applied_immediate = false;
        self.media_size = Size::default();
        self.media_preview = None;
        self.media_pending = MediaPending::default();
        self.deferred_sixel_preview = None;
        self.media_error = None;
        self.media_view.drag_position = None;
        self.media_view.preview_until = None;
    }

    fn clear_active_terminal_media(&mut self) {
        if self.active_kitty_image.take().is_some() {
            self.pending_terminal_cleanup
                .extend_from_slice(KITTY_DELETE_ALL.as_bytes());
        }
        if let Some(active) = self.active_inline_image.take() {
            append_clear_area(&mut self.pending_terminal_cleanup, active.area);
        }
    }

    pub(crate) fn media_zoom_percent(&self) -> u16 {
        (MEDIA_ZOOM_STEP.powi(i32::from(self.media_view.zoom_level)) * 100.0).round() as u16
    }

    pub(crate) fn zoom_media(&mut self, zoom_in: bool, anchor: Option<Position>) -> bool {
        let current = self.media_view.zoom_level;
        let next = if zoom_in {
            current.saturating_add(1).min(MAX_MEDIA_ZOOM_LEVEL)
        } else {
            current.saturating_sub(1)
        };
        if next == current {
            return false;
        }
        if let Some(metrics) = self.media_view.metrics {
            let zoom_ratio = MEDIA_ZOOM_STEP.powi(i32::from(next) - i32::from(current));
            if let Some(anchor) = anchor {
                (self.media_view.center_x, self.media_view.center_y) = metrics.anchored_center(
                    self.media_view.center_x,
                    self.media_view.center_y,
                    anchor,
                    zoom_ratio,
                );
            }
            self.media_view.metrics = Some(metrics.zoomed(zoom_ratio));
        }
        self.media_view.zoom_level = next;
        if next == 0 {
            self.media_view.center_x = 0.5;
            self.media_view.center_y = 0.5;
        }
        self.media_view.preview_until = Some(Instant::now() + MEDIA_INTERACTION_SETTLE);
        self.media_view.revision = self.media_view.revision.wrapping_add(1);
        true
    }

    pub(crate) fn begin_media_pan(&mut self, position: Position) {
        self.media_view.drag_position = Some(position);
        self.media_view.preview_until = None;
    }

    pub(crate) fn media_pan_active(&self) -> bool {
        self.media_view.drag_position.is_some()
    }

    pub(crate) fn pan_media(&mut self, position: Position) -> bool {
        let Some(previous) = self.media_view.drag_position.replace(position) else {
            return false;
        };
        let Some(metrics) = self.media_view.metrics else {
            return false;
        };
        let clamp_center = |center: f64, scaled: f64, viewport: f64| {
            if scaled <= viewport {
                0.5
            } else {
                let half_visible = viewport / scaled / 2.0;
                center.clamp(half_visible, 1.0 - half_visible)
            }
        };
        let next_x = clamp_center(
            self.media_view.center_x
                - (f64::from(position.x) - f64::from(previous.x)) / metrics.scaled_width,
            metrics.scaled_width,
            f64::from(metrics.viewport.width),
        );
        let next_y = clamp_center(
            self.media_view.center_y
                - (f64::from(position.y) - f64::from(previous.y)) / metrics.scaled_height,
            metrics.scaled_height,
            f64::from(metrics.viewport.height),
        );
        if next_x == self.media_view.center_x && next_y == self.media_view.center_y {
            return false;
        }
        self.media_view.center_x = next_x;
        self.media_view.center_y = next_y;
        self.media_view.revision = self.media_view.revision.wrapping_add(1);
        true
    }

    pub(crate) fn end_media_pan(&mut self) {
        if self.media_view.drag_position.take().is_some() {
            self.media_view.preview_until = None;
            self.media_view.revision = self.media_view.revision.wrapping_add(1);
        }
    }

    fn media_live_preview_active_at(&self, now: Instant) -> bool {
        self.media_view.drag_position.is_some()
            || self
                .media_view
                .preview_until
                .is_some_and(|deadline| deadline > now)
    }

    pub(crate) fn poll_media(&mut self) -> bool {
        self.poll_media_at(Instant::now())
    }

    fn apply_media_completion_at(
        &mut self,
        completion: MediaWorkerCompletion,
        now: Instant,
    ) -> bool {
        let accepted = self
            .media_state
            .as_ref()
            .expect("media worker is running")
            .accepts(completion.id);
        if !accepted {
            return false;
        }
        self.media_pending.complete(completion.stage);
        match completion.result {
            Ok(Some(MediaWorkerOutput::Protocol(protocol))) => {
                let payload_bytes = stateful_payload_len(&protocol);
                let state = self.media_state.as_mut().expect("media worker is running");
                state.set_protocol(protocol);
                state.mark_final_applied();
                self.deferred_sixel_preview = None;
                self.media_error = None;
                self.media_preview = None;
                self.media_frame_revision = self.media_frame_revision.wrapping_add(1);
                crate::diagnostics::event(format!(
                    "media encode finished protocol={} elapsed_ms={} payload_bytes={} cached=false",
                    completion.kind.as_str(),
                    completion.elapsed.as_millis(),
                    payload_bytes
                ));
                true
            }
            Ok(Some(MediaWorkerOutput::Sixel { sixel, key })) => {
                let payload_bytes = sixel.data.len();
                let final_applied = self
                    .media_state
                    .as_ref()
                    .expect("media worker is running")
                    .final_applied();
                self.cache_sixel(key, sixel.clone());
                let defer = key.encoding == SixelEncoding::Preview
                    && !final_applied
                    && self.media_pending.final_frame
                    && self.media_pending.final_encoding == Some(SixelEncoding::Fast);
                let apply = key.encoding.is_final() || (!final_applied && !defer);
                if apply {
                    if key.encoding.is_final() {
                        self.media_state
                            .as_mut()
                            .expect("media worker is running")
                            .mark_final_applied();
                        self.deferred_sixel_preview = None;
                        self.media_error = None;
                    } else if self.media_pending.final_frame {
                        self.media_error = None;
                    }
                    self.media_preview = Some(Protocol::Sixel(sixel));
                    self.media_frame_revision = self.media_frame_revision.wrapping_add(1);
                } else if defer {
                    self.deferred_sixel_preview = Some(DeferredSixelPreview {
                        sixel,
                        release_at: now + FAST_SIXEL_PREVIEW_GRACE,
                    });
                }
                crate::diagnostics::event(format!(
                    "media encode finished protocol={} elapsed_ms={} payload_bytes={} cached=false applied={apply} deferred={defer}",
                    key.encoding.diagnostic_name(),
                    completion.elapsed.as_millis(),
                    payload_bytes
                ));
                apply
            }
            Ok(None) => false,
            Err(error) if completion.stage == MediaWorkerStage::Preview => {
                crate::diagnostics::event(format!(
                    "media preview encode failed elapsed_ms={} error={error}",
                    completion.elapsed.as_millis()
                ));
                let final_applied = self
                    .media_state
                    .as_ref()
                    .expect("media worker is running")
                    .final_applied();
                if !self.media_pending.final_frame && !final_applied {
                    self.media_error = Some(format!("Could not render media preview: {error}"));
                    return true;
                }
                false
            }
            Err(error) => {
                self.apply_deferred_sixel_preview();
                self.media_error = Some(format!("Could not render final media preview: {error}"));
                true
            }
        }
    }

    fn apply_deferred_sixel_preview(&mut self) -> bool {
        let Some(deferred) = self.deferred_sixel_preview.take() else {
            return false;
        };
        let final_applied = self
            .media_state
            .as_ref()
            .expect("media worker is running")
            .final_applied();
        if final_applied {
            return false;
        }
        self.media_preview = Some(Protocol::Sixel(deferred.sixel));
        self.media_frame_revision = self.media_frame_revision.wrapping_add(1);
        self.media_error = None;
        crate::diagnostics::event("media SIXEL preview grace elapsed; applying preview");
        true
    }

    fn poll_media_at(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self
            .media_view
            .preview_until
            .is_some_and(|deadline| deadline <= now)
        {
            self.media_view.preview_until = None;
            self.media_view.revision = self.media_view.revision.wrapping_add(1);
            changed = true;
        }
        loop {
            let completion = match self.media_receiver.try_recv() {
                Ok(completion) => completion,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.media_pending.any() {
                        self.media_pending = MediaPending::default();
                        self.deferred_sixel_preview = None;
                        self.media_error = Some("Media preview worker stopped".to_owned());
                        changed = true;
                    }
                    break;
                }
            };
            changed |= self.apply_media_completion_at(completion, now);
        }
        if self
            .deferred_sixel_preview
            .as_ref()
            .is_some_and(|preview| preview.release_at <= now)
        {
            changed |= self.apply_deferred_sixel_preview();
        }
        changed
    }

    pub(crate) fn media_error(&self) -> Option<&str> {
        self.media_error.as_deref()
    }

    pub(crate) fn media_work_pending(&self) -> bool {
        self.media_pending.any()
    }

    pub(crate) fn queue_kitty_frame(
        &mut self,
        revision: u64,
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
                revision,
                image_id: transmission.image_id,
                area,
            });
            return;
        }

        let Some(active) = self.active_kitty_image.as_ref() else {
            return;
        };
        if active.revision != revision {
            self.active_kitty_image = None;
            self.pending_terminal_cleanup
                .extend_from_slice(KITTY_DELETE_ALL.as_bytes());
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
        self.active_kitty_image
            .as_mut()
            .expect("active Kitty image was checked")
            .area = area;
    }

    pub(crate) fn queue_inline_frame(
        &mut self,
        revision: u64,
        protocol: MediaPreviewProtocol,
        area: Rect,
        transmission: Option<Vec<u8>>,
    ) {
        let Some(transmission) = transmission else {
            return;
        };
        let next = ActiveInlineImage {
            revision,
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
        self.media_sixel_quality = None;
        self.media_available = Size::default();
        self.media_applied_immediate = false;
        self.media_size = Size::default();
        self.media_preview = None;
        self.media_pending = MediaPending::default();
        self.deferred_sixel_preview = None;
        self.media_error = None;
        self.media_view.drag_position = None;
        self.media_view.preview_until = None;
    }

    pub(crate) fn configure_media_picker(&mut self, mut picker: Picker, allow_auto_kitty: bool) {
        self.terminal_restarted();
        picker.set_background_color(Some(media_background_color()));
        self.media_sixel_tmux = picker_uses_tmux(&picker);
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
        sixel_quality: SixelQuality,
        available: Rect,
    ) -> (Rect, MediaPreviewProtocol, u64, MediaRenderState<'_>) {
        let final_protocol = self.effective_protocol(protocol);
        let immediate = self.media_live_preview_active_at(Instant::now());
        if available.is_empty() || image.width() == 0 || image.height() == 0 {
            self.media_size = Size::default();
            self.media_view.metrics = None;
            return (
                Rect::new(available.x, available.y, 0, 0),
                final_protocol,
                self.media_frame_revision,
                MediaRenderState::Empty,
            );
        }
        let available_size = available.into();
        if self.media_generation != Some(generation)
            || self.media_protocol != Some(protocol)
            || self.media_sixel_quality != Some(sixel_quality)
            || self.media_available != available_size
            || self.media_applied_view_revision != self.media_view.revision
            || self.media_applied_immediate != immediate
        {
            let font_size = self.media_picker.font_size();
            let image_width = f64::from(image.width());
            let image_height = f64::from(image.height());
            let font_width = f64::from(font_size.width.max(1));
            let font_height = f64::from(font_size.height.max(1));
            let viewport_width = f64::from(available.width) * font_width;
            let viewport_height = f64::from(available.height) * font_height;
            let fit_scale = (viewport_width / image_width)
                .min(viewport_height / image_height)
                .min(1.0);
            let scale = fit_scale * MEDIA_ZOOM_STEP.powi(i32::from(self.media_view.zoom_level));
            let scaled_width = image_width * scale;
            let scaled_height = image_height * scale;
            let visible_width = scaled_width.min(viewport_width);
            let visible_height = scaled_height.min(viewport_height);
            let crop_width = ((visible_width / scale).ceil() as u32).clamp(1, image.width());
            let crop_height = ((visible_height / scale).ceil() as u32).clamp(1, image.height());
            let half_crop_x = f64::from(crop_width) / image_width / 2.0;
            let half_crop_y = f64::from(crop_height) / image_height / 2.0;
            self.media_view.center_x = self
                .media_view
                .center_x
                .clamp(half_crop_x, 1.0 - half_crop_x);
            self.media_view.center_y = self
                .media_view
                .center_y
                .clamp(half_crop_y, 1.0 - half_crop_y);
            let crop_x = (self.media_view.center_x * image_width - f64::from(crop_width) / 2.0)
                .round()
                .clamp(0.0, f64::from(image.width() - crop_width)) as u32;
            let crop_y = (self.media_view.center_y * image_height - f64::from(crop_height) / 2.0)
                .round()
                .clamp(0.0, f64::from(image.height() - crop_height))
                as u32;
            self.media_size = Size::new(
                ((visible_width / font_width).ceil() as u16).min(available.width),
                ((visible_height / font_height).ceil() as u16).min(available.height),
            );
            self.media_view.metrics = Some(MediaViewMetrics {
                scaled_width: scaled_width / font_width,
                scaled_height: scaled_height / font_height,
                viewport: available,
                area: Rect::default(),
            });
            self.clear_active_terminal_media();
            self.media_pending = MediaPending::default();
            self.deferred_sixel_preview = None;
            if immediate {
                self.media_state
                    .as_mut()
                    .expect("media worker is running")
                    .empty_protocol();
                match interaction_preview(
                    image,
                    crop_x,
                    crop_y,
                    crop_width,
                    crop_height,
                    self.media_size,
                ) {
                    Ok(preview) => {
                        self.media_preview = Some(preview);
                        self.media_error = None;
                    }
                    Err(error) => {
                        self.media_preview = None;
                        self.media_error = Some(format!(
                            "Could not render interactive media preview: {error}"
                        ));
                    }
                }
                self.effective_media_protocol = MediaPreviewProtocol::Halfblocks;
            } else {
                if self.media_preview.as_ref().is_none_or(|preview| {
                    protocol_kind(preview) != MediaPreviewProtocol::Halfblocks
                }) {
                    match interaction_preview(
                        image,
                        crop_x,
                        crop_y,
                        crop_width,
                        crop_height,
                        self.media_size,
                    ) {
                        Ok(preview) => self.media_preview = Some(preview),
                        Err(error) => {
                            self.media_preview = None;
                            self.media_error = Some(format!(
                                "Could not render progressive media preview: {error}"
                            ));
                        }
                    }
                }
                if final_protocol == MediaPreviewProtocol::Halfblocks {
                    self.media_state
                        .as_mut()
                        .expect("media worker is running")
                        .empty_protocol();
                    self.effective_media_protocol = MediaPreviewProtocol::Halfblocks;
                    self.media_error = None;
                } else {
                    if final_protocol == MediaPreviewProtocol::Sixel {
                        let view = if crop_x == 0
                            && crop_y == 0
                            && crop_width == image.width()
                            && crop_height == image.height()
                        {
                            Arc::clone(image)
                        } else {
                            Arc::new(image.crop_imm(crop_x, crop_y, crop_width, crop_height))
                        };
                        let font_size = self.media_picker.font_size();
                        let preview_key = sixel_cache_key(
                            &view,
                            self.media_size,
                            font_size,
                            self.media_sixel_tmux,
                            SixelEncoding::Preview,
                        );
                        let final_encoding = match sixel_quality {
                            SixelQuality::Fast => SixelEncoding::Fast,
                            SixelQuality::Quality => SixelEncoding::Quality,
                        };
                        let final_key = SixelCacheKey {
                            encoding: final_encoding,
                            ..preview_key
                        };
                        if let Some(sixel) = self.cached_sixel(final_key) {
                            let payload_bytes = sixel.data.len();
                            self.media_state
                                .as_mut()
                                .expect("media worker is running")
                                .empty_protocol();
                            self.media_preview = Some(Protocol::Sixel(sixel));
                            self.media_error = None;
                            crate::diagnostics::event(format!(
                                "media encode cache hit protocol={} payload_bytes={payload_bytes}",
                                final_encoding.diagnostic_name()
                            ));
                        } else {
                            let alternate_final_key = SixelCacheKey {
                                encoding: match final_encoding {
                                    SixelEncoding::Fast => SixelEncoding::Quality,
                                    SixelEncoding::Quality => SixelEncoding::Fast,
                                    SixelEncoding::Preview => unreachable!(),
                                },
                                ..preview_key
                            };
                            let preview_missing = if let Some(sixel) =
                                self.cached_sixel(alternate_final_key)
                            {
                                let payload_bytes = sixel.data.len();
                                self.media_preview = Some(Protocol::Sixel(sixel));
                                crate::diagnostics::event(format!(
                                    "media encode cache hit protocol={} payload_bytes={payload_bytes}",
                                    alternate_final_key.encoding.diagnostic_name()
                                ));
                                false
                            } else if let Some(sixel) = self.cached_sixel(preview_key) {
                                let payload_bytes = sixel.data.len();
                                self.media_preview = Some(Protocol::Sixel(sixel));
                                crate::diagnostics::event(format!(
                                    "media encode cache hit protocol={} payload_bytes={payload_bytes}",
                                    SixelEncoding::Preview.diagnostic_name()
                                ));
                                false
                            } else {
                                true
                            };
                            let request_sent = self
                                .media_state
                                .as_mut()
                                .expect("media worker is running")
                                .request_progressive_sixel(ProgressiveSixelRequest {
                                    image: view,
                                    font_size,
                                    size: self.media_size,
                                    background: media_background_color(),
                                    is_tmux: self.media_sixel_tmux,
                                    preview_key: preview_missing.then_some(preview_key),
                                    final_key,
                                });
                            if request_sent {
                                self.media_pending = MediaPending {
                                    preview: preview_missing,
                                    final_frame: true,
                                    final_encoding: Some(final_encoding),
                                };
                                self.media_error = None;
                            } else {
                                self.media_error = Some("Media preview worker stopped".to_owned());
                            }
                        }
                        self.effective_media_protocol = MediaPreviewProtocol::Sixel;
                    } else {
                        let view = image.crop_imm(crop_x, crop_y, crop_width, crop_height);
                        let mut picker = self.media_picker.clone();
                        let protocol_state = if final_protocol == MediaPreviewProtocol::Kitty {
                            let image_id = ((generation % 99_999) + 1) as u32;
                            StatefulProtocol::new(
                                view,
                                picker.font_size(),
                                None,
                                StatefulProtocolType::Kitty(StatefulKitty::new(image_id, false)),
                            )
                        } else {
                            picker.set_protocol_type(match final_protocol {
                                MediaPreviewProtocol::Auto | MediaPreviewProtocol::Halfblocks => {
                                    ProtocolType::Halfblocks
                                }
                                MediaPreviewProtocol::Kitty => ProtocolType::Kitty,
                                MediaPreviewProtocol::Iterm2 => ProtocolType::Iterm2,
                                MediaPreviewProtocol::Sixel => ProtocolType::Sixel,
                            });
                            picker.new_resize_protocol(view)
                        };
                        let request_sent = self
                            .media_state
                            .as_mut()
                            .expect("media worker is running")
                            .request_protocol(
                                protocol_state,
                                Resize::Scale(None),
                                self.media_size,
                                final_protocol,
                            );
                        self.media_pending.final_frame = request_sent;
                        self.media_pending.final_encoding = None;
                        self.effective_media_protocol = final_protocol;
                        self.media_error = if request_sent {
                            None
                        } else {
                            Some("Media preview worker stopped".to_owned())
                        };
                    }
                }
            }
            self.media_generation = Some(generation);
            self.media_protocol = Some(protocol);
            self.media_sixel_quality = Some(sixel_quality);
            self.media_available = available_size;
            self.media_applied_view_revision = self.media_view.revision;
            self.media_applied_immediate = immediate;
            self.media_frame_revision = self.media_frame_revision.wrapping_add(1);
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
        if let Some(metrics) = self.media_view.metrics.as_mut() {
            metrics.viewport = available;
            metrics.area = area;
        }
        if let Some(preview) = self.media_preview.as_ref() {
            return (
                area,
                protocol_kind(preview),
                self.media_frame_revision,
                MediaRenderState::Immediate(preview),
            );
        }
        let render_state = self
            .media_state
            .as_mut()
            .expect("media worker is running")
            .protocol_mut()
            .map_or(MediaRenderState::Empty, MediaRenderState::Threaded);
        (
            area,
            self.effective_media_protocol,
            self.media_frame_revision,
            render_state,
        )
    }

    #[cfg(test)]
    pub(crate) fn media_center_for_test(&self) -> (f64, f64) {
        (self.media_view.center_x, self.media_view.center_y)
    }

    pub(crate) fn shutdown(&mut self) {
        self.media_state.take();
        for worker in self.media_workers.drain(..) {
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
        let cache_content_matches = self.cache.as_ref().is_some_and(|cache| {
            let markdown_wrapped = render_markdown && input.wrapped;
            cache.generation == input.generation
                && cache.path == input.path
                && cache.is_diff == is_diff
                && cache.markdown == render_markdown
                && cache.markdown_wrapped == markdown_wrapped
                && cache.show_initial_diff_header == input.show_initial_diff_header
        });
        let cache_matches = cache_content_matches
            && self.cache.as_ref().is_some_and(|cache| {
                cache.width == input.width
                    || (!render_markdown && (cache.width >= 72) == (input.width >= 72))
            });
        if cache_matches
            && self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.width != input.width)
        {
            let cache = self.cache.as_mut().expect("preview cache exists");
            cache.width = input.width;
            cache.wrapped_line_starts = None;
            cache.wrapped_window = None;
            cache.wrapped_hunks = None;
        }
        if !cache_matches {
            let source_lines = (!render_markdown
                && matches!(input.content, PreviewContent::Source(_)))
            .then(|| SourceLineIndex::new(raw));
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
                    PreviewContent::Source(_) => {
                        source_lines
                            .as_ref()
                            .expect("source line index was initialized")
                            .count
                    }
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
                source_lines,
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
        if count == 0 {
            return Vec::new();
        }
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
                let source = input.content.as_str();
                let Some((checkpoint_line, byte_offset)) = cache
                    .source_lines
                    .as_ref()
                    .and_then(|lines| lines.checkpoint(window_start))
                else {
                    return Vec::new();
                };
                styled_source_window_from(
                    &source[byte_offset..],
                    input.path,
                    input.width,
                    checkpoint_line,
                    window_start.saturating_sub(checkpoint_line),
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

fn extend_editor_wrapped_lines(
    cache: &mut EditorPreviewCache,
    input: EditorPreviewInput<'_>,
    target: usize,
) {
    let target = target.min(input.line_starts.len());
    while cache.wrapped_line_starts.len().saturating_sub(1) < target {
        let line = cache.wrapped_line_starts.len().saturating_sub(1);
        let height =
            super::text::word_wrapped_height(editor_source_line(input, line), input.width.max(1));
        cache.wrapped_line_starts.push(
            cache
                .wrapped_line_starts
                .last()
                .copied()
                .unwrap_or_default()
                .saturating_add(height),
        );
        #[cfg(test)]
        {
            cache.wrapped_lines_computed += 1;
        }
    }
}

fn editor_source_line(input: EditorPreviewInput<'_>, line: usize) -> &str {
    let Some(start) = input.line_starts.get(line).copied() else {
        return "";
    };
    let mut end = input
        .line_starts
        .get(line.saturating_add(1))
        .copied()
        .unwrap_or(input.source.len());
    if end > start && input.source.as_bytes().get(end - 1) == Some(&b'\n') {
        end -= 1;
        if end > start && input.source.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
    }
    input.source.get(start..end).unwrap_or_default()
}

impl Drop for PreviewPresentation {
    fn drop(&mut self) {
        self.shutdown();
    }
}
