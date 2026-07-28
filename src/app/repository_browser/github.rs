use std::{path::Path, process::Command, time::Duration};

use serde_json::Value;

use crate::process::{self, Limits};

use super::{Issue, PullRequest};

pub(super) fn load_pull_requests(root: &Path) -> Result<Vec<PullRequest>, String> {
    let value = run(
        root,
        &[
            "pr",
            "list",
            "--limit",
            "100",
            "--json",
            "number,title,headRefName,author,isDraft",
        ],
    )?;
    parse_pull_requests(&value)
}

pub(super) fn load_issues(root: &Path) -> Result<Vec<Issue>, String> {
    let value = run(
        root,
        &[
            "issue",
            "list",
            "--limit",
            "100",
            "--json",
            "number,title,author,labels",
        ],
    )?;
    parse_issues(&value)
}

fn parse_pull_requests(value: &Value) -> Result<Vec<PullRequest>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "GitHub CLI returned invalid pull request data".to_owned())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let field = |name: &str| {
                item.get(name)
                    .ok_or_else(|| format!("Pull request {index} has no {name}"))
            };
            Ok(PullRequest {
                number: field("number")?
                    .as_u64()
                    .ok_or_else(|| format!("Pull request {index} has an invalid number"))?,
                title: field("title")?
                    .as_str()
                    .ok_or_else(|| format!("Pull request {index} has an invalid title"))?
                    .to_owned(),
                branch: field("headRefName")?
                    .as_str()
                    .ok_or_else(|| format!("Pull request {index} has an invalid headRefName"))?
                    .to_owned(),
                author: author_login(item),
                draft: field("isDraft")?
                    .as_bool()
                    .ok_or_else(|| format!("Pull request {index} has an invalid isDraft"))?,
            })
        })
        .collect()
}

fn parse_issues(value: &Value) -> Result<Vec<Issue>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "GitHub CLI returned invalid issue data".to_owned())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let labels = item
                .get("labels")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Issue {index} has invalid labels"))?
                .iter()
                .enumerate()
                .map(|(label_index, label)| {
                    label.get("name").and_then(Value::as_str).ok_or_else(|| {
                        format!("Issue {index} has an invalid label at {label_index}")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(Issue {
                number: item
                    .get("number")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("Issue {index} has an invalid number"))?,
                title: item
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("Issue {index} has an invalid title"))?
                    .to_owned(),
                author: author_login(item),
                labels,
            })
        })
        .collect()
}

fn author_login(item: &Value) -> String {
    item.get("author")
        .and_then(|author| author.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned()
}

fn run(root: &Path, args: &[&str]) -> Result<Value, String> {
    let output = process::run(
        Command::new("gh")
            .args(args)
            .current_dir(root)
            .env("GH_PROMPT_DISABLED", "1"),
        Limits::new(4 * 1024 * 1024, 256 * 1024, Duration::from_secs(60)),
    )
    .map_err(|error| format!("GitHub CLI unavailable: {error}"))?;
    if output.timed_out {
        return Err("GitHub CLI timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("GitHub CLI returned more than 4 MiB".to_owned());
    }
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr)
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Could not load GitHub data")
            .trim()
            .to_owned();
        return Err(error);
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not read GitHub CLI output: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_items() {
        let pull_requests = serde_json::json!([
            {"number":42,"title":"Branch browser","headRefName":"feature/browser","author":{"login":"octo"},"isDraft":true}
        ]);
        let issues = serde_json::json!([
            {"number":7,"title":"Keyboard navigation","author":null,"labels":[{"name":"ux"}]}
        ]);

        let parsed_pulls = parse_pull_requests(&pull_requests).unwrap();
        assert_eq!(parsed_pulls[0].branch, "feature/browser");
        assert_eq!(parsed_pulls[0].author, "octo");
        let parsed_issues = parse_issues(&issues).unwrap();
        assert_eq!(parsed_issues[0].author, "unknown");
    }

    #[test]
    fn rejects_malformed_items_instead_of_returning_partial_data() {
        let pull_requests = serde_json::json!([
            {"number": 1, "title": "valid", "headRefName": "one", "isDraft": false},
            {"number": 2, "title": "missing branch", "isDraft": false}
        ]);
        let issues = serde_json::json!([
            {"number": 1, "title": "bad label", "labels": [{}]}
        ]);

        assert!(parse_pull_requests(&pull_requests).is_err());
        assert!(parse_issues(&issues).is_err());
    }
}
