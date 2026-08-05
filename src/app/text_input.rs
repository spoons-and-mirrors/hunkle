use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BLINK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditOutcome {
    Unhandled,
    Navigated,
    Edited,
}

#[derive(Debug)]
pub(crate) struct TextInput {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
    cursor_visible: bool,
    next_blink: Instant,
}

impl Default for TextInput {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: None,
            cursor_visible: true,
            next_blink: Instant::now() + BLINK_INTERVAL,
        }
    }
}

impl TextInput {
    pub(crate) fn handle_edit_key(&mut self, key: KeyEvent) -> EditOutcome {
        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_all();
                EditOutcome::Navigated
            }
            KeyCode::Left => {
                self.move_left();
                EditOutcome::Navigated
            }
            KeyCode::Right => {
                self.move_right();
                EditOutcome::Navigated
            }
            KeyCode::Home => {
                self.move_home();
                EditOutcome::Navigated
            }
            KeyCode::End => {
                self.move_end();
                EditOutcome::Navigated
            }
            KeyCode::Delete => {
                self.delete();
                EditOutcome::Edited
            }
            KeyCode::Backspace => {
                self.backspace();
                EditOutcome::Edited
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert_char(character);
                EditOutcome::Edited
            }
            _ => EditOutcome::Unhandled,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then_some({
            if anchor < self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    pub(crate) fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub(crate) fn visual_cursor_row(&self, width: usize) -> usize {
        visual_position(&self.text, self.cursor, width).0
    }

    pub(crate) fn visual_height(&self, width: usize) -> usize {
        visual_position(&self.text, self.text.len(), width).0 + 1
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub(crate) fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn insert(&mut self, text: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.reset_blink();
    }

    pub(crate) fn insert_single_line(&mut self, text: &str) {
        self.insert(
            &text
                .chars()
                .filter(|character| !matches!(character, '\r' | '\n'))
                .collect::<String>(),
        );
    }

    pub(crate) fn insert_char(&mut self, character: char) {
        self.delete_selection();
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        self.reset_blink();
    }

    pub(crate) fn backspace(&mut self) {
        if self.delete_selection() {
            self.reset_blink();
            return;
        }
        let start = previous_boundary(&self.text, self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.reset_blink();
    }

    pub(crate) fn delete(&mut self) {
        if self.delete_selection() {
            self.reset_blink();
            return;
        }
        let end = next_boundary(&self.text, self.cursor);
        self.text.drain(self.cursor..end);
        self.reset_blink();
    }

    pub(crate) fn delete_word(&mut self) {
        if self.delete_selection() {
            self.reset_blink();
            return;
        }

        let mut start = self.cursor;
        while start > 0 {
            let previous = previous_boundary(&self.text, start);
            if !self.text[previous..start].chars().all(char::is_whitespace) {
                break;
            }
            start = previous;
        }
        let word = start > 0 && {
            let previous = previous_boundary(&self.text, start);
            self.text[previous..start].chars().all(is_word_character)
        };
        while start > 0 {
            let previous = previous_boundary(&self.text, start);
            let character = self.text[previous..start]
                .chars()
                .next()
                .expect("character boundary");
            if character.is_whitespace() || is_word_character(character) != word {
                break;
            }
            start = previous;
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.reset_blink();
    }

    pub(crate) fn move_left(&mut self) {
        if let Some((start, _)) = self.selection() {
            self.cursor = start;
        } else {
            self.cursor = previous_boundary(&self.text, self.cursor);
        }
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn move_right(&mut self) {
        if let Some((_, end)) = self.selection() {
            self.cursor = end;
        } else {
            self.cursor = next_boundary(&self.text, self.cursor);
        }
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |index| self.cursor + index);
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn move_up(&mut self, width: usize) {
        self.move_vertical(width, -1);
    }

    pub(crate) fn move_down(&mut self, width: usize) {
        self.move_vertical(width, 1);
    }

    pub(crate) fn set_cursor_at_visual_position(
        &mut self,
        width: usize,
        row: usize,
        column: usize,
    ) {
        self.cursor = closest_visual_cursor(&self.text, width, row, column);
        self.anchor = None;
        self.reset_blink();
    }

    pub(crate) fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.text.len();
        self.reset_blink();
    }

    pub(crate) fn focus(&mut self) {
        self.reset_blink();
    }

    pub(crate) fn poll_blink(&mut self, focused: bool) -> bool {
        if !focused {
            self.cursor_visible = true;
            self.next_blink = Instant::now() + BLINK_INTERVAL;
            return false;
        }
        let now = Instant::now();
        if now < self.next_blink {
            return false;
        }
        self.cursor_visible = !self.cursor_visible;
        self.next_blink = now + BLINK_INTERVAL;
        true
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.text.drain(start..end);
        self.cursor = start;
        self.anchor = None;
        true
    }

    fn reset_blink(&mut self) {
        self.cursor_visible = true;
        self.next_blink = Instant::now() + BLINK_INTERVAL;
    }

    fn move_vertical(&mut self, width: usize, direction: isize) {
        let (row, column) = visual_position(&self.text, self.cursor, width);
        let target_row = if direction < 0 {
            row.saturating_sub(1)
        } else {
            row.saturating_add(1)
        };
        if target_row == row {
            return;
        }
        let cursor = closest_visual_cursor(&self.text, width, target_row, column);
        if visual_position(&self.text, cursor, width).0 == target_row {
            self.cursor = cursor;
            self.anchor = None;
            self.reset_blink();
        }
    }
}

fn closest_visual_cursor(text: &str, width: usize, row: usize, column: usize) -> usize {
    let width = width.max(1);
    let mut visual_row = 0;
    let mut line_width = 0;
    let mut best = (usize::MAX, usize::MAX, 0);
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        let candidate = (
            (visual_row + line_width / width).abs_diff(row),
            (line_width % width).abs_diff(column),
            index,
        );
        if candidate < best {
            best = candidate;
        }
        if character == '\n' {
            visual_row += line_width.saturating_sub(1) / width + 1;
            line_width = 0;
        } else {
            line_width += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    best.2
}

fn visual_position(text: &str, cursor: usize, width: usize) -> (usize, usize) {
    let width = width.max(1);
    let mut row = 0;
    let mut lines = text[..cursor].split('\n').peekable();
    while let Some(line) = lines.next() {
        let line_width = UnicodeWidthStr::width(line);
        if lines.peek().is_some() {
            row += line_width.saturating_sub(1) / width + 1;
        } else {
            row += line_width / width;
            return (row, line_width % width);
        }
    }
    (row, 0)
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(index, _)| cursor + index)
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{EditOutcome, TextInput};

    #[test]
    fn edits_unicode_and_replaces_selection() {
        let mut input = TextInput::default();
        input.set("one café");
        input.move_left();
        input.backspace();
        assert_eq!(input.text(), "one caé");

        input.select_all();
        input.insert("replacement");
        assert_eq!(input.text(), "replacement");
        assert_eq!(input.cursor(), input.text().len());
    }

    #[test]
    fn deletes_the_previous_word_and_whitespace() {
        let mut input = TextInput::default();
        input.set("subject with words   ");
        input.delete_word();
        assert_eq!(input.text(), "subject with ");
        input.delete_word();
        assert_eq!(input.text(), "subject ");
    }

    #[test]
    fn navigates_wrapped_visual_lines() {
        let mut input = TextInput::default();
        input.set("abcdef\nghij");

        input.move_up(4);
        assert_eq!(input.cursor(), 7);
        input.move_up(4);
        assert_eq!(input.cursor(), 4);
        input.move_down(4);
        assert_eq!(input.cursor(), 7);
        input.move_down(4);
        assert_eq!(input.cursor(), input.text().len());

        input.set_cursor_at_visual_position(4, 0, 3);
        assert_eq!(input.cursor(), 3);
        input.set_cursor_at_visual_position(4, 1, 1);
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn blinks_only_while_focused() {
        let mut input = TextInput {
            next_blink: std::time::Instant::now(),
            ..TextInput::default()
        };
        assert!(input.poll_blink(true));
        assert!(!input.cursor_visible());
        assert!(!input.poll_blink(false));
        assert!(input.cursor_visible());
    }

    #[test]
    fn handles_common_editing_keys() {
        let mut input = TextInput::default();
        assert_eq!(
            input.handle_edit_key(KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE)),
            EditOutcome::Edited
        );
        assert_eq!(
            input.handle_edit_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            EditOutcome::Navigated
        );
        assert_eq!(
            input.handle_edit_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            EditOutcome::Unhandled
        );
        input.insert_single_line(" one\r\ntwo");
        assert_eq!(input.text(), " onetwoé");
    }
}
