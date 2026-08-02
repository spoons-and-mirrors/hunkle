use std::{process::Command, time::Duration};

use serde_json::Value;

use crate::process::{self, Limits};

const QUERY_LIMITS: Limits = Limits::new(2 * 1024 * 1024, 64 * 1024, Duration::from_secs(5));

pub(super) fn fetch(session_id: &str) -> Result<Vec<String>, String> {
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
               AND json_extract(m.data, '$.role') = 'user' \
             ORDER BY m.time_created DESC LIMIT 5\
         ) \
         SELECT recent.id AS message_id, json_extract(p.data, '$.text') AS text \
         FROM recent JOIN part p ON p.message_id = recent.id \
         WHERE json_extract(p.data, '$.type') = 'text' \
         ORDER BY recent.time_created, p.time_created"
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

fn parse(output: &[u8]) -> Result<Vec<String>, String> {
    let rows: Value = serde_json::from_slice(output)
        .map_err(|error| format!("OpenCode returned invalid query data: {error}"))?;
    let mut messages: Vec<(String, String)> = Vec::new();
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
            message.push_str("\n\n");
            message.push_str(text);
        } else {
            messages.push((id.to_owned(), text.to_owned()));
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

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn extracts_text_from_the_latest_user_message() {
        let rows = serde_json::json!([
            { "message_id": "one", "text": "Earlier request" },
            { "message_id": "two", "text": "Latest request" },
            { "message_id": "two", "text": "with context" }
        ]);

        assert_eq!(
            parse(&serde_json::to_vec(&rows).unwrap()).unwrap(),
            vec!["Earlier request", "Latest request\n\nwith context"]
        );
    }
}
