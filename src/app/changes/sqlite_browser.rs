use crate::repo_path::RepoPath;

pub(crate) const SQLITE_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqliteFocus {
    Objects,
    Rows,
}

#[derive(Debug)]
pub(crate) struct SqliteObject {
    pub(crate) kind: String,
    pub(crate) name: String,
}

#[derive(Debug)]
pub(crate) struct SqliteColumn {
    pub(crate) name: String,
    pub(crate) data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlitePageKey {
    pub(crate) object: String,
    pub(crate) offset: usize,
    pub(crate) cursor: Option<SqlitePageCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SqlitePageCursor {
    pub(crate) value: i64,
    pub(crate) reverse: bool,
}

#[derive(Debug)]
pub(crate) struct SqlitePage {
    pub(crate) key: SqlitePageKey,
    pub(crate) columns: Vec<SqliteColumn>,
    pub(crate) columns_truncated: bool,
    pub(crate) rows: Vec<Vec<String>>,
    pub(crate) has_next: bool,
    pub(crate) first_cursor: Option<i64>,
    pub(crate) last_cursor: Option<i64>,
}

#[derive(Debug)]
pub(crate) struct SqliteDatabase {
    pub(crate) file_size: u64,
    pub(crate) user_version: i64,
    pub(crate) objects: Vec<SqliteObject>,
    pub(crate) objects_truncated: bool,
    pub(crate) first_page: Option<Result<SqlitePage, String>>,
}

#[derive(Debug)]
pub(crate) struct SqliteBrowser {
    pub(crate) path: RepoPath,
    pub(crate) file_size: u64,
    pub(crate) user_version: i64,
    pub(crate) objects: Vec<SqliteObject>,
    pub(crate) objects_truncated: bool,
    pub(crate) selected_object: Option<usize>,
    pub(crate) object_scroll: usize,
    pub(crate) focus: SqliteFocus,
    pub(crate) active: bool,
    pub(crate) page: Option<SqlitePage>,
    pub(crate) page_loading: bool,
    pub(crate) page_error: Option<String>,
    pub(crate) selected_row: Option<usize>,
    pub(crate) row_scroll: usize,
    pub(crate) column_scroll: usize,
    pub(crate) generation: u64,
}

impl SqliteBrowser {
    pub(crate) fn new(path: RepoPath, database: SqliteDatabase, generation: u64) -> Self {
        let (page, page_error) = match database.first_page {
            Some(Ok(page)) => (Some(page), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        let selected_row = page
            .as_ref()
            .is_some_and(|page| !page.rows.is_empty())
            .then_some(0);
        Self {
            path,
            file_size: database.file_size,
            user_version: database.user_version,
            selected_object: (!database.objects.is_empty()).then_some(0),
            objects: database.objects,
            objects_truncated: database.objects_truncated,
            object_scroll: 0,
            focus: SqliteFocus::Objects,
            active: false,
            page,
            page_loading: false,
            page_error,
            selected_row,
            row_scroll: 0,
            column_scroll: 0,
            generation,
        }
    }

    pub(crate) fn selected_object(&self) -> Option<&SqliteObject> {
        self.selected_object
            .and_then(|index| self.objects.get(index))
    }

    pub(crate) fn select_object(&mut self, index: usize, viewport: usize) -> Option<SqlitePageKey> {
        if index >= self.objects.len() {
            return None;
        }
        self.selected_object = Some(index);
        ensure_visible(&mut self.object_scroll, index, viewport, self.objects.len());
        self.focus = SqliteFocus::Objects;
        self.begin_page(0)
    }

    pub(crate) fn move_object(&mut self, delta: isize, viewport: usize) -> Option<SqlitePageKey> {
        let index = move_index(self.selected_object, self.objects.len(), delta)?;
        if self.selected_object == Some(index) {
            return None;
        }
        self.select_object(index, viewport)
    }

    pub(crate) fn select_object_boundary(
        &mut self,
        last: bool,
        viewport: usize,
    ) -> Option<SqlitePageKey> {
        let index = if last {
            self.objects.len().checked_sub(1)?
        } else if self.objects.is_empty() {
            return None;
        } else {
            0
        };
        (self.selected_object != Some(index))
            .then(|| self.select_object(index, viewport))
            .flatten()
    }

    pub(crate) fn move_row(&mut self, delta: isize, viewport: usize) {
        let Some(page) = &self.page else {
            return;
        };
        let Some(index) = move_index(self.selected_row, page.rows.len(), delta) else {
            return;
        };
        self.selected_row = Some(index);
        ensure_visible(&mut self.row_scroll, index, viewport, page.rows.len());
    }

    pub(crate) fn select_row(&mut self, index: usize, viewport: usize) -> bool {
        let Some(page) = &self.page else {
            return false;
        };
        if index >= page.rows.len() {
            return false;
        }
        self.selected_row = Some(index);
        ensure_visible(&mut self.row_scroll, index, viewport, page.rows.len());
        self.focus = SqliteFocus::Rows;
        true
    }

    pub(crate) fn begin_page(&mut self, offset: usize) -> Option<SqlitePageKey> {
        let key = SqlitePageKey {
            object: self.selected_object()?.name.clone(),
            offset,
            cursor: None,
        };
        self.page = None;
        self.page_loading = true;
        self.page_error = None;
        self.selected_row = None;
        self.row_scroll = 0;
        self.column_scroll = 0;
        self.generation = self.generation.wrapping_add(1);
        Some(key)
    }

    pub(crate) fn apply_page(&mut self, key: &SqlitePageKey, result: Result<SqlitePage, String>) {
        let expected = self
            .selected_object()
            .is_some_and(|object| object.name == key.object);
        if !self.page_loading || !expected {
            return;
        }
        self.page_loading = false;
        match result {
            Ok(page) => {
                self.selected_row = (!page.rows.is_empty()).then_some(0);
                self.page = Some(page);
                self.page_error = None;
            }
            Err(error) => {
                self.page = None;
                self.page_error = Some(error);
            }
        }
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn page_by(&mut self, delta: isize) -> Option<SqlitePageKey> {
        let page = self.page.as_ref()?;
        let (offset, cursor) = if delta > 0 {
            if !page.has_next {
                return None;
            }
            (
                page.key.offset.saturating_add(SQLITE_PAGE_SIZE),
                page.last_cursor.map(|value| SqlitePageCursor {
                    value,
                    reverse: false,
                }),
            )
        } else {
            if page.key.offset == 0 {
                return None;
            }
            (
                page.key.offset.saturating_sub(SQLITE_PAGE_SIZE),
                page.first_cursor.map(|value| SqlitePageCursor {
                    value,
                    reverse: true,
                }),
            )
        };
        let mut key = self.begin_page(offset)?;
        key.cursor = cursor;
        Some(key)
    }

    pub(crate) fn shift_columns(&mut self, delta: isize) {
        let count = self.page.as_ref().map_or(0, |page| page.columns.len());
        self.column_scroll = add_signed(self.column_scroll, delta).min(count.saturating_sub(1));
    }
}

fn move_index(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(add_signed(selected.unwrap_or(0), delta).min(len - 1))
}

fn ensure_visible(scroll: &mut usize, selected: usize, viewport: usize, len: usize) {
    if selected < *scroll {
        *scroll = selected;
    } else if viewport > 0 && selected >= scroll.saturating_add(viewport) {
        *scroll = selected.saturating_add(1).saturating_sub(viewport);
    }
    *scroll = (*scroll).min(len.saturating_sub(viewport));
}

fn add_signed(value: usize, delta: isize) -> usize {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        value.saturating_add(delta as usize)
    }
}
