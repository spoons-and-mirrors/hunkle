use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{execute, style::Print};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Style,
};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Default)]
pub struct SelectionState {
    anchor: Option<Position>,
    cursor: Option<Position>,
    region: Option<Rect>,
    active: bool,
    dragged: bool,
    screen: Option<ScreenSnapshot>,
    selected: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectionOutcome {
    Click,
    Selected(Option<String>),
}

#[derive(Debug)]
struct ScreenSnapshot {
    area: Rect,
    rows: Vec<ScreenRow>,
}

#[derive(Debug)]
struct ScreenRow {
    text: String,
    symbols: Vec<ScreenSymbol>,
}

#[derive(Debug)]
struct ScreenSymbol {
    x: u16,
    width: u16,
    start: usize,
    end: usize,
}

impl SelectionState {
    pub fn begin(&mut self, point: Position, region: Rect) {
        self.anchor = Some(point);
        self.cursor = Some(point);
        self.region = Some(region);
        self.active = true;
        self.dragged = false;
        self.selected = None;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn has_selection(&self) -> bool {
        self.anchor.is_some() && self.dragged
    }

    pub fn update(&mut self, point: Position) {
        let Some(anchor) = self.anchor else {
            return;
        };
        self.cursor = Some(point);
        self.dragged |= point != anchor;
    }

    pub fn finish(&mut self, point: Position) -> SelectionOutcome {
        self.update(point);
        self.active = false;
        if !self.dragged {
            self.clear();
            return SelectionOutcome::Click;
        }
        self.selected = self.selected_text().filter(|text| !text.is_empty());
        SelectionOutcome::Selected(self.selected.clone())
    }

    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
        self.region = None;
        self.active = false;
        self.dragged = false;
        self.selected = None;
    }

    pub fn render(&self, buffer: &mut Buffer, style: Style) {
        let Some((region, start, end)) = self.bounds(buffer.area) else {
            return;
        };
        for y in start.y..=end.y {
            let left = if y == start.y { start.x } else { region.x };
            let right = if y == end.y {
                end.x
            } else {
                region.right().saturating_sub(1)
            };
            buffer.set_style(
                Rect::new(left, y, right.saturating_sub(left).saturating_add(1), 1),
                style,
            );
        }
    }

    pub fn capture(&mut self, buffer: &Buffer) {
        if self
            .screen
            .as_ref()
            .is_some_and(|screen| screen.area != buffer.area)
        {
            self.clear();
        }
        let mut rows = Vec::with_capacity(usize::from(buffer.area.height));
        for y in buffer.area.y..buffer.area.bottom() {
            let mut text = String::with_capacity(usize::from(buffer.area.width));
            let mut symbols = Vec::with_capacity(usize::from(buffer.area.width));
            let mut x = buffer.area.x;
            while x < buffer.area.right() {
                let row = usize::from(y.saturating_sub(buffer.area.y));
                let column = usize::from(x.saturating_sub(buffer.area.x));
                let index = row * usize::from(buffer.area.width) + column;
                let symbol = buffer.content[index].symbol();
                let width = UnicodeWidthStr::width(symbol).max(1) as u16;
                let start = text.len();
                text.push_str(symbol);
                symbols.push(ScreenSymbol {
                    x,
                    width,
                    start,
                    end: text.len(),
                });
                x = x.saturating_add(width);
            }
            rows.push(ScreenRow { text, symbols });
        }
        self.screen = Some(ScreenSnapshot {
            area: buffer.area,
            rows,
        });
    }

    pub fn needs_capture(&self, area: Rect) -> bool {
        self.active
            && self
                .screen
                .as_ref()
                .is_none_or(|screen| screen.area != area)
    }

    pub fn discard_inactive_capture(&mut self) {
        if !self.active {
            self.screen = None;
        }
    }

    pub fn screen_area(&self) -> Option<Rect> {
        self.screen.as_ref().map(|screen| screen.area)
    }

    fn bounds(&self, screen_area: Rect) -> Option<(Rect, Position, Position)> {
        if !self.dragged || screen_area.is_empty() {
            return None;
        }
        let area = intersection(self.region?, screen_area);
        if area.is_empty() {
            return None;
        }
        let anchor = clamp(self.anchor?, area);
        let cursor = clamp(self.cursor?, area);
        if (anchor.y, anchor.x) <= (cursor.y, cursor.x) {
            Some((area, anchor, cursor))
        } else {
            Some((area, cursor, anchor))
        }
    }

    fn selected_text(&self) -> Option<String> {
        let screen = self.screen.as_ref()?;
        let (region, start, end) = self.bounds(screen.area)?;
        let mut result = String::new();

        for y in start.y..=end.y {
            if y > start.y {
                result.push('\n');
            }
            let left = if y == start.y { start.x } else { region.x };
            let right = if y == end.y {
                end.x
            } else {
                region.right().saturating_sub(1)
            };
            let mut line = String::new();
            let row = screen.row(y)?;
            for symbol in &row.symbols {
                let symbol_right = symbol.x.saturating_add(symbol.width.saturating_sub(1));
                if symbol.x > right {
                    break;
                }
                if symbol_right >= left {
                    line.push_str(&row.text[symbol.start..symbol.end]);
                }
            }
            result.push_str(line.trim_end_matches(' '));
        }
        Some(result)
    }
}

impl ScreenSnapshot {
    fn row(&self, y: u16) -> Option<&ScreenRow> {
        self.rows.get(usize::from(y.saturating_sub(self.area.y)))
    }
}

fn intersection(left: Rect, right: Rect) -> Rect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left.right().min(right.right());
    let bottom = left.bottom().min(right.bottom());
    Rect::new(x, y, right_edge.saturating_sub(x), bottom.saturating_sub(y))
}

fn clamp(point: Position, area: Rect) -> Position {
    Position::new(
        point.x.clamp(area.x, area.right().saturating_sub(1)),
        point.y.clamp(area.y, area.bottom().saturating_sub(1)),
    )
}

pub fn copy_to_clipboard(text: &str) -> io::Result<()> {
    if copy_with_native_tool(text) {
        return Ok(());
    }
    let encoded = STANDARD.encode(text.as_bytes());
    let mut stdout = io::stdout();
    execute!(stdout, Print(format!("\u{1b}]52;c;{encoded}\u{7}")))?;
    stdout.flush()
}

fn copy_with_native_tool(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    if run_clipboard_command("pbcopy", &[], text) {
        return true;
    }

    #[cfg(target_os = "windows")]
    if run_clipboard_command("clip.exe", &[], text) {
        return true;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            && run_clipboard_command("wl-copy", &[], text)
        {
            return true;
        }
        if std::env::var_os("DISPLAY").is_some()
            && (run_clipboard_command("xclip", &["-selection", "clipboard", "-in"], text)
                || run_clipboard_command("xsel", &["--clipboard", "--input"], text))
        {
            return true;
        }
    }

    false
}

fn run_clipboard_command(command: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    let succeeded = child.wait().is_ok_and(|status| status.success());
    wrote && succeeded
}

#[cfg(test)]
mod tests {
    use ratatui::{buffer::Buffer, layout::Rect};

    use super::*;

    #[test]
    fn extracts_forward_and_reverse_multiline_selections() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 3));
        buffer.set_string(0, 0, "alpha", Style::default());
        buffer.set_string(0, 1, "beta", Style::default());
        buffer.set_string(0, 2, "gamma", Style::default());
        let mut selection = SelectionState::default();
        selection.capture(&buffer);

        selection.begin(Position::new(2, 0), buffer.area);
        selection.update(Position::new(1, 2));
        assert_eq!(
            selection.finish(Position::new(1, 2)),
            SelectionOutcome::Selected(Some("pha\nbeta\nga".to_owned()))
        );

        selection.begin(Position::new(1, 2), buffer.area);
        selection.update(Position::new(2, 0));
        assert_eq!(
            selection.finish(Position::new(2, 0)),
            SelectionOutcome::Selected(Some("pha\nbeta\nga".to_owned()))
        );
    }

    #[test]
    fn copies_wide_symbols_once() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        buffer.set_string(0, 0, "a界b", Style::default());
        let mut selection = SelectionState::default();
        selection.capture(&buffer);
        selection.begin(Position::new(1, 0), buffer.area);
        selection.update(Position::new(3, 0));

        assert_eq!(
            selection.finish(Position::new(3, 0)),
            SelectionOutcome::Selected(Some("界b".to_owned()))
        );
    }

    #[test]
    fn blank_drag_is_still_a_selection_gesture() {
        let buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        let mut selection = SelectionState::default();
        selection.capture(&buffer);
        selection.begin(Position::new(0, 0), buffer.area);

        assert_eq!(
            selection.finish(Position::new(2, 0)),
            SelectionOutcome::Selected(None)
        );
    }

    #[test]
    fn multiline_selection_stays_inside_its_starting_region() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 2));
        buffer.set_string(0, 0, "left  one", Style::default());
        buffer.set_string(0, 1, "left  two", Style::default());
        let mut selection = SelectionState::default();
        selection.capture(&buffer);
        selection.begin(Position::new(6, 0), Rect::new(6, 0, 6, 2));

        assert_eq!(
            selection.finish(Position::new(8, 1)),
            SelectionOutcome::Selected(Some("one\ntwo".to_owned()))
        );
    }
}
