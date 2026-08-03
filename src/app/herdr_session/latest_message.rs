use std::{path::Path, process::Command, time::Duration};

use serde_json::Value;

use crate::process::{self, Limits};

use super::{AgentActivityPreview, AgentRequestPartPreview, AgentRequestPreview, AgentUserMessage};

const QUERY_LIMITS: Limits = Limits::new(2 * 1024 * 1024, 64 * 1024, Duration::from_secs(5));

pub(super) fn fetch(session_id: &str) -> Result<Vec<AgentUserMessage>, String> {
    if !session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("OpenCode session ID contains unsupported characters".to_owned());
    }
    let query = format!(
        "WITH recent AS (\
             SELECT m.id, m.time_created FROM message m \
             WHERE m.session_id = '{session_id}' \
                AND json_extract(m.data, '$.role') = 'user'\
         ) \
         SELECT recent.id AS message_id, p.id AS user_part_id, \
                json_extract(p.data, '$.text') AS text, response.id AS response_id, \
                (SELECT json_group_array(json(part_json)) FROM ( \
                     SELECT json_object( \
                                'type', json_extract(response_part.data, '$.type'), \
                                'text', CASE \
                                    WHEN json_extract(response_part.data, '$.type') = 'text' \
                                    THEN json_extract(response_part.data, '$.text') \
                                    ELSE NULL END, \
                                'name', json_extract(response_part.data, '$.tool'), \
                                'title', json_extract(response_part.data, '$.state.title'), \
                                'status', json_extract(response_part.data, '$.state.status')) \
                                AS part_json \
                       FROM part response_part \
                      WHERE response_part.message_id = response.id \
                        AND json_extract(response_part.data, '$.type') \
                            IN ('reasoning', 'tool', 'text') \
                      ORDER BY response_part.time_created, response_part.id \
                 )) AS response_parts \
         FROM recent \
         JOIN part p ON p.message_id = recent.id \
                    AND json_extract(p.data, '$.type') = 'text' \
         LEFT JOIN message response \
                ON response.session_id = '{session_id}' \
               AND json_extract(response.data, '$.role') = 'assistant' \
               AND json_extract(response.data, '$.parentID') = recent.id \
         ORDER BY recent.time_created, recent.id, p.time_created, p.id, \
                  response.time_created, response.id"
    );
    let output = process::run(
        Command::new("opencode").args(["db", &query, "--format", "json", "--pure"]),
        QUERY_LIMITS,
    )
    .map_err(|error| format!("Could not query OpenCode: {error}"))?;
    if output.timed_out {
        return Err("OpenCode query timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("OpenCode message was too large".to_owned());
    }
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(error.trim().to_owned());
    }
    parse(&output.stdout)
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

fn parse(output: &[u8]) -> Result<Vec<AgentUserMessage>, String> {
    let rows: Value = serde_json::from_slice(output)
        .map_err(|error| format!("OpenCode returned invalid query data: {error}"))?;
    let mut messages: Vec<(String, Vec<String>, Vec<String>, AgentUserMessage)> = Vec::new();
    for row in rows.as_array().into_iter().flatten() {
        let Some(id) = row.get("message_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(text) = row.get("text").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let Some(user_part_id) = row.get("user_part_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some((_, user_parts, response_ids, message)) = messages
            .last_mut()
            .filter(|(message_id, _, _, _)| message_id == id)
        {
            if !user_parts.iter().any(|part_id| part_id == user_part_id) {
                message.text.push_str("\n\n");
                message.text.push_str(text);
                user_parts.push(user_part_id.to_owned());
            }
            if let Some(response_id) = row.get("response_id").and_then(Value::as_str)
                && !response_ids.iter().any(|id| id == response_id)
            {
                message.requests.push(request(row));
                response_ids.push(response_id.to_owned());
            }
        } else {
            let mut response_ids = Vec::new();
            let mut requests = Vec::new();
            if let Some(response_id) = row.get("response_id").and_then(Value::as_str) {
                response_ids.push(response_id.to_owned());
                requests.push(request(row));
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
    let messages = messages
        .into_iter()
        .map(|(_, _, _, message)| message)
        .collect::<Vec<_>>();
    (!messages.is_empty())
        .then_some(messages)
        .ok_or_else(|| "OpenCode session has no user message".to_owned())
}

fn request(row: &Value) -> AgentRequestPreview {
    let parts = row
        .get("response_parts")
        .and_then(Value::as_str)
        .and_then(|parts| serde_json::from_str::<Value>(parts).ok());
    let parts = parts.as_ref().and_then(Value::as_array);
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
            Some("reasoning") => Some(AgentRequestPartPreview::Activity(
                AgentActivityPreview::Reasoning,
            )),
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
    AgentRequestPreview {
        parts: request_parts,
        tool_call_count,
        reasoning_active,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentActivityPreview, AgentRequestPartPreview, parse, parse_session_id, sql_string,
    };

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
            {"type":"reasoning","text":null,"name":null,"title":null,"status":null}
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
    fn resolves_only_an_unambiguous_session() {
        assert_eq!(sql_string("it's here"), "'it''s here'");
        assert_eq!(
            parse_session_id(br#"[{"id":"ses_exact"}]"#).unwrap(),
            "ses_exact"
        );
        assert!(parse_session_id(b"[]").is_err());
        assert!(parse_session_id(br#"[{"id":"one"},{"id":"two"}]"#).is_err());
    }
}
