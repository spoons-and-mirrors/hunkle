use std::{path::Path, process::Command, time::Duration};

use serde_json::Value;

use crate::process::{self, Limits};

use super::{AgentActivityPreview, AgentUserMessage};

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
         SELECT recent.id AS message_id, json_extract(p.data, '$.text') AS text, \
                 COUNT(DISTINCT response.id) AS request_count, \
                 COUNT(DISTINCT tool.id) AS tool_call_count, \
                  (SELECT json_extract(agent_text.data, '$.text') \
                    FROM message agent_response \
                    JOIN part agent_text ON agent_text.message_id = agent_response.id \
                                        AND json_extract(agent_text.data, '$.type') = 'text' \
                   WHERE agent_response.session_id = '{session_id}' \
                     AND json_extract(agent_response.data, '$.role') = 'assistant' \
                     AND json_extract(agent_response.data, '$.parentID') = recent.id \
                     AND trim(COALESCE(json_extract(agent_text.data, '$.text'), '')) <> '' \
                    ORDER BY agent_text.time_created DESC, agent_text.id DESC LIMIT 1) \
                     AS latest_agent_text, \
                 (SELECT json_group_array(json(activity_json)) FROM ( \
                      SELECT json_object( \
                                 'type', json_extract(activity.data, '$.type'), \
                                 'name', json_extract(activity.data, '$.tool'), \
                                 'title', json_extract(activity.data, '$.state.title'), \
                                 'status', json_extract(activity.data, '$.state.status')) \
                                 AS activity_json \
                        FROM message activity_response \
                        JOIN part activity ON activity.message_id = activity_response.id \
                                          AND json_extract(activity.data, '$.type') \
                                              IN ('reasoning', 'tool') \
                       WHERE activity_response.session_id = '{session_id}' \
                         AND json_extract(activity_response.data, '$.role') = 'assistant' \
                         AND json_extract(activity_response.data, '$.parentID') = recent.id \
                       ORDER BY activity.time_created DESC, activity.id DESC LIMIT 2 \
                  )) AS latest_activities, \
                 (SELECT json_extract(activity.data, '$.type') \
                    FROM message activity_response \
                    JOIN part activity ON activity.message_id = activity_response.id \
                                      AND json_extract(activity.data, '$.type') \
                                          IN ('reasoning', 'tool', 'text') \
                   WHERE activity_response.session_id = '{session_id}' \
                     AND json_extract(activity_response.data, '$.role') = 'assistant' \
                     AND json_extract(activity_response.data, '$.parentID') = recent.id \
                   ORDER BY activity.time_created DESC, activity.id DESC LIMIT 1) \
                     AS latest_activity_type \
         FROM recent \
         JOIN part p ON p.message_id = recent.id \
                    AND json_extract(p.data, '$.type') = 'text' \
         LEFT JOIN message response \
                ON response.session_id = '{session_id}' \
               AND json_extract(response.data, '$.role') = 'assistant' \
               AND json_extract(response.data, '$.parentID') = recent.id \
         LEFT JOIN part tool ON tool.message_id = response.id \
                            AND json_extract(tool.data, '$.type') = 'tool' \
         GROUP BY recent.id, recent.time_created, p.id, p.time_created \
         ORDER BY recent.time_created, recent.id, p.time_created, p.id"
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
    let mut messages: Vec<(String, AgentUserMessage)> = Vec::new();
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
        if let Some((_, message)) = messages
            .last_mut()
            .filter(|(message_id, _)| message_id == id)
        {
            message.text.push_str("\n\n");
            message.text.push_str(text);
            if message.latest_agent_text.is_none() {
                message.latest_agent_text = latest_agent_text(row);
            }
            if message.activities.is_empty() {
                message.activities = activities(row);
            }
            message.reasoning_active |= reasoning_active(row);
        } else {
            messages.push((
                id.to_owned(),
                AgentUserMessage {
                    text: text.to_owned(),
                    latest_agent_text: latest_agent_text(row),
                    activities: activities(row),
                    reasoning_active: reasoning_active(row),
                    request_count: row
                        .get("request_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    tool_call_count: row
                        .get("tool_call_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                },
            ));
        }
    }
    let messages = messages
        .into_iter()
        .map(|(_, message)| message)
        .collect::<Vec<_>>();
    (!messages.is_empty())
        .then_some(messages)
        .ok_or_else(|| "OpenCode session has no user message".to_owned())
}

fn latest_agent_text(row: &Value) -> Option<String> {
    row.get("latest_agent_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn activities(row: &Value) -> Vec<AgentActivityPreview> {
    let Some(activities) = row
        .get("latest_activities")
        .and_then(Value::as_str)
        .and_then(|activities| serde_json::from_str::<Value>(activities).ok())
    else {
        return Vec::new();
    };
    let mut activities = activities
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(
            |activity| match activity.get("type").and_then(Value::as_str) {
                Some("reasoning") => Some(AgentActivityPreview::Reasoning),
                Some("tool") => {
                    let name = activity.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    let title = activity
                        .get("title")
                        .and_then(Value::as_str)
                        .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
                        .filter(|title| !title.is_empty());
                    Some(AgentActivityPreview::Tool {
                        name: name.to_owned(),
                        title,
                        running: activity.get("status").and_then(Value::as_str) == Some("running"),
                    })
                }
                _ => None,
            },
        )
        .collect::<Vec<_>>();
    activities.reverse();
    activities
}

fn reasoning_active(row: &Value) -> bool {
    row.get("latest_activity_type").and_then(Value::as_str) == Some("reasoning")
}

#[cfg(test)]
mod tests {
    use super::{AgentActivityPreview, parse, parse_session_id, sql_string};

    #[test]
    fn extracts_text_from_the_latest_user_message() {
        let rows = serde_json::json!([
            {
                "message_id": "one",
                "text": "Earlier request",
                "latest_agent_text": "Earlier response",
                "latest_activities": serde_json::json!([
                    {"type":"tool","name":"read","title":"src/main.rs","status":"completed"}
                ]).to_string(),
                "latest_activity_type": "text",
                "request_count": 3,
                "tool_call_count": 8
            },
            {
                "message_id": "two",
                "text": "Latest request",
                "latest_agent_text": "Latest response",
                "latest_activities": serde_json::json!([
                    {"type":"reasoning","name":null,"title":null,"status":null},
                    {"type":"tool","name":"apply_patch","title":"Updated files\nwith details","status":"running"}
                ]).to_string(),
                "latest_activity_type": "reasoning",
                "request_count": 2,
                "tool_call_count": 5
            },
            {
                "message_id": "two",
                "text": "with context",
                "latest_agent_text": "Latest response",
                "latest_activities": serde_json::json!([
                    {"type":"reasoning","name":null,"title":null,"status":null},
                    {"type":"tool","name":"apply_patch","title":"Updated files\nwith details","status":"running"}
                ]).to_string(),
                "latest_activity_type": "reasoning",
                "request_count": 2,
                "tool_call_count": 5
            }
        ]);

        let messages = parse(&serde_json::to_vec(&rows).unwrap()).unwrap();
        assert_eq!(messages[0].text, "Earlier request");
        assert_eq!(
            messages[0].latest_agent_text.as_deref(),
            Some("Earlier response")
        );
        assert_eq!(messages[0].request_count, 3);
        assert_eq!(messages[0].tool_call_count, 8);
        assert_eq!(
            messages[0].activities,
            vec![AgentActivityPreview::Tool {
                name: "read".to_owned(),
                title: Some("src/main.rs".to_owned()),
                running: false,
            }]
        );
        assert!(!messages[0].reasoning_active);
        assert_eq!(messages[1].text, "Latest request\n\nwith context");
        assert_eq!(
            messages[1].latest_agent_text.as_deref(),
            Some("Latest response")
        );
        assert_eq!(messages[1].request_count, 2);
        assert_eq!(messages[1].tool_call_count, 5);
        assert_eq!(
            messages[1].activities,
            vec![
                AgentActivityPreview::Tool {
                    name: "apply_patch".to_owned(),
                    title: Some("Updated files with details".to_owned()),
                    running: true,
                },
                AgentActivityPreview::Reasoning,
            ]
        );
        assert!(messages[1].reasoning_active);
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
