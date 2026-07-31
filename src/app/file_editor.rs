use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{filesystem, repo_path::RepoPath};

const MAX_EDITABLE_BYTES: usize = 1024 * 1024;
pub(crate) const TAB_WIDTH: usize = 4;

pub(crate) struct FileEditor {
    root: PathBuf,
    path: RepoPath,
    original: Vec<u8>,
    text: String,
    line_ending: &'static str,
    cursor: usize,
    preferred_column: Option<usize>,
    pub(crate) scroll_line: usize,
    pub(crate) scroll_column: usize,
    pub(crate) wrap_scroll_row: usize,
    pub(crate) discard_armed: bool,
}

impl FileEditor {
    pub(crate) fn open(root: &Path, path: RepoPath, line: usize, column: usize) -> Result<Self> {
        let file = filesystem::safe_regular_file(root, &path)?;
        let metadata = file
            .metadata()
            .with_context(|| format!("could not inspect {}", file.display()))?;
        if metadata.len() > MAX_EDITABLE_BYTES as u64 {
            bail!("{} is too large for inline editing", path.display());
        }
        if metadata.permissions().readonly() {
            bail!("{} is read-only", path.display());
        }
        let original =
            std::fs::read(&file).with_context(|| format!("could not read {}", file.display()))?;
        if original.len() > MAX_EDITABLE_BYTES {
            bail!("{} is too large for inline editing", path.display());
        }
        if original.contains(&0) {
            bail!("{} is a binary file", path.display());
        }
        let text = String::from_utf8(original.clone())
            .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        let line_ending = preferred_line_ending(&text);
        let mut editor = Self {
            root: root.to_owned(),
            path,
            original,
            text,
            line_ending,
            cursor: 0,
            preferred_column: None,
            scroll_line: 0,
            scroll_column: 0,
            wrap_scroll_row: 0,
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
        validate_text(&self.text)?;
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

    pub(crate) fn anchor_cursor_at(&mut self, row: usize, column: usize) {
        let (cursor_line, cursor_column) = self.cursor_position();
        self.scroll_line = cursor_line.saturating_sub(row);
        self.scroll_column = cursor_column.saturating_sub(column);
    }

    pub(crate) fn ensure_wrapped_cursor_visible(&mut self, cursor_row: usize, height: usize) {
        let height = height.max(1);
        if cursor_row < self.wrap_scroll_row {
            self.wrap_scroll_row = cursor_row;
        } else if cursor_row >= self.wrap_scroll_row.saturating_add(height) {
            self.wrap_scroll_row = cursor_row.saturating_sub(height - 1);
        }
        self.scroll_column = 0;
    }

    pub(crate) fn anchor_wrapped_cursor_at(&mut self, cursor_row: usize, row: usize) {
        self.wrap_scroll_row = cursor_row.saturating_sub(row);
        self.scroll_column = 0;
    }

    pub(crate) fn insert(&mut self, value: &str) -> Result<()> {
        let value = normalize_newlines(value, self.line_ending);
        self.validate_insertion(&value)?;
        self.text.insert_str(self.cursor, &value);
        self.cursor += value.len();
        self.changed();
        Ok(())
    }

    pub(crate) fn insert_char(&mut self, character: char) -> Result<()> {
        if character == '\0' {
            bail!("NUL bytes cannot be inserted into a text file");
        }
        if self.text.len().saturating_add(character.len_utf8()) > MAX_EDITABLE_BYTES {
            bail!("file would exceed the 1 MiB inline editing limit");
        }
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        self.changed();
        Ok(())
    }

    pub(crate) fn insert_newline(&mut self) -> Result<()> {
        let ending = self.line_ending;
        self.validate_insertion(ending)?;
        self.text.insert_str(self.cursor, ending);
        self.cursor += ending.len();
        self.changed();
        Ok(())
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

    fn validate_insertion(&self, value: &str) -> Result<()> {
        if value.contains('\0') {
            bail!("NUL bytes cannot be inserted into a text file");
        }
        if self.text.len().saturating_add(value.len()) > MAX_EDITABLE_BYTES {
            bail!("file would exceed the 1 MiB inline editing limit");
        }
        Ok(())
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
            width + (TAB_WIDTH - width % TAB_WIDTH)
        } else {
            width + UnicodeWidthStr::width(grapheme)
        }
    })
}

fn closest_column(text: &str, start: usize, end: usize, column: usize) -> usize {
    let mut best = (column, start);
    let mut width = 0usize;
    for (offset, grapheme) in text[start..end]
        .grapheme_indices(true)
        .chain(std::iter::once((end - start, "")))
    {
        let index = start + offset;
        let distance = width.abs_diff(column);
        if distance < best.0 {
            best = (distance, index);
        }
        if grapheme.is_empty() || width > column {
            break;
        }
        width += if grapheme == "\t" {
            TAB_WIDTH - width % TAB_WIDTH
        } else {
            UnicodeWidthStr::width(grapheme)
        };
    }
    best.1
}

fn normalize_newlines(value: &str, ending: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', ending)
}

fn preferred_line_ending(text: &str) -> &'static str {
    let crlf = text
        .as_bytes()
        .windows(2)
        .filter(|pair| *pair == b"\r\n")
        .count();
    let lf = text.bytes().filter(|byte| *byte == b'\n').count();
    if crlf > lf.saturating_sub(crlf) {
        "\r\n"
    } else {
        "\n"
    }
}

fn validate_text(text: &str) -> Result<()> {
    if text.len() > MAX_EDITABLE_BYTES {
        bail!("file exceeds the 1 MiB inline editing limit");
    }
    if text.contains('\0') {
        bail!("NUL bytes cannot be saved in a text file");
    }
    Ok(())
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
        editor.insert_newline().unwrap();
        editor.insert("pasted\nline").unwrap();
        assert_eq!(editor.text(), "first\r\npasted\r\nline\r\nsecond\r\n");
    }

    #[test]
    fn save_rejects_external_changes() {
        let (directory, mut editor) = editor("original\n");
        editor.insert("mine").unwrap();
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
        editor.insert("second\n").unwrap();

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

    #[test]
    fn rejects_insertions_that_make_the_buffer_non_editable() {
        let (_directory, mut editor) = editor("text");
        assert!(editor.insert("\0").is_err());
        assert!(editor.insert(&"x".repeat(MAX_EDITABLE_BYTES)).is_err());
        assert_eq!(editor.text(), "text");
    }

    #[test]
    fn keeps_the_original_dominant_line_ending_after_deletions() {
        let (_directory, mut mixed_editor) = editor("first\nsecond\r\nthird\n");
        mixed_editor.set_cursor(2, 0);
        mixed_editor.backspace();
        mixed_editor.set_cursor(0, 5);
        mixed_editor.insert_newline().unwrap();
        assert!(mixed_editor.text().starts_with("first\n\nsecond"));

        let (_directory, mut crlf_editor) = editor("first\r\nsecond\r\n");
        crlf_editor.set_cursor(1, 0);
        crlf_editor.backspace();
        crlf_editor.insert_newline().unwrap();
        assert_eq!(crlf_editor.text(), "first\r\nsecond\r\n");
    }

    #[test]
    fn rejects_files_that_are_read_only_at_open_or_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.txt");
        fs::write(&path, "text\n").unwrap();
        let mut editor = FileEditor::open(directory.path(), "file.txt".into(), 1, 0).unwrap();
        editor.insert("edited ").unwrap();

        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();

        assert!(editor.save().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "text\n");
        assert!(FileEditor::open(directory.path(), "file.txt".into(), 1, 0).is_err());
    }
}
