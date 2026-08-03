use std::ops::Range;

use crate::repo_path::RepoPath;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffLineKind {
    FileHeader,
    Hunk,
    Addition,
    Deletion,
    Context,
    Other,
}

#[derive(Clone, Debug)]
struct DiffLine {
    range: Range<usize>,
    payload: Range<usize>,
    kind: DiffLineKind,
    old_line: Option<u32>,
    new_line: Option<u32>,
    new_cursor: Option<u32>,
    path: Option<RepoPath>,
    first_new_line: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayRow {
    Line(usize),
    Separator,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffDocument {
    raw: String,
    lines: Vec<DiffLine>,
    compact: Vec<DisplayRow>,
    with_headers: Vec<DisplayRow>,
    hunk_count: usize,
}

impl DiffDocument {
    pub(crate) fn parse(raw: String) -> Self {
        Self::parse_inner(raw, true)
    }

    pub(crate) fn parse_untracked(raw: String) -> Self {
        Self::parse_inner(raw, false)
    }

    fn parse_inner(raw: String, patch: bool) -> Self {
        let ranges = line_ranges(&raw);
        let mut lines = Vec::with_capacity(ranges.len());
        let mut old_line = None;
        let mut new_line = None;
        let mut hunk_count = 0;

        for range in ranges {
            let text = &raw[range.clone()];
            let kind = if patch && text.starts_with("diff --git") {
                old_line = None;
                new_line = None;
                DiffLineKind::FileHeader
            } else if patch && text.starts_with("@@") {
                old_line = None;
                new_line = None;
                if let Some((old, new)) = parse_hunk_lines(text) {
                    old_line = Some(old.max(1));
                    new_line = Some(new.max(1));
                    hunk_count += 1;
                    DiffLineKind::Hunk
                } else {
                    DiffLineKind::Other
                }
            } else if old_line.is_some() && text.starts_with('+') && !text.starts_with("+++") {
                DiffLineKind::Addition
            } else if old_line.is_some() && text.starts_with('-') && !text.starts_with("---") {
                DiffLineKind::Deletion
            } else if old_line.is_some() && text.starts_with(' ') {
                DiffLineKind::Context
            } else {
                DiffLineKind::Other
            };
            let (line_old, line_new) = match kind {
                DiffLineKind::Addition => (None, new_line),
                DiffLineKind::Deletion => (old_line, None),
                DiffLineKind::Context => (old_line, new_line),
                _ => (None, None),
            };
            let payload = if matches!(
                kind,
                DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context
            ) {
                range.start.saturating_add(1)..range.end
            } else {
                range.clone()
            };
            lines.push(DiffLine {
                range,
                payload,
                kind,
                old_line: line_old,
                new_line: line_new,
                new_cursor: new_line,
                path: None,
                first_new_line: 1,
            });
            match kind {
                DiffLineKind::Addition => new_line = new_line.map(|line| line.saturating_add(1)),
                DiffLineKind::Deletion => old_line = old_line.map(|line| line.saturating_add(1)),
                DiffLineKind::Context => {
                    old_line = old_line.map(|line| line.saturating_add(1));
                    new_line = new_line.map(|line| line.saturating_add(1));
                }
                _ => {}
            }
        }

        assign_file_paths(&raw, &mut lines);
        let has_hunks = hunk_count > 0;
        let compact = project(&lines, has_hunks, false);
        let with_headers = project(&lines, has_hunks, true);
        Self {
            raw,
            lines,
            compact,
            with_headers,
            hunk_count,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.raw
    }

    pub(crate) fn display_len(&self, show_headers: bool) -> usize {
        self.projection(show_headers).len()
    }

    pub(crate) fn display_line(&self, row: usize, show_headers: bool) -> Option<&str> {
        let index = match self.projection(show_headers).get(row)? {
            DisplayRow::Line(index) => *index,
            DisplayRow::Separator => return Some(""),
        };
        Some(self.line_text(index))
    }

    pub(crate) fn display_kind(&self, row: usize, show_headers: bool) -> Option<DiffLineKind> {
        let DisplayRow::Line(index) = *self.projection(show_headers).get(row)? else {
            return None;
        };
        Some(self.lines[index].kind)
    }

    pub(crate) fn display_path(&self, row: usize, show_headers: bool) -> Option<&RepoPath> {
        let DisplayRow::Line(index) = *self.projection(show_headers).get(row)? else {
            return None;
        };
        self.lines[index].path.as_ref()
    }

    pub(crate) fn display_new_position(
        &self,
        row: usize,
        show_headers: bool,
    ) -> Option<(usize, &str)> {
        let DisplayRow::Line(index) = *self.projection(show_headers).get(row)? else {
            return None;
        };
        let line = &self.lines[index];
        if !matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Context) {
            return None;
        }
        Some((
            line.new_line?.max(1) as usize,
            &self.raw[line.payload.clone()],
        ))
    }

    pub(crate) fn display_file_position(
        &self,
        row: usize,
        show_headers: bool,
    ) -> Option<(RepoPath, usize, &str)> {
        if !show_headers {
            return None;
        }
        let DisplayRow::Line(index) = *self.projection(true).get(row)? else {
            return None;
        };
        let line = &self.lines[index];
        matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Context)
            .then(|| {
                Some((
                    line.path.clone()?,
                    line.new_line?.max(1) as usize,
                    &self.raw[line.payload.clone()],
                ))
            })
            .flatten()
    }

    pub(crate) fn display_file_header(
        &self,
        row: usize,
        show_headers: bool,
    ) -> Option<(RepoPath, usize)> {
        if !show_headers {
            return None;
        }
        let DisplayRow::Line(index) = *self.projection(true).get(row)? else {
            return None;
        };
        let line = &self.lines[index];
        (line.kind == DiffLineKind::FileHeader)
            .then(|| Some((line.path.clone()?, line.first_new_line)))
            .flatten()
    }

    pub(crate) fn wrapped_line_starts(&self, width: usize, show_headers: bool) -> Vec<usize> {
        let width = width.max(1);
        let numbered = width >= 72;
        let mut starts = Vec::with_capacity(self.display_len(show_headers).saturating_add(1));
        starts.push(0usize);
        for row in 0..self.display_len(show_headers) {
            let line = self.display_line(row, show_headers).unwrap_or_default();
            let kind = self.display_kind(row, show_headers);
            let prefix = if matches!(
                kind,
                Some(DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context)
            ) {
                usize::from(numbered) * 6 + 1
            } else {
                0
            };
            let payload = if prefix > 0 { &line[1..] } else { line };
            let height = super::super::text::word_wrapped_height(
                payload,
                width.saturating_sub(prefix).max(1),
            );
            starts.push(starts.last().copied().unwrap_or(0).saturating_add(height));
        }
        starts
    }

    pub(crate) fn hunk_rows(
        &self,
        wrapped_starts: Option<&[usize]>,
        wrapped: bool,
        show_headers: bool,
    ) -> (Vec<(usize, usize)>, usize) {
        let projection = self.projection(show_headers);
        let mut hunks = Vec::new();
        for (row, projected) in projection.iter().enumerate() {
            let DisplayRow::Line(index) = projected else {
                continue;
            };
            if self.lines[*index].kind == DiffLineKind::Hunk {
                let rendered = if wrapped {
                    let Some(rendered) = wrapped_starts.and_then(|starts| starts.get(row)).copied()
                    else {
                        return (Vec::new(), 0);
                    };
                    rendered
                } else {
                    row
                };
                hunks.push((hunks.len(), rendered));
            }
        }
        let height = if wrapped {
            let Some(height) = wrapped_starts.and_then(|starts| starts.last()).copied() else {
                return (Vec::new(), 0);
            };
            height
        } else {
            projection.len()
        };
        (hunks, height)
    }

    pub(crate) fn hunk_count(&self) -> usize {
        self.hunk_count
    }

    pub(crate) fn hunk_patch(&self, index: usize) -> Option<String> {
        let target = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.kind == DiffLineKind::Hunk)
            .nth(index)?
            .0;
        let file_start = self.lines[..=target]
            .iter()
            .rposition(|line| line.kind == DiffLineKind::FileHeader)
            .unwrap_or(0);
        let prefix_end = self.lines[file_start..target]
            .iter()
            .find(|line| self.raw[line.range.clone()].starts_with("@@"))
            .map_or(self.lines[target].range.start, |line| line.range.start);
        let hunk_end = self.lines[target + 1..]
            .iter()
            .find(|line| {
                line.kind == DiffLineKind::FileHeader
                    || self.raw[line.range.clone()].starts_with("@@")
            })
            .map_or(self.raw.len(), |line| line.range.start);
        let mut patch = self.raw[self.lines[file_start].range.start..prefix_end].to_owned();
        if !patch.is_empty() && !patch.ends_with('\n') {
            patch.push('\n');
        }
        patch.push_str(&self.raw[self.lines[target].range.start..hunk_end]);
        if !patch.ends_with('\n') {
            patch.push('\n');
        }
        Some(patch)
    }

    pub(crate) fn new_line_markers(&self, target: &RepoPath) -> Vec<(usize, char)> {
        let mut markers = Vec::new();
        let mut deletion_pending = false;
        let mut pending_line: Option<usize> = None;
        let has_file_headers = self
            .lines
            .iter()
            .any(|line| line.kind == DiffLineKind::FileHeader);
        let mut current_path = (!has_file_headers).then_some(target);
        for line in &self.lines {
            if line.kind == DiffLineKind::FileHeader {
                if deletion_pending
                    && current_path == Some(target)
                    && let Some(number) = pending_line
                {
                    markers.push((number.saturating_sub(1), '-'));
                }
                current_path = line.path.as_ref();
                deletion_pending = false;
                pending_line = None;
                continue;
            }
            if has_file_headers && line.path.as_ref() != Some(target) {
                continue;
            }
            match line.kind {
                DiffLineKind::Hunk => {
                    deletion_pending = false;
                    pending_line = None;
                }
                DiffLineKind::Deletion => {
                    deletion_pending = true;
                    pending_line = line.new_cursor.map(|line| line as usize);
                }
                DiffLineKind::Addition => {
                    let number = line.new_line.unwrap_or(1).saturating_sub(1) as usize;
                    markers.push((number, if deletion_pending { '~' } else { '+' }));
                    deletion_pending = false;
                    pending_line = None;
                }
                DiffLineKind::Context => {
                    if deletion_pending {
                        markers.push((line.new_line.unwrap_or(1).saturating_sub(1) as usize, '-'));
                    }
                    deletion_pending = false;
                    pending_line = None;
                }
                _ => {}
            }
        }
        if deletion_pending
            && current_path == Some(target)
            && let Some(number) = pending_line
        {
            markers.push((number.saturating_sub(1), '-'));
        }
        markers
    }

    pub(crate) fn line_numbers_at_display_row(
        &self,
        row: usize,
        show_headers: bool,
    ) -> (Option<u32>, Option<u32>) {
        let Some(DisplayRow::Line(index)) = self.projection(show_headers).get(row) else {
            return (None, None);
        };
        let line = &self.lines[*index];
        (line.old_line, line.new_line)
    }

    fn projection(&self, show_headers: bool) -> &[DisplayRow] {
        if show_headers {
            &self.with_headers
        } else {
            &self.compact
        }
    }

    fn line_text(&self, index: usize) -> &str {
        &self.raw[self.lines[index].range.clone()]
    }
}

fn line_ranges(raw: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for chunk in raw.split_inclusive('\n') {
        let end = start + chunk.len() - usize::from(chunk.ends_with('\n'));
        let end = end - usize::from(end > start && raw.as_bytes()[end - 1] == b'\r');
        ranges.push(start..end);
        start += chunk.len();
    }
    if start < raw.len() {
        ranges.push(start..raw.len());
    }
    ranges
}

fn project(lines: &[DiffLine], has_hunks: bool, show_headers: bool) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    let mut in_hunk = false;
    let mut seen_header = false;
    for (index, line) in lines.iter().enumerate() {
        if line.kind == DiffLineKind::FileHeader {
            in_hunk = false;
            if show_headers {
                if seen_header {
                    rows.push(DisplayRow::Separator);
                }
                seen_header = true;
                rows.push(DisplayRow::Line(index));
                continue;
            }
        }
        if has_hunks && !in_hunk && line.kind != DiffLineKind::Hunk {
            continue;
        }
        if line.kind == DiffLineKind::Hunk {
            if in_hunk {
                rows.push(DisplayRow::Separator);
            }
            in_hunk = true;
        }
        rows.push(DisplayRow::Line(index));
    }
    rows
}

fn assign_file_paths(raw: &str, lines: &mut [DiffLine]) {
    let headers = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.kind == DiffLineKind::FileHeader).then_some(index))
        .collect::<Vec<_>>();
    if headers.is_empty() {
        return;
    }
    for (position, start) in headers.iter().copied().enumerate() {
        let end = headers.get(position + 1).copied().unwrap_or(lines.len());
        let mut destination = None;
        let mut deleted = false;
        let mut first_new_line = 1;
        for line in &lines[start + 1..end] {
            let text = &raw[line.range.clone()];
            if destination.is_none()
                && let Some(path) = text.strip_prefix("+++ ")
            {
                if path == "/dev/null" {
                    deleted = true;
                } else {
                    destination = parse_git_diff_path(path);
                }
            }
            if destination.is_none()
                && let Some(path) = text.strip_prefix("rename to ")
            {
                destination = parse_git_diff_path(path);
            }
            if line.kind == DiffLineKind::Hunk {
                first_new_line = parse_hunk_lines(text).map_or(1, |(_, line)| line.max(1) as usize);
                break;
            }
        }
        if !deleted && destination.is_none() {
            let header = &raw[lines[start].range.clone()];
            destination = parse_git_diff_header_destination(header);
        }
        for line in &mut lines[start..end] {
            line.path = (!deleted).then(|| destination.clone()).flatten();
            line.first_new_line = first_new_line;
        }
    }
}

fn parse_git_diff_header_destination(header: &str) -> Option<RepoPath> {
    let value = header.strip_prefix("diff --git ")?;
    let tokenized = parse_git_diff_tokens(value).and_then(|(_, path)| {
        path.strip_prefix(b"b/")
            .and_then(|path| RepoPath::from_git_bytes(path).ok())
    });
    tokenized.or_else(|| {
        let old = value.strip_prefix("a/")?;
        old.match_indices(" b/").find_map(|(split, _)| {
            let (old_path, new_path) = old.split_at(split);
            let new_path = new_path.strip_prefix(" b/")?;
            (old_path == new_path)
                .then(|| RepoPath::from_git_bytes(new_path.as_bytes()).ok())
                .flatten()
        })
    })
}

fn parse_hunk_lines(line: &str) -> Option<(u32, u32)> {
    let mut fields = line.split_whitespace();
    fields.next()?;
    let old = fields
        .next()?
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = fields
        .next()?
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse()
        .ok()?;
    Some((old, new))
}

fn parse_git_diff_path(value: &str) -> Option<RepoPath> {
    let bytes = if value.starts_with('"') {
        parse_git_diff_token(value.as_bytes())?.0
    } else {
        value.as_bytes().to_vec()
    };
    let path = bytes.strip_prefix(b"b/").unwrap_or(bytes.as_slice());
    RepoPath::from_git_bytes(path).ok()
}

fn parse_git_diff_tokens(value: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let bytes = value.as_bytes();
    let (first, consumed) = parse_git_diff_token(bytes)?;
    let remaining = bytes.get(consumed..)?;
    let whitespace = remaining
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())?;
    let (second, _) = parse_git_diff_token(&remaining[whitespace..])?;
    Some((first, second))
}

fn parse_git_diff_token(value: &[u8]) -> Option<(Vec<u8>, usize)> {
    if value.first() != Some(&b'"') {
        let end = value
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(value.len());
        return Some((value[..end].to_vec(), end));
    }
    let mut output = Vec::new();
    let mut index = 1;
    while index < value.len() {
        match value[index] {
            b'"' => return Some((output, index + 1)),
            b'\\' => {
                index += 1;
                let escaped = *value.get(index)?;
                if (b'0'..=b'7').contains(&escaped) {
                    let mut byte = 0_u8;
                    let mut digits = 0;
                    while digits < 3
                        && value
                            .get(index)
                            .is_some_and(|byte| (b'0'..=b'7').contains(byte))
                    {
                        byte = byte.saturating_mul(8).saturating_add(value[index] - b'0');
                        index += 1;
                        digits += 1;
                    }
                    output.push(byte);
                    continue;
                }
                output.push(match escaped {
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 0x0b,
                    b'f' => 0x0c,
                    b'r' => b'\r',
                    other => other,
                });
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_files_hunks_separators_and_mappings() {
        let document = DiffDocument::parse(
            concat!(
                "diff --git a/one.rs b/one.rs\n",
                "--- a/one.rs\n+++ b/one.rs\n",
                "@@ -1,2 +3,2 @@\n context\n-old\n+new\n",
                "@@ -8 +9 @@\n+tail\n",
                "diff --git \"a/space name.rs\" \"b/space name.rs\"\n",
                "--- \"a/space name.rs\"\n+++ \"b/space\\040name.rs\"\n",
                "@@ -1 +4 @@\n+second\n",
            )
            .to_owned(),
        );
        assert_eq!(document.display_len(false), 9);
        assert_eq!(document.display_len(true), 12);
        assert_eq!(document.display_line(4, false), Some(""));
        assert_eq!(document.display_line(8, true), Some(""));
        assert_eq!(
            document.display_new_position(1, false).map(|value| value.0),
            Some(3)
        );
        assert_eq!(
            document.display_file_header(9, true),
            Some((RepoPath::from("space name.rs"), 4))
        );
        assert_eq!(document.hunk_count(), 3);
    }

    #[test]
    fn handles_deletions_and_no_hunk_documents() {
        let deleted = DiffDocument::parse(
            concat!(
                "diff --git a/gone.rs b/gone.rs\n",
                "deleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n",
                "@@ -1 +0,0 @@\n-old\n",
            )
            .to_owned(),
        );
        assert_eq!(deleted.display_file_header(0, true), None);
        let modes = DiffDocument::parse(
            "diff --git \"a/odd b/target\" \"b/odd b/target\"\nold mode 100644\nnew mode 100755\n"
                .to_owned(),
        );
        assert_eq!(modes.display_len(false), 3);
        assert_eq!(modes.display_len(true), 3);
        assert_eq!(
            modes.display_file_header(0, true),
            Some((RepoPath::from("odd b/target"), 1))
        );

        let unquoted = DiffDocument::parse(
            "diff --git a/space name.rs b/space name.rs\nold mode 100644\nnew mode 100755\n"
                .to_owned(),
        );
        assert_eq!(
            unquoted.display_file_header(0, true),
            Some((RepoPath::from("space name.rs"), 1))
        );

        let renamed = DiffDocument::parse(
            "diff --git a/old name.rs b/new name.rs\nsimilarity index 100%\nrename from old name.rs\nrename to new name.rs\n"
                .to_owned(),
        );
        assert_eq!(
            renamed.display_file_header(0, true),
            Some((RepoPath::from("new name.rs"), 1))
        );
    }

    #[test]
    fn maps_editor_markers_for_headerless_single_file_diffs() {
        let document = DiffDocument::parse("@@ -1 +1 @@\n-old\n+new\n".to_owned());

        assert_eq!(
            document.new_line_markers(&RepoPath::from("any.rs")),
            vec![(0, '~')]
        );
    }

    #[test]
    fn maps_trailing_deletions_to_the_destination_cursor() {
        let document = DiffDocument::parse("@@ -8,2 +8 @@\n keep\n-gone\n".to_owned());

        assert_eq!(
            document.line_numbers_at_display_row(2, false),
            (Some(9), None)
        );
        assert_eq!(
            document.new_line_markers(&RepoPath::from("any.rs")),
            vec![(8, '-')]
        );
    }

    #[test]
    fn untracked_source_does_not_parse_patch_shaped_content() {
        let document = DiffDocument::parse_untracked(
            "Untracked file: notes.txt\n\n@@ heading\n+literal\n".to_owned(),
        );

        assert_eq!(document.hunk_count(), 0);
        assert_eq!(document.display_len(false), 4);
        assert_eq!(document.display_line(2, false), Some("@@ heading"));
    }

    #[test]
    fn unsupported_hunks_reset_line_mapping_without_becoming_actions() {
        let document = DiffDocument::parse(
            "@@ -1 +1 @@\n+first\n@@@ -2,1 -2,1 +2,1 @@@\n+combined\n".to_owned(),
        );

        assert_eq!(document.hunk_count(), 1);
        assert_eq!(document.display_new_position(3, false), None);
    }

    #[test]
    fn extracts_stageable_patches_by_parsed_hunk_index() {
        let document = DiffDocument::parse(
            concat!(
                "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n",
                "@@@ -1,1 -1,1 +1,1 @@@\n--combined\n",
                "@@ -2 +2 @@\n-old\n+new\n",
            )
            .to_owned(),
        );

        let patch = document.hunk_patch(0).expect("ordinary hunk");
        assert!(patch.starts_with("diff --git a/file.rs b/file.rs\n"));
        assert!(!patch.contains("@@@"));
        assert!(patch.contains("@@ -2 +2 @@\n-old\n+new\n"));
        assert!(document.hunk_patch(1).is_none());
    }
}
