use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;

use crate::process::{self, Limits};

use super::{
    AgentActivityPreview, AgentRequestPartPreview, AgentRequestPreview, AgentUserMessage,
    unix_time_ms,
};

const QUERY_LIMITS: Limits = Limits::new(2 * 1024 * 1024, 64 * 1024, Duration::from_secs(5));

static DATABASE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static TRANSCRIPT_DATABASE: Mutex<Option<TranscriptDatabase>> = Mutex::new(None);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TranscriptFetch {
    Changed(Vec<AgentUserMessage>),
    Unchanged,
}

struct TranscriptDatabase {
    connection: Connection,
    session_versions: HashMap<String, i64>,
}

pub(super) fn fetch(session_id: &str, allow_unchanged: bool) -> Result<TranscriptFetch, String> {
    validate_session_id(session_id)?;
    let path = database_path()?;
    fetch_from_database(&TRANSCRIPT_DATABASE, &path, session_id, allow_unchanged)
}

pub(super) fn final_assistant_text(session_id: &str) -> Result<String, String> {
    validate_session_id(session_id)?;
    let path = database_path()?;
    let mut database = TRANSCRIPT_DATABASE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if database.is_none() {
        *database = Some(TranscriptDatabase::open(&path)?);
    }
    query_final_assistant_text(
        &database
            .as_ref()
            .expect("transcript database was opened")
            .connection,
        session_id,
    )
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .then_some(())
        .ok_or_else(|| "OpenCode session ID contains unsupported characters".to_owned())
}

fn query_final_assistant_text(connection: &Connection, session_id: &str) -> Result<String, String> {
    let response_id = connection
        .query_row(
            "SELECT response.id FROM message response \
             WHERE response.session_id = ?1 \
               AND json_extract(response.data, '$.role') = 'assistant' \
               AND json_extract(response.data, '$.parentID') = ( \
                   SELECT user.id FROM message user \
                   WHERE user.session_id = ?1 \
                     AND json_extract(user.data, '$.role') = 'user' \
                   ORDER BY user.time_created DESC, user.id DESC LIMIT 1 \
               ) \
               AND json_extract(response.data, '$.time.completed') IS NOT NULL \
             ORDER BY response.time_created DESC, response.id DESC LIMIT 1",
            [session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Could not query the final OpenCode response: {error}"))?
        .ok_or_else(|| "OpenCode's latest assistant response is not complete".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT json_extract(data, '$.text') FROM part \
             WHERE message_id = ?1 AND json_extract(data, '$.type') = 'text' \
             ORDER BY time_created, id",
        )
        .map_err(|error| format!("Could not prepare the final OpenCode response: {error}"))?;
    let text = statement
        .query_map([response_id], |row| row.get::<_, Option<String>>(0))
        .map_err(|error| format!("Could not query the final OpenCode response: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read the final OpenCode response: {error}"))?
        .into_iter()
        .flatten()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty())
        .then_some(text)
        .ok_or_else(|| "OpenCode's final assistant response has no text".to_owned())
}

fn fetch_from_database(
    cache: &Mutex<Option<TranscriptDatabase>>,
    path: &Path,
    session_id: &str,
    allow_unchanged: bool,
) -> Result<TranscriptFetch, String> {
    let mut database = cache.lock().unwrap_or_else(|error| error.into_inner());
    if database.is_none() {
        *database = Some(TranscriptDatabase::open(path)?);
    }
    let result = database
        .as_mut()
        .expect("transcript database was opened")
        .fetch(session_id, allow_unchanged);
    if result.is_err() {
        *database = None;
    }
    result
}

impl TranscriptDatabase {
    fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("Could not open the OpenCode database: {error}"))?;
        Ok(Self {
            connection,
            session_versions: HashMap::new(),
        })
    }

    fn fetch(
        &mut self,
        session_id: &str,
        allow_unchanged: bool,
    ) -> Result<TranscriptFetch, String> {
        let data_version = self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("Could not query OpenCode: {error}"))?;
        if allow_unchanged && self.session_versions.get(session_id) == Some(&data_version) {
            return Ok(TranscriptFetch::Unchanged);
        }

        let messages = query(&self.connection, session_id)?;
        self.session_versions
            .insert(session_id.to_owned(), data_version);
        Ok(TranscriptFetch::Changed(messages))
    }
}

fn query(connection: &Connection, session_id: &str) -> Result<Vec<AgentUserMessage>, String> {
    let mut statement = connection
        .prepare(
            "WITH responses AS (\
             SELECT m.*, row_number() OVER (\
                        PARTITION BY json_extract(m.data, '$.parentID') \
                        ORDER BY m.time_created, m.id) AS response_index \
             FROM message m \
             WHERE m.session_id = ?1 \
               AND json_extract(m.data, '$.role') = 'assistant'\
           ) \
         SELECT user.id AS message_id, p.id AS user_part_id, \
         CASE WHEN response.response_index IS NULL OR response.response_index = 1 \
              THEN json_extract(p.data, '$.text') \
              ELSE '' END AS text, \
                response.id AS response_id, \
                response.time_created AS response_started_at, \
                json_extract(response.data, '$.time.completed') AS response_completed_at, \
                 (SELECT json_group_array(json(part_json)) FROM ( \
                     SELECT json_object( \
                                'type', json_extract(response_part.data, '$.type'), \
                                'text', CASE \
                                    WHEN json_extract(response_part.data, '$.type') = 'text' \
                                      THEN json_extract(response_part.data, '$.text') \
                                     ELSE NULL END, \
                                 'name', json_extract(response_part.data, '$.tool'), \
                                  'title', json_extract(response_part.data, '$.state.title'), \
                                 'status', json_extract(response_part.data, '$.state.status'), \
                                 'started_at', json_extract(response_part.data, '$.time.start'), \
                                 'completed_at', json_extract(response_part.data, '$.time.end')) \
                                AS part_json \
                       FROM part response_part \
                      WHERE response_part.message_id = response.id \
                        AND json_extract(response_part.data, '$.type') \
                            IN ('reasoning', 'tool', 'text') \
                      ORDER BY response_part.time_created, response_part.id \
                 )) AS response_parts \
         FROM message user \
         JOIN part p ON p.message_id = user.id \
                     AND json_extract(p.data, '$.type') = 'text' \
         LEFT JOIN responses response \
                ON json_extract(response.data, '$.parentID') = user.id \
         WHERE user.session_id = ?1 \
           AND json_extract(user.data, '$.role') = 'user' \
         ORDER BY user.time_created, user.id, p.time_created, p.id, \
                  response.time_created, response.id",
        )
        .map_err(|error| format!("Could not prepare the OpenCode query: {error}"))?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok(serde_json::json!({
                "message_id": row.get::<_, String>(0)?,
                "user_part_id": row.get::<_, String>(1)?,
                "text": row.get::<_, String>(2)?,
                "response_id": row.get::<_, Option<String>>(3)?,
                "response_started_at": row.get::<_, Option<u64>>(4)?,
                "response_completed_at": row.get::<_, Option<u64>>(5)?,
                "response_parts": row.get::<_, Option<String>>(6)?,
            }))
        })
        .map_err(|error| format!("Could not query OpenCode: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read OpenCode query data: {error}"))?;
    parse_rows(&rows)
}

fn database_path() -> Result<PathBuf, String> {
    cached_database_path(&DATABASE_PATH, resolve_database_path)
}

fn cached_database_path(
    cache: &Mutex<Option<PathBuf>>,
    resolve: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    let mut path = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(path) = path.as_ref() {
        return Ok(path.clone());
    }
    let resolved = resolve()?;
    *path = Some(resolved.clone());
    Ok(resolved)
}

fn resolve_database_path() -> Result<PathBuf, String> {
    let output = process::run(
        Command::new("opencode").args(["db", "path", "--pure"]),
        QUERY_LIMITS,
    )
    .map_err(|error| format!("Could not locate the OpenCode database: {error}"))?;
    if output.timed_out {
        return Err("OpenCode database lookup timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("OpenCode returned an invalid database path".to_owned());
    }
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(error.trim().to_owned());
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    (!path.as_os_str().is_empty())
        .then_some(path)
        .ok_or_else(|| "OpenCode returned an empty database path".to_owned())
}

pub(super) fn resolve_session_id(directory: &Path, title: &str) -> Result<String, String> {
    let directory = sql_string(&directory.to_string_lossy());
    let title = sql_string(title);
    let query = format!(
        "SELECT id FROM session \
         WHERE directory = {directory} AND title = {title} \
           AND parent_id IS NULL AND time_archived IS NULL"
    );
    let output = process::run(
        Command::new("opencode").args(["db", &query, "--format", "json", "--pure"]),
        QUERY_LIMITS,
    )
    .map_err(|error| format!("Could not query OpenCode sessions: {error}"))?;
    if output.timed_out {
        return Err("OpenCode session lookup timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("OpenCode returned too many matching sessions".to_owned());
    }
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("OpenCode session lookup failed: {}", error.trim()));
    }
    parse_session_id(&output.stdout)
}

pub(super) fn resolve_scheduled_session_id(
    directory: &Path,
    prompt: &str,
    run_created_at_ms: i64,
) -> Result<String, String> {
    resolve_scheduled_session_id_cancellable(directory, prompt, run_created_at_ms, &|| false)
}

pub(super) fn resolve_scheduled_session_id_cancellable(
    directory: &Path,
    prompt: &str,
    run_created_at_ms: i64,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    resolve_scheduled_session_id_with_program(
        Path::new("opencode"),
        directory,
        prompt,
        run_created_at_ms,
        cancelled,
    )
}

pub(super) fn resolve_scheduled_session_id_with_program(
    program: &Path,
    directory: &Path,
    prompt: &str,
    run_created_at_ms: i64,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, String> {
    let directory = sql_string(&directory.to_string_lossy());
    let prompt = sql_string(prompt);
    let query = format!(
        "SELECT DISTINCT session.id FROM session \
         JOIN message ON message.session_id = session.id \
         JOIN part ON part.message_id = message.id \
         WHERE session.directory = {directory} \
            AND session.parent_id IS NULL \
            AND json_extract(message.data, '$.role') = 'user' \
            AND json_extract(part.data, '$.type') = 'text' \
            AND trim(json_extract(part.data, '$.text')) = trim({prompt}) \
          ORDER BY abs(session.time_created - {run_created_at_ms}) \
          LIMIT 1"
    );
    let output = process::run_cancellable(
        Command::new(program).args(["db", &query, "--format", "json", "--pure"]),
        QUERY_LIMITS,
        cancelled,
    )
    .map_err(|error| format!("Could not query OpenCode sessions: {error}"))?;
    if output.timed_out {
        return Err("OpenCode session lookup timed out".to_owned());
    }
    if !output.status.success() {
        return Err(format!(
            "OpenCode session lookup failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_session_id(&output.stdout)
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn parse_session_id(output: &[u8]) -> Result<String, String> {
    let rows: Value = serde_json::from_slice(output)
        .map_err(|error| format!("OpenCode returned invalid session data: {error}"))?;
    let mut ids = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("id").and_then(Value::as_str));
    let Some(id) = ids.next() else {
        return Err("OpenCode session could not be identified".to_owned());
    };
    if ids.next().is_some() {
        return Err("Multiple OpenCode sessions match this agent".to_owned());
    }
    Ok(id.to_owned())
}

#[cfg(test)]
fn parse(output: &[u8]) -> Result<Vec<AgentUserMessage>, String> {
    let rows: Value = serde_json::from_slice(output)
        .map_err(|error| format!("OpenCode returned invalid query data: {error}"))?;
    parse_rows(rows.as_array().map(Vec::as_slice).unwrap_or_default())
}

fn parse_rows(rows: &[Value]) -> Result<Vec<AgentUserMessage>, String> {
    let mut messages: Vec<(String, Vec<String>, Vec<String>, AgentUserMessage)> = Vec::new();
    let now_ms = unix_time_ms();
    for row in rows {
        let Some(id) = row.get("message_id").and_then(Value::as_str) else {
            continue;
        };
        let text = row
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let Some(user_part_id) = row.get("user_part_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some((_, user_parts, response_ids, message)) = messages
            .last_mut()
            .filter(|(message_id, _, _, _)| message_id == id)
        {
            if !text.is_empty() && !user_parts.iter().any(|part_id| part_id == user_part_id) {
                message.text.push_str("\n\n");
                message.text.push_str(text);
                user_parts.push(user_part_id.to_owned());
            }
            if let Some(response_id) = row.get("response_id").and_then(Value::as_str)
                && !response_ids.iter().any(|id| id == response_id)
            {
                message.requests.push(request(row, now_ms));
                response_ids.push(response_id.to_owned());
            }
        } else if !text.is_empty() {
            let mut response_ids = Vec::new();
            let mut requests = Vec::new();
            if let Some(response_id) = row.get("response_id").and_then(Value::as_str) {
                response_ids.push(response_id.to_owned());
                requests.push(request(row, now_ms));
            }
            messages.push((
                id.to_owned(),
                vec![user_part_id.to_owned()],
                response_ids,
                AgentUserMessage {
                    text: text.to_owned(),
                    requests,
                },
            ));
        }
    }
    let mut grouped = Vec::<AgentUserMessage>::new();
    for message in messages.into_iter().map(|(_, _, _, message)| message) {
        if let Some(pending) = grouped
            .last_mut()
            .filter(|message| waiting_for_agent_output(message))
        {
            pending.text.push_str("\n\n");
            pending.text.push_str(&message.text);
            pending.requests = message.requests;
        } else {
            grouped.push(message);
        }
    }
    (!grouped.is_empty())
        .then_some(grouped)
        .ok_or_else(|| "OpenCode session has no user message".to_owned())
}

fn waiting_for_agent_output(message: &AgentUserMessage) -> bool {
    message
        .requests
        .iter()
        .all(|request| request.parts.is_empty())
}

fn request(row: &Value, now_ms: u64) -> AgentRequestPreview {
    let parts = row
        .get("response_parts")
        .and_then(Value::as_str)
        .and_then(|parts| serde_json::from_str::<Value>(parts).ok());
    let parts = parts.as_ref().and_then(Value::as_array);
    let reasoning_duration_ms = parts.and_then(|parts| {
        let mut found = false;
        let duration = parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("reasoning"))
            .filter_map(|part| {
                let started_at = part.get("started_at")?.as_u64()?;
                found = true;
                let completed_at = part
                    .get("completed_at")
                    .and_then(Value::as_u64)
                    .unwrap_or(now_ms);
                Some(completed_at.saturating_sub(started_at))
            })
            .fold(0_u64, u64::saturating_add);
        found.then_some(duration)
    });
    let mut reasoning_seen = false;
    let request_parts = parts
        .into_iter()
        .flatten()
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("text") => part
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| AgentRequestPartPreview::Text(text.to_owned())),
            Some("reasoning") if !reasoning_seen => {
                reasoning_seen = true;
                Some(AgentRequestPartPreview::Activity(
                    AgentActivityPreview::Reasoning,
                ))
            }
            Some("tool") => {
                let name = part.get("name")?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                let title = part
                    .get("title")
                    .and_then(Value::as_str)
                    .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|title| !title.is_empty());
                Some(AgentRequestPartPreview::Activity(
                    AgentActivityPreview::Tool {
                        name: name.to_owned(),
                        title,
                        running: part.get("status").and_then(Value::as_str) == Some("running"),
                    },
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let reasoning_active = parts
        .into_iter()
        .flatten()
        .last()
        .and_then(|part| part.get("type"))
        .and_then(Value::as_str)
        == Some("reasoning");
    let tool_call_count = request_parts
        .iter()
        .filter(|part| {
            matches!(
                part,
                AgentRequestPartPreview::Activity(AgentActivityPreview::Tool { .. })
            )
        })
        .count()
        .try_into()
        .unwrap_or(u64::MAX);
    let duration_ms = row
        .get("response_started_at")
        .and_then(Value::as_u64)
        .map(|started_at| {
            row.get("response_completed_at")
                .and_then(Value::as_u64)
                .unwrap_or(now_ms)
                .saturating_sub(started_at)
        });
    AgentRequestPreview {
        parts: request_parts,
        tool_call_count,
        reasoning_active,
        duration_ms,
        reasoning_duration_ms,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc, Barrier, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use rusqlite::{Connection, params};
    use tempfile::tempdir;

    use super::{
        AgentActivityPreview, AgentRequestPartPreview, TranscriptDatabase, TranscriptFetch,
        cached_database_path, fetch_from_database, parse, parse_session_id,
        query_final_assistant_text, sql_string,
    };

    fn create_transcript_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE message (\
                     id TEXT PRIMARY KEY, \
                     session_id TEXT NOT NULL, \
                     time_created INTEGER NOT NULL, \
                     data TEXT NOT NULL\
                 ); \
                 CREATE TABLE part (\
                     id TEXT PRIMARY KEY, \
                     message_id TEXT NOT NULL, \
                     session_id TEXT NOT NULL, \
                     time_created INTEGER NOT NULL, \
                     data TEXT NOT NULL\
                 );",
            )
            .unwrap();
    }

    fn insert_user_message(connection: &Connection, session_id: &str, text: &str) {
        let message_id = format!("{session_id}-message");
        connection
            .execute(
                "INSERT INTO message (id, session_id, time_created, data) \
                 VALUES (?1, ?2, 1, '{\"role\":\"user\"}')",
                params![message_id, session_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part (id, message_id, session_id, time_created, data) \
                 VALUES (?1, ?2, ?3, 1, json_object('type', 'text', 'text', ?4))",
                params![format!("{session_id}-part"), message_id, session_id, text],
            )
            .unwrap();
    }

    #[test]
    fn extracts_text_from_the_latest_user_message() {
        let earlier_parts = serde_json::json!([
            {"type":"text","text":"Earlier response","name":null,"title":null,"status":null},
            {"type":"tool","text":null,"name":"read","title":"src/main.rs","status":"completed"}
        ])
        .to_string();
        let latest_parts = serde_json::json!([
            {"type":"text","text":"Latest response","name":null,"title":null,"status":null},
            {"type":"tool","text":null,"name":"apply_patch","title":"Updated files\nwith details","status":"running"},
            {"type":"reasoning","text":null,"name":null,"title":null,"status":null,"started_at":1000,"completed_at":2500},
            {"type":"reasoning","text":null,"name":null,"title":null,"status":null,"started_at":3000,"completed_at":3750}
        ])
        .to_string();
        let rows = serde_json::json!([
            {
                "message_id": "one",
                "user_part_id": "one-user",
                "text": "Earlier request",
                "response_id": "one-response-a",
                "response_parts": earlier_parts
            },
            {
                "message_id": "two",
                "user_part_id": "two-user-a",
                "text": "Latest request",
                "response_id": "two-response-a",
                "response_parts": serde_json::json!([
                    {"type":"tool","text":null,"name":"read","title":"context","status":"completed"}
                ]).to_string()
            },
            {
                "message_id": "two",
                "user_part_id": "two-user-a",
                "text": "Latest request",
                "response_id": "two-response-b",
                "response_started_at": 500,
                "response_completed_at": 4500,
                "response_parts": latest_parts
            },
            {
                "message_id": "two",
                "user_part_id": "two-user-b",
                "text": "with context",
                "response_id": "two-response-a",
                "response_parts": "[]"
            },
            {
                "message_id": "two",
                "user_part_id": "two-user-b",
                "text": "with context",
                "response_id": "two-response-b",
                "response_parts": "[]"
            }
        ]);

        let messages = parse(&serde_json::to_vec(&rows).unwrap()).unwrap();
        assert_eq!(messages[0].text, "Earlier request");
        assert_eq!(messages[0].requests.len(), 1);
        assert_eq!(messages[0].requests[0].tool_call_count, 1);
        assert_eq!(
            messages[0].requests[0].parts,
            vec![
                AgentRequestPartPreview::Text("Earlier response".to_owned()),
                AgentRequestPartPreview::Activity(AgentActivityPreview::Tool {
                    name: "read".to_owned(),
                    title: Some("src/main.rs".to_owned()),
                    running: false,
                }),
            ]
        );
        assert!(!messages[0].requests[0].reasoning_active);
        assert_eq!(messages[1].text, "Latest request\n\nwith context");
        assert_eq!(messages[1].requests.len(), 2);
        assert_eq!(messages[1].requests[1].tool_call_count, 1);
        assert_eq!(messages[1].requests[1].duration_ms, Some(4000));
        assert_eq!(messages[1].requests[1].reasoning_duration_ms, Some(2250));
        assert_eq!(
            messages[1].requests[1].parts,
            vec![
                AgentRequestPartPreview::Text("Latest response".to_owned()),
                AgentRequestPartPreview::Activity(AgentActivityPreview::Tool {
                    name: "apply_patch".to_owned(),
                    title: Some("Updated files with details".to_owned()),
                    running: true,
                }),
                AgentRequestPartPreview::Activity(AgentActivityPreview::Reasoning),
            ]
        );
        assert!(messages[1].requests[1].reasoning_active);
    }

    #[test]
    fn groups_queued_user_messages_with_the_final_response() {
        let rows = serde_json::json!([
            {
                "message_id": "completed",
                "user_part_id": "completed-user",
                "text": "Completed request",
                "response_id": "completed-response",
                "response_parts": serde_json::json!([
                    {"type":"text","text":"Completed response"}
                ]).to_string()
            },
            {
                "message_id": "queued-one",
                "user_part_id": "queued-one-user",
                "text": "First queued prompt",
                "response_id": "queued-one-response",
                "response_parts": "[]"
            },
            {
                "message_id": "queued-two",
                "user_part_id": "queued-two-user",
                "text": "Second queued prompt",
                "response_id": "queued-two-response",
                "response_parts": "[]"
            },
            {
                "message_id": "queued-three",
                "user_part_id": "queued-three-user",
                "text": "Third queued prompt",
                "response_id": "queued-response",
                "response_parts": serde_json::json!([
                    {"type":"text","text":"Combined response"}
                ]).to_string()
            }
        ]);

        let messages = parse(&serde_json::to_vec(&rows).unwrap()).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "Completed request");
        assert_eq!(messages[0].requests.len(), 1);
        assert_eq!(
            messages[1].text,
            "First queued prompt\n\nSecond queued prompt\n\nThird queued prompt"
        );
        assert_eq!(messages[1].requests.len(), 1);
        assert_eq!(
            messages[1].requests[0].parts,
            vec![AgentRequestPartPreview::Text(
                "Combined response".to_owned()
            )]
        );
    }

    #[test]
    fn final_text_requires_a_completed_response_to_the_latest_user_message() {
        let connection = Connection::open_in_memory().unwrap();
        create_transcript_schema(&connection);
        insert_user_message(&connection, "alpha", "First request");
        connection
            .execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES ('response-one', 'alpha', 2, '{\"role\":\"assistant\",\"parentID\":\"alpha-message\",\"time\":{\"completed\":2}}')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part (id, message_id, session_id, time_created, data) VALUES ('response-one-text', 'response-one', 'alpha', 2, '{\"type\":\"text\",\"text\":\"First result\"}')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES ('latest-user', 'alpha', 3, '{\"role\":\"user\"}')",
                [],
            )
            .unwrap();
        assert!(query_final_assistant_text(&connection, "alpha").is_err());

        connection
            .execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES ('response-two', 'alpha', 4, '{\"role\":\"assistant\",\"parentID\":\"latest-user\",\"time\":{\"completed\":4}}')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO part (id, message_id, session_id, time_created, data) VALUES ('response-two-a', 'response-two', 'alpha', 4, '{\"type\":\"text\",\"text\":\"Latest\"}'), ('response-two-b', 'response-two', 'alpha', 5, '{\"type\":\"text\",\"text\":\"result\"}')",
                [],
            )
            .unwrap();
        assert_eq!(
            query_final_assistant_text(&connection, "alpha").unwrap(),
            "Latest\n\nresult"
        );
    }

    #[test]
    fn groups_queued_user_messages_before_output_starts() {
        let rows = serde_json::json!([
            {
                "message_id": "queued-one",
                "user_part_id": "queued-one-user",
                "text": "First queued prompt"
            },
            {
                "message_id": "queued-two",
                "user_part_id": "queued-two-user",
                "text": "Second queued prompt"
            }
        ]);

        let messages = parse(&serde_json::to_vec(&rows).unwrap()).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].text,
            "First queued prompt\n\nSecond queued prompt"
        );
        assert!(messages[0].requests.is_empty());
    }

    #[test]
    fn resolves_only_an_unambiguous_session() {
        assert_eq!(sql_string("it's here"), "'it''s here'");
        assert_eq!(
            parse_session_id(br#"[{"id":"ses_exact"}]"#).unwrap(),
            "ses_exact"
        );
        assert!(parse_session_id(b"[]").is_err());
        assert!(parse_session_id(br#"[{"id":"one"},{"id":"two"}]"#).is_err());
    }

    #[test]
    fn database_path_cache_retries_failures_and_serializes_initial_success() {
        let cache = Arc::new(Mutex::new(None));
        assert_eq!(
            cached_database_path(&cache, || Err("not ready".to_owned())),
            Err("not ready".to_owned())
        );

        let attempts = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let attempts = Arc::clone(&attempts);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    cached_database_path(&cache, || {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        thread::sleep(Duration::from_millis(10));
                        Ok(PathBuf::from("/tmp/opencode.db"))
                    })
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            assert_eq!(thread.join().unwrap(), PathBuf::from("/tmp/opencode.db"));
        }
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn data_version_is_tracked_per_session_and_can_be_bypassed() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        let writer = Connection::open(&path).unwrap();
        create_transcript_schema(&writer);
        insert_user_message(&writer, "alpha", "Alpha request");
        insert_user_message(&writer, "beta", "Beta request");
        let mut database = TranscriptDatabase::open(&path).unwrap();

        let TranscriptFetch::Changed(alpha) = database.fetch("alpha", true).unwrap() else {
            panic!("first session fetch must query");
        };
        assert_eq!(
            database.fetch("alpha", true).unwrap(),
            TranscriptFetch::Unchanged
        );
        assert!(matches!(
            database.fetch("alpha", false).unwrap(),
            TranscriptFetch::Changed(_)
        ));
        assert!(matches!(
            database.fetch("beta", true).unwrap(),
            TranscriptFetch::Changed(_)
        ));

        insert_user_message(&writer, "unrelated", "Unrelated request");
        assert_eq!(
            database.fetch("alpha", true).unwrap(),
            TranscriptFetch::Changed(alpha)
        );
        assert_eq!(
            database.fetch("alpha", true).unwrap(),
            TranscriptFetch::Unchanged
        );
    }

    #[test]
    fn failed_query_discards_the_cached_connection_for_recovery() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("opencode.db");
        let writer = Connection::open(&path).unwrap();
        create_transcript_schema(&writer);
        insert_user_message(&writer, "alpha", "Before failure");
        let cache = Mutex::new(None);

        assert!(matches!(
            fetch_from_database(&cache, &path, "alpha", true).unwrap(),
            TranscriptFetch::Changed(_)
        ));
        writer
            .execute_batch("DROP TABLE part; DROP TABLE message;")
            .unwrap();
        assert!(fetch_from_database(&cache, &path, "alpha", true).is_err());
        assert!(cache.lock().unwrap().is_none());

        create_transcript_schema(&writer);
        insert_user_message(&writer, "alpha", "After failure");
        let TranscriptFetch::Changed(messages) =
            fetch_from_database(&cache, &path, "alpha", true).unwrap()
        else {
            panic!("reopened connection must query");
        };
        assert_eq!(messages[0].text, "After failure");
    }
}
