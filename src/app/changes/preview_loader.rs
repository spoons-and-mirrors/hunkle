use std::{
    collections::VecDeque,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    sync::mpsc::{self, Receiver, SyncSender},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use image::{DynamicImage, ImageReader, Limits as ImageLimits};
use rusqlite::{Connection, OpenFlags, types::ValueRef};

use crate::{
    git::{self, Change},
    process::{self, Limits},
    repo_path::RepoPath,
};

use super::sqlite_browser::{
    SQLITE_PAGE_SIZE, SqliteColumn, SqliteDatabase, SqliteObject, SqlitePage, SqlitePageCursor,
    SqlitePageKey,
};

const MAX_IMAGE_SOURCE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_DECODED_WIDTH: u32 = 16_384;
const MAX_DECODED_HEIGHT: u32 = 16_384;
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PREVIEW_WIDTH: u32 = 3_840;
const MAX_PREVIEW_HEIGHT: u32 = 2_160;
const MAX_VIDEO_FRAME_BYTES: usize = 32 * 1024 * 1024;
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";
const MAX_SQLITE_OBJECTS: usize = 1_000;
const MAX_SQLITE_COLUMNS: usize = 128;
const MAX_SQLITE_VALUE_CHARS: usize = 80;
const SQLITE_QUERY_TIMEOUT: Duration = Duration::from_millis(750);
const PULL_REQUEST_CACHE_SIZE: usize = 8;

pub(super) enum LoadedPreview {
    Text(String),
    Database {
        path: RepoPath,
        database: SqliteDatabase,
    },
    DatabasePage {
        path: RepoPath,
        key: SqlitePageKey,
        result: Result<SqlitePage, String>,
    },
    Image(Arc<DynamicImage>),
    Svg {
        source: String,
        preview: Result<Arc<DynamicImage>, String>,
    },
    Error(String),
}

pub(super) struct PreviewLoader {
    generation: u64,
    cancellation_generation: Arc<AtomicU64>,
    pending: Arc<Mutex<Option<Request>>>,
    wake: Option<SyncSender<()>>,
    receiver: Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewLoader {
    pub(super) fn new() -> Self {
        let pending = Arc::new(Mutex::new(None::<Request>));
        let worker_pending = Arc::clone(&pending);
        let cancellation_generation = Arc::new(AtomicU64::new(0));
        let worker_cancellation_generation = Arc::clone(&cancellation_generation);
        let (wake, request_rx) = mpsc::sync_channel::<()>(1);
        let (result_tx, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut pull_request_cache = VecDeque::<PullRequestCacheEntry>::new();
            while request_rx.recv().is_ok() {
                let Some(request) = worker_pending.lock().ok().and_then(|mut slot| slot.take())
                else {
                    continue;
                };
                let content = match &request.task {
                    Task::File(path) => load_file_preview_cancellable(&request.root, path, &|| {
                        worker_cancellation_generation.load(Ordering::Relaxed) != request.generation
                    }),
                    Task::Commit(oid) => git::commit_diff(&request.root, oid)
                        .map(LoadedPreview::Text)
                        .unwrap_or_else(|error| LoadedPreview::Error(error.to_string())),
                    Task::Diff(change) => git::diff(&request.root, change)
                        .map(LoadedPreview::Text)
                        .unwrap_or_else(|error| LoadedPreview::Error(error.to_string())),
                    Task::SectionDiff { changes, staged } => {
                        git::section_diff(&request.root, changes, *staged)
                            .map(LoadedPreview::Text)
                            .unwrap_or_else(|error| LoadedPreview::Error(error.to_string()))
                    }
                    Task::BranchDiff { target, current } => {
                        git::branch_diff(&request.root, target, current)
                            .map(LoadedPreview::Text)
                            .unwrap_or_else(|error| LoadedPreview::Error(error.to_string()))
                    }
                    Task::PullRequestDiff {
                        repository,
                        repository_url,
                        base,
                        head,
                    } => {
                        let cancelled = || {
                            worker_cancellation_generation.load(Ordering::Relaxed)
                                != request.generation
                        };
                        if let Some(diff) = cached_pull_request_diff(
                            &mut pull_request_cache,
                            &request.root,
                            base,
                            head,
                        ) {
                            LoadedPreview::Text(diff)
                        } else {
                            let loaded =
                                match git::pull_request_diff(&request.root, base, head, &cancelled)
                                {
                                    Ok(Some(diff)) => LoadedPreview::Text(diff),
                                    Ok(None) => crate::app::issues::pull_request_diff(
                                        &request.root,
                                        repository,
                                        repository_url,
                                        base,
                                        head,
                                        &cancelled,
                                    )
                                    .map(LoadedPreview::Text)
                                    .unwrap_or_else(LoadedPreview::Error),
                                    Err(error) => LoadedPreview::Error(error.to_string()),
                                };
                            if let LoadedPreview::Text(diff) = &loaded
                                && !cancelled()
                            {
                                pull_request_cache.push_back(PullRequestCacheEntry {
                                    root: request.root.clone(),
                                    base: base.clone(),
                                    head: head.clone(),
                                    diff: diff.clone(),
                                });
                                if pull_request_cache.len() > PULL_REQUEST_CACHE_SIZE {
                                    pull_request_cache.pop_front();
                                }
                            }
                            loaded
                        }
                    }
                    Task::SqlitePage { path, key } => LoadedPreview::DatabasePage {
                        path: path.clone(),
                        key: key.clone(),
                        result: load_sqlite_page(&request.root.join(path.as_path()), key),
                    },
                };
                if result_tx
                    .send(Completion {
                        generation: request.generation,
                        root: request.root,
                        content,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            generation: 0,
            cancellation_generation,
            pending,
            wake: Some(wake),
            receiver,
            worker: Some(worker),
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.cancellation_generation
            .store(self.generation, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn generation_for_test(&self) -> u64 {
        self.generation
    }

    pub(super) fn request_file(&mut self, root: &Path, path: RepoPath) {
        self.request(root, Task::File(path));
    }

    pub(super) fn request_commit(&mut self, root: &Path, oid: String) {
        self.request(root, Task::Commit(oid));
    }

    pub(super) fn request_diff(&mut self, root: &Path, change: Change) {
        self.request(root, Task::Diff(change));
    }

    pub(super) fn request_section_diff(&mut self, root: &Path, changes: Vec<Change>, staged: bool) {
        self.request(root, Task::SectionDiff { changes, staged });
    }

    pub(super) fn request_branch_diff(&mut self, root: &Path, target: String, current: String) {
        self.request(root, Task::BranchDiff { target, current });
    }

    pub(super) fn request_pull_request_diff(
        &mut self,
        root: &Path,
        repository: String,
        repository_url: String,
        base: String,
        head: String,
    ) {
        self.request(
            root,
            Task::PullRequestDiff {
                repository,
                repository_url,
                base,
                head,
            },
        );
    }

    pub(super) fn request_sqlite_page(&mut self, root: &Path, path: RepoPath, key: SqlitePageKey) {
        self.request(root, Task::SqlitePage { path, key });
    }

    pub(super) fn poll(&mut self, active_root: Option<&Path>) -> Option<LoadedPreview> {
        let mut content = None;
        while let Ok(result) = self.receiver.try_recv() {
            if result.generation == self.generation
                && active_root.is_some_and(|root| root == result.root)
            {
                content = Some(result.content);
            }
        }
        content
    }

    fn request(&mut self, root: &Path, task: Task) {
        self.invalidate();
        let request = Request {
            generation: self.generation,
            root: root.to_path_buf(),
            task,
        };
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(request);
            if let Some(wake) = &self.wake {
                let _ = wake.try_send(());
            }
        }
    }

    pub(super) fn shutdown(&mut self) {
        self.invalidate();
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for PreviewLoader {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Request {
    generation: u64,
    root: PathBuf,
    task: Task,
}

enum Task {
    File(RepoPath),
    Commit(String),
    Diff(Change),
    SectionDiff {
        changes: Vec<Change>,
        staged: bool,
    },
    BranchDiff {
        target: String,
        current: String,
    },
    PullRequestDiff {
        repository: String,
        repository_url: String,
        base: String,
        head: String,
    },
    SqlitePage {
        path: RepoPath,
        key: SqlitePageKey,
    },
}

struct PullRequestCacheEntry {
    root: PathBuf,
    base: String,
    head: String,
    diff: String,
}

fn cached_pull_request_diff(
    cache: &mut VecDeque<PullRequestCacheEntry>,
    root: &Path,
    base: &str,
    head: &str,
) -> Option<String> {
    let index = cache
        .iter()
        .position(|entry| entry.root == root && entry.base == base && entry.head == head)?;
    let entry = cache.remove(index)?;
    let diff = entry.diff.clone();
    cache.push_back(entry);
    Some(diff)
}

struct Completion {
    generation: u64,
    root: PathBuf,
    content: LoadedPreview,
}

#[cfg(test)]
fn load_file_preview(root: &Path, path: &RepoPath) -> LoadedPreview {
    load_file_preview_cancellable(root, path, &|| false)
}

fn load_file_preview_cancellable(
    root: &Path,
    path: &RepoPath,
    cancelled: &dyn Fn() -> bool,
) -> LoadedPreview {
    let full_path = root.join(path.as_path());
    match fs::symlink_metadata(&full_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return git::file_content(root, path)
                .map(LoadedPreview::Text)
                .unwrap_or_else(|error| LoadedPreview::Error(error.to_string()));
        }
        Err(error) => return LoadedPreview::Error(format!("Could not inspect file: {error}")),
    }
    if has_sqlite_header(&full_path) {
        return load_sqlite_database(&full_path)
            .map(|database| LoadedPreview::Database {
                path: path.clone(),
                database,
            })
            .unwrap_or_else(|error| {
                LoadedPreview::Error(format!("Could not read SQLite database: {error}"))
            });
    }
    if is_video(path.as_path()) {
        return load_video_frame(&full_path, cancelled)
            .map(|image| LoadedPreview::Image(Arc::new(image)))
            .unwrap_or_else(LoadedPreview::Error);
    }
    if is_svg(path.as_path()) {
        return match git::file_content(root, path) {
            Ok(source) => {
                let preview = render_svg(source.as_bytes()).map(Arc::new);
                LoadedPreview::Svg { source, preview }
            }
            Err(error) => LoadedPreview::Error(error.to_string()),
        };
    }
    if is_image(path.as_path()) {
        return load_image(&full_path)
            .map(|image| LoadedPreview::Image(Arc::new(image)))
            .unwrap_or_else(LoadedPreview::Error);
    }
    git::file_content(root, path)
        .map(LoadedPreview::Text)
        .unwrap_or_else(|error| LoadedPreview::Error(error.to_string()))
}

fn has_sqlite_header(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; SQLITE_HEADER.len()];
    file.read_exact(&mut header).is_ok() && &header == SQLITE_HEADER
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

fn load_sqlite_database(path: &Path) -> Result<SqliteDatabase, String> {
    let connection = open_sqlite(path)?;
    let mut statement = connection
        .prepare(
            "SELECT type, name FROM sqlite_schema \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name COLLATE NOCASE \
             LIMIT ?1",
        )
        .map_err(|error| error.to_string())?;
    let mut objects = statement
        .query_map(
            [i64::try_from(MAX_SQLITE_OBJECTS + 1).unwrap_or(i64::MAX)],
            |row| {
                Ok(SqliteObject {
                    kind: row.get(0)?,
                    name: row.get(1)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let objects_truncated = objects.len() > MAX_SQLITE_OBJECTS;
    objects.truncate(MAX_SQLITE_OBJECTS);
    let file_size = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let user_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    let first_page = objects.first().map(|object| {
        load_sqlite_page_with_connection(&connection, &object.name, &object.kind, 0, None)
    });
    drop(statement);
    Ok(SqliteDatabase {
        file_size,
        user_version,
        objects,
        objects_truncated,
        first_page,
    })
}

fn load_sqlite_page(path: &Path, key: &SqlitePageKey) -> Result<SqlitePage, String> {
    let connection = open_sqlite(path)?;
    let kind = connection
        .query_row(
            "SELECT type FROM sqlite_schema \
             WHERE type IN ('table', 'view') AND name = ?1",
            [&key.object],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => "table or view no longer exists".to_owned(),
            error => error.to_string(),
        })?;
    load_sqlite_page_with_connection(&connection, &key.object, &kind, key.offset, key.cursor)
}

fn open_sqlite(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA query_only = ON")
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + SQLITE_QUERY_TIMEOUT;
    connection.progress_handler(10_000, Some(move || Instant::now() >= deadline));
    Ok(connection)
}

fn sqlite_columns(
    connection: &Connection,
    table: &str,
) -> Result<(Vec<SqliteColumn>, bool), String> {
    let mut statement = connection
        .prepare(
            "SELECT name, type FROM pragma_table_xinfo(?1) \
             WHERE hidden != 1 ORDER BY cid LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let mut columns = statement
        .query_map(
            rusqlite::params![
                table,
                i64::try_from(MAX_SQLITE_COLUMNS + 1).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok(SqliteColumn {
                    name: row.get(0)?,
                    data_type: row.get(1)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let truncated = columns.len() > MAX_SQLITE_COLUMNS;
    columns.truncate(MAX_SQLITE_COLUMNS);
    Ok((columns, truncated))
}

fn sqlite_primary_key(connection: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM pragma_table_xinfo(?1) \
             WHERE hidden != 1 AND pk > 0 ORDER BY pk LIMIT 32",
        )
        .map_err(|error| error.to_string())?;
    statement
        .query_map([table], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn load_sqlite_page_with_connection(
    connection: &Connection,
    table: &str,
    kind: &str,
    offset: usize,
    cursor: Option<SqlitePageCursor>,
) -> Result<SqlitePage, String> {
    let (columns, columns_truncated) = sqlite_columns(connection, table)?;
    if columns.is_empty() {
        return Ok(SqlitePage {
            key: SqlitePageKey {
                object: table.to_owned(),
                offset,
                cursor,
            },
            columns: Vec::new(),
            columns_truncated,
            rows: Vec::new(),
            has_next: false,
            first_cursor: None,
            last_cursor: None,
        });
    }
    let projection = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let primary_key = sqlite_primary_key(connection, table)?;
    let integer_primary_key = (primary_key.len() == 1)
        .then(|| {
            let name = &primary_key[0];
            columns
                .iter()
                .position(|column| {
                    column.name == *name && column.data_type.eq_ignore_ascii_case("INTEGER")
                })
                .map(|index| (name, index))
        })
        .flatten();
    let order = if !primary_key.is_empty() {
        format!(
            " ORDER BY {}",
            primary_key
                .iter()
                .map(|column| quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else if kind == "table" {
        " ORDER BY rowid".to_owned()
    } else {
        format!(
            " ORDER BY {}",
            (1..=columns.len())
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let keyset = cursor.zip(integer_primary_key);
    let sql = if let Some((cursor, (column, _))) = keyset {
        let direction = if cursor.reverse { "<" } else { ">" };
        let order_direction = if cursor.reverse { " DESC" } else { "" };
        format!(
            "SELECT {projection} FROM {} WHERE {} {direction} ?1 ORDER BY {}{order_direction} LIMIT ?2",
            quote_identifier(table),
            quote_identifier(column),
            quote_identifier(column)
        )
    } else {
        format!(
            "SELECT {projection} FROM {}{order} LIMIT ?1 OFFSET ?2",
            quote_identifier(table)
        )
    };
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let page_limit = i64::try_from(SQLITE_PAGE_SIZE + 1).unwrap_or(i64::MAX);
    let mut query = if let Some((cursor, _)) = keyset {
        statement.query(rusqlite::params![cursor.value, page_limit])
    } else {
        statement.query(rusqlite::params![
            page_limit,
            i64::try_from(offset).unwrap_or(i64::MAX)
        ])
    }
    .map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    let mut cursors = Vec::new();
    while let Some(row) = query.next().map_err(|error| error.to_string())? {
        if let Some((_, index)) = integer_primary_key {
            cursors.push(
                row.get::<_, i64>(index)
                    .map_err(|error| error.to_string())?,
            );
        }
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(
                row.get_ref(index)
                    .map(format_sqlite_value)
                    .map_err(|error| error.to_string())?,
            );
        }
        rows.push(values);
    }
    let has_next = cursor.is_some_and(|cursor| cursor.reverse) || rows.len() > SQLITE_PAGE_SIZE;
    rows.truncate(SQLITE_PAGE_SIZE);
    cursors.truncate(SQLITE_PAGE_SIZE);
    if cursor.is_some_and(|cursor| cursor.reverse) {
        rows.reverse();
        cursors.reverse();
    }
    Ok(SqlitePage {
        key: SqlitePageKey {
            object: table.to_owned(),
            offset,
            cursor,
        },
        columns,
        columns_truncated,
        rows,
        has_next: if cursor.is_some_and(|cursor| cursor.reverse) {
            true
        } else {
            has_next
        },
        first_cursor: cursors.first().copied(),
        last_cursor: cursors.last().copied(),
    })
}

fn format_sqlite_value(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_owned(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => {
            let byte_limit = MAX_SQLITE_VALUE_CHARS.saturating_mul(4);
            let bytes_truncated = value.len() > byte_limit;
            let bounded = &value[..value.len().min(byte_limit)];
            let value = String::from_utf8_lossy(bounded);
            let mut escaped = escape_control_characters(&value);
            if bytes_truncated {
                escaped.push_str("...");
            }
            truncate_chars(&escaped, MAX_SQLITE_VALUE_CHARS)
        }
        ValueRef::Blob(value) => format!("<blob: {} bytes>", value.len()),
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn escape_control_characters(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            character if character.is_control() => escaped.extend(character.escape_default()),
            character => escaped.push(character),
        }
    }
    escaped
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(maximum).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "bmp"
                    | "gif"
                    | "ico"
                    | "jpg"
                    | "jpeg"
                    | "png"
                    | "pnm"
                    | "pbm"
                    | "pgm"
                    | "ppm"
                    | "qoi"
                    | "tga"
                    | "tif"
                    | "tiff"
                    | "webp"
            )
        })
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "3gp"
                    | "avi"
                    | "flv"
                    | "m4v"
                    | "mkv"
                    | "mov"
                    | "mp4"
                    | "mpeg"
                    | "mpg"
                    | "ogv"
                    | "webm"
                    | "wmv"
            )
        })
}

fn load_image(path: &Path) -> Result<DynamicImage, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Could not inspect image: {error}"))?;
    if metadata.len() > MAX_IMAGE_SOURCE_BYTES {
        return Err(format!(
            "Image is too large to preview (limit: {} MB)",
            MAX_IMAGE_SOURCE_BYTES / 1024 / 1024
        ));
    }
    let reader = ImageReader::open(path)
        .map_err(|error| format!("Could not open image: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("Could not identify image: {error}"))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("Could not read image dimensions: {error}"))?;
    if width > MAX_DECODED_WIDTH || height > MAX_DECODED_HEIGHT {
        return Err(format!(
            "Image dimensions are too large to preview ({width}x{height})"
        ));
    }
    let mut reader = ImageReader::open(path)
        .map_err(|error| format!("Could not open image: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("Could not identify image: {error}"))?;
    let mut limits = ImageLimits::default();
    limits.max_image_width = Some(MAX_DECODED_WIDTH);
    limits.max_image_height = Some(MAX_DECODED_HEIGHT);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("Could not decode image: {error}"))?;
    Ok(bound_preview_dimensions(image))
}

fn render_svg(source: &[u8]) -> Result<DynamicImage, String> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(source, &options)
        .map_err(|error| format!("Could not parse SVG: {error}"))?;
    let size = tree.size();
    let scale = (MAX_PREVIEW_WIDTH as f32 / size.width())
        .min(MAX_PREVIEW_HEIGHT as f32 / size.height())
        .min(1.0);
    let width = (size.width() * scale)
        .ceil()
        .clamp(1.0, MAX_PREVIEW_WIDTH as f32) as u32;
    let height = (size.height() * scale)
        .ceil()
        .clamp(1.0, MAX_PREVIEW_HEIGHT as f32) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "SVG dimensions are too large to preview".to_owned())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|error| format!("Could not encode SVG preview: {error}"))?;
    image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|error| format!("Could not decode SVG preview: {error}"))
}

fn load_video_frame(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<DynamicImage, String> {
    let output = process::run_cancellable(
        Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-vf",
                "thumbnail=30,scale=1920:1080:force_original_aspect_ratio=decrease",
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ]),
        Limits::new(MAX_VIDEO_FRAME_BYTES, 64 * 1024, Duration::from_secs(8)),
        cancelled,
    )
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "Video previews require ffmpeg on PATH".to_owned()
        } else {
            format!("Could not start ffmpeg: {error}")
        }
    })?;
    if output.timed_out {
        return Err("Video thumbnail generation timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("Video thumbnail exceeded the preview size limit".to_owned());
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.lines().find(|line| !line.trim().is_empty());
        return Err(detail.map_or_else(
            || "Could not generate a video thumbnail".to_owned(),
            |line| format!("Could not generate a video thumbnail: {line}"),
        ));
    }
    let mut reader = ImageReader::new(Cursor::new(output.stdout))
        .with_guessed_format()
        .map_err(|error| format!("Could not identify video thumbnail: {error}"))?;
    let mut limits = ImageLimits::default();
    limits.max_image_width = Some(MAX_PREVIEW_WIDTH);
    limits.max_image_height = Some(MAX_PREVIEW_HEIGHT);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    reader
        .decode()
        .map(bound_preview_dimensions)
        .map_err(|error| format!("Could not decode video thumbnail: {error}"))
}

fn bound_preview_dimensions(image: DynamicImage) -> DynamicImage {
    if image.width() <= MAX_PREVIEW_WIDTH && image.height() <= MAX_PREVIEW_HEIGHT {
        image
    } else {
        image.thumbnail(MAX_PREVIEW_WIDTH, MAX_PREVIEW_HEIGHT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_joins_the_preview_worker_once() {
        let mut loader = PreviewLoader::new();
        loader.shutdown();
        loader.shutdown();
        assert!(loader.worker.is_none());
        assert!(loader.wake.is_none());
    }

    #[test]
    fn recognizes_supported_static_media() {
        for name in [
            "photo.PNG",
            "scan.tiff",
            "animation.GIF",
            "clip.webm",
            "movie.MP4",
        ] {
            let path = Path::new(name);
            assert!(is_image(path) || is_video(path), "{name}");
        }
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_video(Path::new("archive.zip")));
    }

    #[test]
    fn rejects_corrupt_images_without_exposing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.png");
        fs::write(&path, b"not actually a png\0\xff").unwrap();
        let error = load_image(&path).unwrap_err();
        assert!(error.starts_with("Could not read image dimensions:"));
    }

    #[test]
    fn loads_sqlite_catalog_and_bounded_pages_by_file_signature() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("extensionless");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, payload BLOB); \
                 CREATE VIEW named_people AS SELECT name FROM people; \
                 INSERT INTO people (name, payload) VALUES \
                    ('Ada', X'0102'), ('Grace', NULL), ('Linus', X'03'), \
                    ('Margaret', NULL), ('Edsger', NULL), ('Barbara', NULL);",
            )
            .unwrap();
        drop(connection);

        let preview = load_file_preview(directory.path(), &RepoPath::from("extensionless"));
        let LoadedPreview::Database { path, database } = preview else {
            panic!("expected a database preview");
        };
        assert_eq!(path, RepoPath::from("extensionless"));
        assert_eq!(database.objects.len(), 2);
        assert_eq!(database.objects[0].name, "people");
        assert_eq!(database.objects[0].kind, "table");
        assert_eq!(database.objects[1].name, "named_people");
        assert_eq!(database.objects[1].kind, "view");
        let page = database.first_page.unwrap().unwrap();
        assert_eq!(page.columns[1].name, "name");
        assert_eq!(page.rows.len(), 6);
        assert_eq!(page.rows[0][1], "Ada");
        assert_eq!(page.rows[0][2], "<blob: 2 bytes>");
        assert!(!page.has_next);
    }

    #[test]
    fn keyset_pages_round_trip_without_offset_scans() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE records(id INTEGER PRIMARY KEY, value TEXT);\
                 WITH RECURSIVE source(value) AS (\
                     SELECT 1 UNION ALL SELECT value + 1 FROM source WHERE value < 250\
                 )\
                 INSERT INTO records(id, value) SELECT value, printf('row-%03d', value) FROM source;",
            )
            .unwrap();

        let first =
            load_sqlite_page_with_connection(&connection, "records", "table", 0, None).unwrap();
        assert_eq!(first.rows.first().unwrap()[0], "1");
        assert_eq!(first.rows.last().unwrap()[0], "100");
        assert!(first.has_next);

        let second = load_sqlite_page_with_connection(
            &connection,
            "records",
            "table",
            SQLITE_PAGE_SIZE,
            Some(SqlitePageCursor {
                value: first.last_cursor.unwrap(),
                reverse: false,
            }),
        )
        .unwrap();
        assert_eq!(second.rows.first().unwrap()[0], "101");
        assert_eq!(second.rows.last().unwrap()[0], "200");
        assert!(second.has_next);

        let previous = load_sqlite_page_with_connection(
            &connection,
            "records",
            "table",
            0,
            Some(SqlitePageCursor {
                value: second.first_cursor.unwrap(),
                reverse: true,
            }),
        )
        .unwrap();
        assert_eq!(previous.rows, first.rows);
        assert!(previous.has_next);
    }
}
