use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{filesystem, repo_path::RepoPath};

const MAX_EDITABLE_BYTES: usize = 1024 * 1024;
const MAX_HISTORY_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const TAB_WIDTH: usize = 4;

#[derive(Clone)]
struct Snapshot {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
    preferred_column: Option<usize>,
    changed_lines: BTreeSet<usize>,
}

impl Snapshot {
    fn memory_size(&self) -> usize {
        self.text.len()
    }
}

pub(crate) struct FileEditor {
    root: PathBuf,
    path: RepoPath,
    original: Vec<u8>,
    text: String,
    line_ending: &'static str,
    cursor: usize,
    selection_anchor: Option<usize>,
    preferred_column: Option<usize>,
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    history_bytes: usize,
    changed_lines: BTreeSet<usize>,
    pub(crate) scroll_line: usize,
    pub(crate) scroll_column: usize,
    pub(crate) wrap_scroll_row: usize,
    cursor_follow: bool,
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
            selection_anchor: None,
            preferred_column: None,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            history_bytes: 0,
            changed_lines: BTreeSet::new(),
            scroll_line: 0,
            scroll_column: 0,
            wrap_scroll_row: 0,
            cursor_follow: true,
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

    pub(crate) fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    pub(crate) fn selected_range(&self) -> Option<(usize, usize)> {
        self.selection_range()
    }

    pub(crate) fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection_range()?;
        Some(&self.text[start..end])
    }

    pub(crate) fn selected_line_range(&self) -> Option<(usize, usize)> {
        let (start, end) = self.selection_range()?;
        Some((
            line_number_at(&self.text, start),
            line_number_at(&self.text, end),
        ))
    }

    pub(crate) fn locally_changed_lines(&self) -> &BTreeSet<usize> {
        &self.changed_lines
    }

    pub(crate) fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.discard_armed = false;
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    pub(crate) fn begin_selection(&mut self) {
        self.selection_anchor = Some(self.cursor);
        self.preferred_column = None;
        self.discard_armed = false;
    }

    pub(crate) fn select_word_at_cursor(&mut self) -> bool {
        let index = if is_word_at(&self.text, self.cursor) {
            self.cursor
        } else if self.cursor > 0 {
            previous_cursor(&self.text, self.cursor)
        } else {
            return false;
        };
        if !is_word_at(&self.text, index) {
            return false;
        }

        let mut start = index;
        while start > 0 {
            let previous = previous_cursor(&self.text, start);
            if !is_word_at(&self.text, previous) {
                break;
            }
            start = previous;
        }
        let mut end = next_cursor(&self.text, index);
        while end < self.text.len() && is_word_at(&self.text, end) {
            end = next_cursor(&self.text, end);
        }
        self.selection_anchor = Some(start);
        self.cursor = end;
        self.preferred_column = None;
        self.discard_armed = false;
        true
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

    pub(crate) fn mark_saved(&mut self) {
        self.original = self.text.as_bytes().to_vec();
        self.changed_lines.clear();
        self.discard_armed = false;
    }

    pub(crate) fn refresh_from_disk(&mut self) -> Result<()> {
        let file = filesystem::safe_regular_file(&self.root, &self.path)?;
        let bytes =
            std::fs::read(&file).with_context(|| format!("could not read {}", file.display()))?;
        if bytes.len() > MAX_EDITABLE_BYTES {
            bail!("{} is too large for inline editing", self.path.display());
        }
        if bytes.contains(&0) {
            bail!("{} is a binary file", self.path.display());
        }
        let text = String::from_utf8(bytes.clone())
            .with_context(|| format!("{} is not valid UTF-8", self.path.display()))?;
        let (line, column) = self.cursor_position();
        self.original = bytes;
        self.text = text;
        self.line_ending = preferred_line_ending(&self.text);
        self.changed_lines.clear();
        self.set_cursor(line, column);
        self.history_bytes = self
            .history_bytes
            .saturating_sub(self.redo.iter().map(Snapshot::memory_size).sum());
        self.redo.clear();
        Ok(())
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

    pub(crate) fn should_follow_cursor(&self) -> bool {
        self.cursor_follow
    }

    pub(crate) fn set_cursor(&mut self, line: usize, column: usize) {
        let start = line_start_at(&self.text, line);
        let end = line_content_end(&self.text, start);
        self.cursor = closest_column(&self.text, start, end, column);
        self.selection_anchor = None;
        self.preferred_column = None;
        self.cursor_follow = true;
        self.discard_armed = false;
    }

    pub(crate) fn extend_cursor(&mut self, line: usize, column: usize) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
        let start = line_start_at(&self.text, line);
        let end = line_content_end(&self.text, start);
        self.cursor = closest_column(&self.text, start, end, column);
        self.preferred_column = None;
        self.cursor_follow = true;
        self.discard_armed = false;
    }

    pub(crate) fn scroll_viewport(&mut self, delta: isize, height: usize, wrapped: bool) {
        self.cursor_follow = false;
        if wrapped {
            self.wrap_scroll_row = adjust_scroll(self.wrap_scroll_row, delta);
            return;
        }
        let maximum = self.visible_line_count().saturating_sub(height.max(1));
        self.scroll_line = adjust_scroll(self.scroll_line, delta).min(maximum);
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
        self.replace_selection(&value)
    }

    pub(crate) fn insert_char(&mut self, character: char) -> Result<()> {
        if character == '\0' {
            bail!("NUL bytes cannot be inserted into a text file");
        }
        self.replace_selection(&character.to_string())
    }

    pub(crate) fn insert_newline(&mut self) -> Result<()> {
        let ending = self.line_ending;
        self.replace_selection(ending)
    }

    pub(crate) fn backspace(&mut self) {
        if self.has_selection() {
            let _ = self.replace_selection("");
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = previous_cursor(&self.text, self.cursor);
        self.replace_range(start, self.cursor, "");
    }

    pub(crate) fn delete(&mut self) {
        if self.has_selection() {
            let _ = self.replace_selection("");
            return;
        }
        if self.cursor == self.text.len() {
            return;
        }
        let end = next_cursor(&self.text, self.cursor);
        self.replace_range(self.cursor, end, "");
    }

    pub(crate) fn toggle_line_comments(
        &mut self,
        first_line: usize,
        last_line: usize,
    ) -> Result<()> {
        let marker = line_comment_marker(self.path.as_path()).ok_or_else(|| {
            anyhow::anyhow!(
                "line comments are not supported for {}",
                self.path.display()
            )
        })?;
        let last_line = last_line.min(self.visible_line_count().saturating_sub(1));
        let mut lines = Vec::new();
        for line in first_line.min(last_line)..=last_line {
            let start = line_start_at(&self.text, line);
            let end = line_content_end(&self.text, start);
            let indent = self.text[start..end]
                .bytes()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count();
            if start + indent < end {
                lines.push((start + indent, end));
            }
        }
        if lines.is_empty() {
            return Ok(());
        }

        let uncomment = lines
            .iter()
            .all(|(start, end)| self.text[*start..*end].starts_with(marker));
        let insertion = format!("{marker} ");
        if !uncomment {
            self.validate_insertion(&insertion.repeat(lines.len()))?;
        }
        self.record_edit();
        for (start, end) in lines.into_iter().rev() {
            if uncomment {
                let marker_end = start + marker.len();
                let remove_end =
                    marker_end + usize::from(self.text.as_bytes().get(marker_end) == Some(&b' '));
                self.text.replace_range(start..remove_end.min(end), "");
                if start < self.cursor {
                    self.cursor -= (remove_end - start).min(self.cursor - start);
                }
            } else {
                self.text.insert_str(start, &insertion);
                if start <= self.cursor {
                    self.cursor += insertion.len();
                }
            }
        }
        self.selection_anchor = None;
        self.mark_changed_lines(first_line, last_line);
        self.changed();
        Ok(())
    }

    pub(crate) fn indent_lines(
        &mut self,
        first_line: usize,
        last_line: usize,
        outdent: bool,
    ) -> Result<()> {
        let last_line = last_line.min(self.visible_line_count().saturating_sub(1));
        let mut edits = Vec::new();
        for line in first_line.min(last_line)..=last_line {
            let start = line_start_at(&self.text, line);
            let end = line_content_end(&self.text, start);
            if start == end {
                continue;
            }
            if outdent {
                let removed = if self.text.as_bytes().get(start) == Some(&b'\t') {
                    1
                } else {
                    self.text[start..end]
                        .bytes()
                        .take_while(|byte| *byte == b' ')
                        .count()
                        .min(TAB_WIDTH)
                };
                if removed > 0 {
                    edits.push((start, start + removed, 0));
                }
            } else {
                edits.push((start, start, 1));
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        let added = edits.iter().map(|(_, _, inserted)| inserted).sum::<usize>();
        let removed = edits
            .iter()
            .map(|(start, end, _)| end.saturating_sub(*start))
            .sum::<usize>();
        if self
            .text
            .len()
            .saturating_sub(removed)
            .saturating_add(added)
            > MAX_EDITABLE_BYTES
        {
            bail!("file would exceed the 1 MiB inline editing limit");
        }

        self.record_edit();
        for (start, end, inserted) in edits.into_iter().rev() {
            let value = if inserted == 0 { "" } else { "\t" };
            self.text.replace_range(start..end, value);
            adjust_offset(&mut self.cursor, start, end, inserted);
            if let Some(anchor) = &mut self.selection_anchor {
                adjust_offset(anchor, start, end, inserted);
            }
        }
        self.mark_changed_lines(first_line, last_line);
        self.changed();
        Ok(())
    }

    pub(crate) fn move_left_with_selection(&mut self, extend: bool) {
        if !extend && let Some((start, _)) = self.selection_range() {
            self.cursor = start;
            self.selection_anchor = None;
        } else {
            let target = previous_cursor(&self.text, self.cursor);
            self.move_to(target, extend);
        }
        self.moved_horizontally();
    }

    pub(crate) fn move_right_with_selection(&mut self, extend: bool) {
        if !extend && let Some((_, end)) = self.selection_range() {
            self.cursor = end;
            self.selection_anchor = None;
        } else {
            let target = next_cursor(&self.text, self.cursor);
            self.move_to(target, extend);
        }
        self.moved_horizontally();
    }

    pub(crate) fn move_home_with_selection(&mut self, extend: bool) {
        let target = line_start(&self.text, self.cursor);
        self.move_to(target, extend);
        self.moved_horizontally();
    }

    pub(crate) fn move_end_with_selection(&mut self, extend: bool) {
        let target = line_content_end(&self.text, line_start(&self.text, self.cursor));
        self.move_to(target, extend);
        self.moved_horizontally();
    }

    pub(crate) fn move_document_start_with_selection(&mut self, extend: bool) {
        self.move_to(0, extend);
        self.moved_horizontally();
    }

    pub(crate) fn move_document_end_with_selection(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
        self.moved_horizontally();
    }

    pub(crate) fn move_vertical_with_selection(&mut self, delta: isize, extend: bool) {
        let (line, column) = self.cursor_position();
        let column = *self.preferred_column.get_or_insert(column);
        let target = if delta < 0 {
            line.saturating_sub(delta.unsigned_abs())
        } else {
            line.saturating_add(delta as usize)
        }
        .min(self.visible_line_count().saturating_sub(1));
        if extend && self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        } else if !extend {
            self.selection_anchor = None;
        }
        self.set_cursor_preserving_column(target, column);
        self.discard_armed = false;
    }

    pub(crate) fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop_back() else {
            return false;
        };
        self.history_bytes = self.history_bytes.saturating_sub(snapshot.memory_size());
        let current = self.snapshot();
        self.history_bytes = self.history_bytes.saturating_add(current.memory_size());
        self.redo.push_back(current);
        self.restore_snapshot(snapshot);
        self.trim_history();
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop_back() else {
            return false;
        };
        self.history_bytes = self.history_bytes.saturating_sub(snapshot.memory_size());
        let current = self.snapshot();
        self.history_bytes = self.history_bytes.saturating_add(current.memory_size());
        self.undo.push_back(current);
        self.restore_snapshot(snapshot);
        self.trim_history();
        true
    }

    fn set_cursor_preserving_column(&mut self, line: usize, column: usize) {
        let start = line_start_at(&self.text, line);
        let end = line_content_end(&self.text, start);
        self.cursor = closest_column(&self.text, start, end, column);
        self.preferred_column = None;
        self.cursor_follow = true;
        self.discard_armed = false;
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

    fn replace_selection(&mut self, value: &str) -> Result<()> {
        let (start, end) = self.selection_range().unwrap_or((self.cursor, self.cursor));
        self.validate_replacement(start, end, value)?;
        if start == end && value.is_empty() {
            return Ok(());
        }
        self.replace_range(start, end, value);
        Ok(())
    }

    fn replace_range(&mut self, start: usize, end: usize, value: &str) {
        let first_line = line_number_at(&self.text, start);
        self.record_edit();
        self.text.replace_range(start..end, value);
        self.cursor = start.saturating_add(value.len());
        self.selection_anchor = None;
        self.mark_changed_lines(first_line, line_number_at(&self.text, self.cursor));
        self.changed();
    }

    fn validate_replacement(&self, start: usize, end: usize, value: &str) -> Result<()> {
        if value.contains('\0') {
            bail!("NUL bytes cannot be inserted into a text file");
        }
        if self
            .text
            .len()
            .saturating_sub(end.saturating_sub(start))
            .saturating_add(value.len())
            > MAX_EDITABLE_BYTES
        {
            bail!("file would exceed the 1 MiB inline editing limit");
        }
        Ok(())
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        (anchor != self.cursor).then_some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    fn move_to(&mut self, target: usize, extend: bool) {
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = target;
        self.cursor_follow = true;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
            preferred_column: self.preferred_column,
            changed_lines: self.changed_lines.clone(),
        }
    }

    fn restore_snapshot(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor.min(self.text.len());
        self.selection_anchor = snapshot
            .selection_anchor
            .filter(|anchor| *anchor <= self.text.len());
        self.preferred_column = snapshot.preferred_column;
        self.changed_lines = snapshot.changed_lines;
        self.cursor_follow = true;
        self.discard_armed = false;
    }

    fn record_edit(&mut self) {
        let snapshot = self.snapshot();
        self.history_bytes = self.history_bytes.saturating_add(snapshot.memory_size());
        self.undo.push_back(snapshot);
        self.history_bytes = self
            .history_bytes
            .saturating_sub(self.redo.iter().map(Snapshot::memory_size).sum());
        self.redo.clear();
        self.trim_history();
    }

    fn trim_history(&mut self) {
        while self.history_bytes > MAX_HISTORY_BYTES {
            let snapshot = self.undo.pop_front().or_else(|| self.redo.pop_front());
            let Some(snapshot) = snapshot else {
                break;
            };
            self.history_bytes = self.history_bytes.saturating_sub(snapshot.memory_size());
        }
    }

    fn changed(&mut self) {
        self.preferred_column = None;
        self.discard_armed = false;
    }

    fn mark_changed_lines(&mut self, first_line: usize, last_line: usize) {
        for line in first_line..=last_line.min(self.visible_line_count().saturating_sub(1)) {
            self.changed_lines.insert(line);
        }
    }

    fn moved_horizontally(&mut self) {
        self.preferred_column = None;
        self.discard_armed = false;
    }
}

fn is_word_at(text: &str, index: usize) -> bool {
    text.get(index..)
        .and_then(|value| value.chars().next())
        .is_some_and(|character| character == '_' || character.is_alphanumeric())
}

fn line_comment_marker(path: &Path) -> Option<&'static str> {
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if matches!(file_name.as_str(), "dockerfile" | "makefile" | "rakefile") {
        return Some("#");
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "c" | "cc" | "cpp" | "cs" | "dart" | "go" | "h" | "hpp" | "java" | "js" | "jsx" | "kt"
        | "kts" | "m" | "mm" | "php" | "proto" | "rs" | "scala" | "swift" | "ts" | "tsx"
        | "zig" => Some("//"),
        "bash" | "conf" | "ex" | "exs" | "fish" | "ini" | "jl" | "nix" | "pl" | "pm"
        | "properties" | "py" | "pyi" | "r" | "rb" | "sh" | "toml" | "yaml" | "yml" | "zsh" => {
            Some("#")
        }
        "hs" | "lua" | "sql" => Some("--"),
        "clj" | "cljs" | "cljc" | "el" | "lisp" | "scm" => Some(";"),
        "tex" => Some("%"),
        _ => None,
    }
}

fn line_start(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind('\n').map_or(0, |index| index + 1)
}

fn line_number_at(text: &str, cursor: usize) -> usize {
    text[..cursor].bytes().filter(|byte| *byte == b'\n').count()
}

fn adjust_scroll(value: usize, delta: isize) -> usize {
    if delta.is_negative() {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        value.saturating_add(delta as usize)
    }
}

fn adjust_offset(offset: &mut usize, start: usize, end: usize, inserted: usize) {
    if *offset >= end {
        *offset = offset
            .saturating_sub(end.saturating_sub(start))
            .saturating_add(inserted);
    } else if *offset >= start {
        *offset = start.saturating_add(inserted);
    }
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
    fn selection_replaces_logical_source_text() {
        let (_directory, mut editor) = editor("first\nsecond\n");
        editor.set_cursor(0, 0);
        editor.begin_selection();
        editor.extend_cursor(1, 3);

        assert_eq!(editor.selected_text(), Some("first\nsec"));
        editor.insert("replacement").unwrap();
        assert_eq!(editor.text(), "replacementond\n");
        assert!(!editor.has_selection());
    }

    #[test]
    fn undo_and_redo_are_kept_for_each_editor() {
        let (_directory, mut first) = editor("text\n");
        let (_other_directory, mut second) = editor("other\n");

        first.insert("one ").unwrap();
        first.insert("two ").unwrap();
        second.insert("changed ").unwrap();

        assert!(first.undo());
        assert_eq!(first.text(), "one text\n");
        assert!(first.undo());
        assert_eq!(first.text(), "text\n");
        assert_eq!(second.text(), "changed other\n");
        assert!(first.redo());
        assert_eq!(first.text(), "one text\n");
    }

    #[test]
    fn indenting_lines_is_one_undoable_edit() {
        let (_directory, mut editor) = editor("one\n  two\nthree\n");

        editor.indent_lines(0, 1, false).unwrap();
        assert_eq!(editor.text(), "\tone\n\t  two\nthree\n");
        assert!(editor.undo());
        assert_eq!(editor.text(), "one\n  two\nthree\n");

        editor.indent_lines(1, 1, true).unwrap();
        assert_eq!(editor.text(), "one\ntwo\nthree\n");
    }

    #[test]
    fn scrolling_the_viewport_does_not_move_the_cursor() {
        let (_directory, mut editor) = editor("one\ntwo\nthree\nfour\n");
        editor.set_cursor(0, 1);

        editor.scroll_viewport(3, 1, false);

        assert_eq!(editor.cursor_position(), (0, 1));
        assert_eq!(editor.scroll_line, 3);
    }

    #[test]
    fn refresh_keeps_undo_history_and_clears_redo() {
        let (directory, mut editor) = editor("original\n");
        editor.insert("edited ").unwrap();
        assert!(editor.undo());
        assert!(editor.redo());
        fs::write(directory.path().join("file.txt"), "formatted\n").unwrap();

        editor.refresh_from_disk().unwrap();

        assert_eq!(editor.text(), "formatted\n");
        assert!(!editor.redo());
        assert!(editor.undo());
        assert_eq!(editor.text(), "original\n");
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
        editor.move_document_end_with_selection(false);
        editor.insert("second\n").unwrap();

        editor.save().unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join("file.txt")).unwrap(),
            "first\nsecond\n"
        );
    }

    #[test]
    fn toggles_language_aware_line_comments() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("code.rs"),
            "fn main() {\n    first();\n\n    second();\n}\n",
        )
        .unwrap();
        let mut editor = FileEditor::open(directory.path(), "code.rs".into(), 2, 4).unwrap();

        editor.toggle_line_comments(1, 3).unwrap();
        assert_eq!(
            editor.text(),
            "fn main() {\n    // first();\n\n    // second();\n}\n"
        );
        assert_eq!(editor.cursor_position(), (1, 7));

        editor.toggle_line_comments(1, 3).unwrap();
        assert_eq!(
            editor.text(),
            "fn main() {\n    first();\n\n    second();\n}\n"
        );
        assert_eq!(editor.cursor_position(), (1, 4));
    }

    #[test]
    fn toggling_comments_is_one_undoable_edit() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("code.rs"), "first();\nsecond();\n").unwrap();
        let mut editor =
            FileEditor::open(directory.path(), RepoPath::from("code.rs"), 0, 0).unwrap();

        editor.toggle_line_comments(0, 1).unwrap();
        assert_eq!(editor.text(), "// first();\n// second();\n");
        assert!(editor.undo());
        assert_eq!(editor.text(), "first();\nsecond();\n");
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
