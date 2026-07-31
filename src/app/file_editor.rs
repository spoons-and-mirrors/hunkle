use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{filesystem, repo_path::RepoPath};

const MAX_EDITABLE_BYTES: u64 = 1024 * 1024;

pub(crate) struct FileEditor {
    root: PathBuf,
    path: RepoPath,
    original: Vec<u8>,
    text: String,
    cursor: usize,
    preferred_column: Option<usize>,
    pub(crate) scroll_line: usize,
    pub(crate) scroll_column: usize,
    pub(crate) discard_armed: bool,
}

impl FileEditor {
    pub(crate) fn open(root: &Path, path: RepoPath, line: usize, column: usize) -> Result<Self> {
        let file = filesystem::safe_regular_file(root, &path)?;
        let metadata = file
            .metadata()
            .with_context(|| format!("could not inspect {}", file.display()))?;
        if metadata.len() > MAX_EDITABLE_BYTES {
            bail!("{} is too large for inline editing", path.display());
        }
        let original =
            std::fs::read(&file).with_context(|| format!("could not read {}", file.display()))?;
        if original.len() > MAX_EDITABLE_BYTES as usize {
            bail!("{} is too large for inline editing", path.display());
        }
        if original.contains(&0) {
            bail!("{} is a binary file", path.display());
        }
        let text = String::from_utf8(original.clone())
            .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        let mut editor = Self {
            root: root.to_owned(),
            path,
            original,
            text,
            cursor: 0,
            preferred_column: None,
            scroll_line: 0,
            scroll_column: 0,
            discard_armed: false,
        };
        editor.set_cursor(line.saturating_sub(1), column);
        Ok(editor)
    }

    pub(crate) fn path(&self) -> &RepoPath {
        &self.path
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn dirty(&self) -> bool {
        self.text.as_bytes() != self.original
    }

    pub(crate) fn save(&self) -> Result<()> {
        filesystem::atomic_write_if_unchanged(
            &self.root,
            &self.path,
            &self.original,
            self.text.as_bytes(),
        )
    }

    pub(crate) fn cursor_position(&self) -> (usize, usize) {
        let start = line_start(&self.text, self.cursor);
        (
            self.text[..start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count(),
            display_width(&self.text[start..self.cursor]),
        )
    }

    pub(crate) fn visible_line_count(&self) -> usize {
        self.text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    pub(crate) fn set_cursor(&mut self, line: usize, column: usize) {
        let start = line_start_at(&self.text, line);
        let end = line_content_end(&self.text, start);
        self.cursor = closest_column(&self.text, start, end, column);
        self.preferred_column = None;
        self.discard_armed = false;
    }

    pub(crate) fn ensure_cursor_visible(&mut self, height: usize, width: usize) {
        let (line, column) = self.cursor_position();
        let height = height.max(1);
        let width = width.max(1);
        if line < self.scroll_line {
            self.scroll_line = line;
        } else if line >= self.scroll_line.saturating_add(height) {
            self.scroll_line = line.saturating_sub(height - 1);
        }
        if column < self.scroll_column {
            self.scroll_column = column;
        } else if column >= self.scroll_column.saturating_add(width) {
            self.scroll_column = column.saturating_sub(width - 1);
        }
    }

    pub(crate) fn insert(&mut self, value: &str) {
        let value = normalize_newlines(value, self.line_ending());
        self.text.insert_str(self.cursor, &value);
        self.cursor += value.len();
        self.changed();
    }

    pub(crate) fn insert_char(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        self.changed();
    }

    pub(crate) fn insert_newline(&mut self) {
        let ending = self.line_ending().to_owned();
        self.text.insert_str(self.cursor, &ending);
        self.cursor += ending.len();
        self.changed();
    }

    pub(crate) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = previous_cursor(&self.text, self.cursor);
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.changed();
    }

    pub(crate) fn delete(&mut self) {
        if self.cursor == self.text.len() {
            return;
        }
        let end = next_cursor(&self.text, self.cursor);
        self.text.drain(self.cursor..end);
        self.changed();
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = previous_cursor(&self.text, self.cursor);
        self.moved_horizontally();
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = next_cursor(&self.text, self.cursor);
        self.moved_horizontally();
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = line_start(&self.text, self.cursor);
        self.moved_horizontally();
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = line_content_end(&self.text, line_start(&self.text, self.cursor));
        self.moved_horizontally();
    }

    pub(crate) fn move_document_start(&mut self) {
        self.cursor = 0;
        self.moved_horizontally();
    }

    pub(crate) fn move_document_end(&mut self) {
        self.cursor = self.text.len();
        self.moved_horizontally();
    }

    pub(crate) fn move_vertical(&mut self, delta: isize) {
        let (line, column) = self.cursor_position();
        let column = *self.preferred_column.get_or_insert(column);
        let target = if delta < 0 {
            line.saturating_sub(delta.unsigned_abs())
        } else {
            line.saturating_add(delta as usize)
        }
        .min(self.visible_line_count().saturating_sub(1));
        self.set_cursor_preserving_column(target, column);
        self.discard_armed = false;
    }

    fn set_cursor_preserving_column(&mut self, line: usize, column: usize) {
        let start = line_start_at(&self.text, line);
        let end = line_content_end(&self.text, start);
        self.cursor = closest_column(&self.text, start, end, column);
    }

    fn line_ending(&self) -> &str {
        if self.text.as_bytes().windows(2).any(|pair| pair == b"\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }

    fn changed(&mut self) {
        self.preferred_column = None;
        self.discard_armed = false;
    }

    fn moved_horizontally(&mut self) {
        self.preferred_column = None;
        self.discard_armed = false;
    }
}

fn line_start(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind('\n').map_or(0, |index| index + 1)
}

fn line_start_at(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 1)
        .map_or(text.len(), |(index, _)| index + 1)
}

fn line_content_end(text: &str, start: usize) -> usize {
    let newline = text[start..]
        .find('\n')
        .map_or(text.len(), |index| start + index);
    if newline > start && text.as_bytes()[newline - 1] == b'\r' {
        newline - 1
    } else {
        newline
    }
}

fn previous_cursor(text: &str, cursor: usize) -> usize {
    let previous = text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index);
    if cursor >= 2 && &text.as_bytes()[cursor - 2..cursor] == b"\r\n" {
        cursor - 2
    } else {
        previous
    }
}

fn next_cursor(text: &str, cursor: usize) -> usize {
    if text.as_bytes().get(cursor..cursor.saturating_add(2)) == Some(b"\r\n") {
        return cursor + 2;
    }
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |grapheme| cursor + grapheme.len())
}

fn display_width(value: &str) -> usize {
    value.graphemes(true).fold(0, |width, grapheme| {
        if grapheme == "\t" {
            width + (4 - width % 4)
        } else {
            width + UnicodeWidthStr::width(grapheme)
        }
    })
}

fn closest_column(text: &str, start: usize, end: usize, column: usize) -> usize {
    let mut best = (column, start);
    for (offset, grapheme) in text[start..end]
        .grapheme_indices(true)
        .chain(std::iter::once((end - start, "")))
    {
        let index = start + offset;
        let distance = display_width(&text[start..index]).abs_diff(column);
        if distance < best.0 {
            best = (distance, index);
        }
        if grapheme.is_empty() || display_width(&text[start..index]) > column {
            break;
        }
    }
    best.1
}

fn normalize_newlines(value: &str, ending: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', ending)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn editor(content: &str) -> (tempfile::TempDir, FileEditor) {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("file.txt"), content).unwrap();
        let editor = FileEditor::open(directory.path(), "file.txt".into(), 1, 0).unwrap();
        (directory, editor)
    }

    #[test]
    fn edits_graphemes_without_splitting_crlf() {
        let (_directory, mut editor) = editor("a\r\ne\u{301}\r\n");
        editor.set_cursor(1, 1);
        editor.backspace();
        assert_eq!(editor.text(), "a\r\n\r\n");
        editor.backspace();
        assert_eq!(editor.text(), "a\r\n");
    }

    #[test]
    fn inserted_lines_follow_existing_line_endings() {
        let (_directory, mut editor) = editor("first\r\nsecond\r\n");
        editor.set_cursor(0, 5);
        editor.insert_newline();
        editor.insert("pasted\nline");
        assert_eq!(editor.text(), "first\r\npasted\r\nline\r\nsecond\r\n");
    }

    #[test]
    fn save_rejects_external_changes() {
        let (directory, mut editor) = editor("original\n");
        editor.insert("mine");
        fs::write(directory.path().join("file.txt"), "theirs\n").unwrap();

        assert!(editor.save().is_err());
        assert_eq!(
            fs::read_to_string(directory.path().join("file.txt")).unwrap(),
            "theirs\n"
        );
    }

    #[test]
    fn save_atomically_writes_the_edited_utf8() {
        let (directory, mut editor) = editor("first\n");
        editor.move_document_end();
        editor.insert("second\n");

        editor.save().unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join("file.txt")).unwrap(),
            "first\nsecond\n"
        );
    }

    #[test]
    fn rejects_binary_and_invalid_utf8_files() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("binary"), b"a\0b").unwrap();
        fs::write(directory.path().join("invalid"), b"a\xffb").unwrap();

        assert!(FileEditor::open(directory.path(), "binary".into(), 1, 0).is_err());
        assert!(FileEditor::open(directory.path(), "invalid".into(), 1, 0).is_err());
    }
}
