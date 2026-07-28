use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use interprocess::local_socket::Stream;
use serde_json::Value;

use crate::process::{self, Limits};

use super::{AgentSessionIdentity, AgentStatus, AgentTimingKey, HerdrAgent, HerdrWorkspace};

pub(super) struct Environment {
    pub(super) workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FocusEvent {
    pub(super) workspace_id: String,
    pub(super) pane_id: String,
}

pub(super) enum Action {
    CreateWorkspace { path: Option<PathBuf> },
    CreateWorktree { workspace_id: String },
    CloseWorkspace { workspace_id: String },
    RemoveWorktree { workspace_id: String },
    FocusWorkspace { workspace_id: String },
    FocusAgent { pane_id: String },
    RenameWorkspace { workspace_id: String, label: String },
}

pub(super) struct RestoreRequest {
    pub(super) path: PathBuf,
    pub(super) label: String,
    pub(super) linked_worktree: bool,
}

struct ParsedWorkspace {
    workspace: HerdrWorkspace,
    repo_key: Option<String>,
}

#[cfg(not(test))]
pub(super) fn environment() -> Option<Environment> {
    environment_from(
        std::env::var("HERDR_ENV").ok().as_deref(),
        std::env::var("HERDR_WORKSPACE_ID").ok(),
    )
}

pub(super) fn perform(action: Action) -> Result<(), String> {
    run(&action_args(action)).map(|_| ())
}

pub(super) fn session_snapshot() -> Result<(Vec<HerdrWorkspace>, Vec<HerdrAgent>), String> {
    run(&["api".to_owned(), "snapshot".to_owned()]).and_then(|value| parse_snapshot(&value))
}

pub(super) fn watch_focus_events(
    mut on_event: impl FnMut(FocusEvent) -> bool,
) -> Result<(), String> {
    let socket_path = std::env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .ok_or_else(|| "Herdr did not provide its API socket path".to_owned())?;
    let mut stream = connect(&socket_path)
        .map_err(|error| format!("Could not subscribe to Herdr focus events: {error}"))?;
    stream
        .write_all(
            concat!(
                r#"{"id":"hunkle:focus","method":"events.subscribe","params":{"subscriptions":["#,
                r#"{"type":"pane.focused"}]}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .and_then(|()| stream.flush())
        .map_err(|error| format!("Could not subscribe to Herdr focus events: {error}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| format!("Herdr focus event stream failed: {error}"))?
        == 0
    {
        return Err("Herdr focus event stream closed".to_owned());
    }
    let acknowledgement: Value = serde_json::from_str(&line)
        .map_err(|error| format!("Could not read the Herdr focus subscription: {error}"))?;
    if let Some(error) = acknowledgement
        .pointer("/error/message")
        .and_then(Value::as_str)
    {
        return Err(error.to_owned());
    }
    if acknowledgement
        .pointer("/result/type")
        .and_then(Value::as_str)
        != Some("subscription_started")
    {
        return Err("Herdr returned an unexpected focus subscription response".to_owned());
    }

    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("Herdr focus event stream failed: {error}"))?;
        if read == 0 {
            return Err("Herdr focus event stream closed".to_owned());
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|error| format!("Could not read a Herdr focus event: {error}"))?;
        if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
            return Err(error.to_owned());
        }
        let Some(event) = parse_focus_event(&value) else {
            continue;
        };
        if !on_event(event) {
            return Ok(());
        }
    }
}

fn connect(path: &Path) -> std::io::Result<Stream> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, prelude::*};

        Stream::connect(path.to_fs_name::<GenericFilePath>()?)
    }
    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, prelude::*};

        let name = path.to_string_lossy().to_string();
        Stream::connect(name.to_ns_name::<GenericNamespaced>()?)
    }
}

fn parse_focus_event(value: &Value) -> Option<FocusEvent> {
    if value.get("event")?.as_str()? != "pane_focused" {
        return None;
    }
    Some(FocusEvent {
        workspace_id: value.pointer("/data/workspace_id")?.as_str()?.to_owned(),
        pane_id: value.pointer("/data/pane_id")?.as_str()?.to_owned(),
    })
}

pub(super) fn restore(request: RestoreRequest) -> Result<Option<String>, String> {
    run(&restore_args(request)).map(|value| workspace_id_in(&value))
}

pub(super) fn send_command_below(command: String) -> Result<String, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Herdr command prompt is only available inside Herdr".to_owned());
    }
    send_command_below_with(command, run)
}

fn send_command_below_with(
    command: String,
    mut runner: impl FnMut(&[String]) -> Result<Value, String>,
) -> Result<String, String> {
    let neighbor = runner(&[
        "pane".to_owned(),
        "neighbor".to_owned(),
        "--direction".to_owned(),
        "down".to_owned(),
        "--current".to_owned(),
    ])?;
    let pane_id = if let Some(pane_id) = neighbor
        .pointer("/result/neighbor/neighbor_pane_id")
        .and_then(Value::as_str)
    {
        pane_id.to_owned()
    } else {
        let split = runner(&[
            "pane".to_owned(),
            "split".to_owned(),
            "--current".to_owned(),
            "--direction".to_owned(),
            "down".to_owned(),
            "--no-focus".to_owned(),
        ])?;
        split
            .pointer("/result/pane/pane_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "Herdr did not identify the new pane".to_owned())?
    };
    runner(&[
        "pane".to_owned(),
        "run".to_owned(),
        pane_id.clone(),
        command,
    ])?;
    Ok(pane_id)
}

fn environment_from(enabled: Option<&str>, workspace_id: Option<String>) -> Option<Environment> {
    (enabled == Some("1")).then_some(Environment { workspace_id })
}

fn restore_args(request: RestoreRequest) -> Vec<String> {
    let mut args = if request.linked_worktree {
        vec![
            "worktree".to_owned(),
            "open".to_owned(),
            "--path".to_owned(),
        ]
    } else {
        vec![
            "workspace".to_owned(),
            "create".to_owned(),
            "--cwd".to_owned(),
        ]
    };
    args.push(request.path.to_string_lossy().into_owned());
    args.extend(["--label".to_owned(), request.label, "--no-focus".to_owned()]);
    args
}

fn action_args(action: Action) -> Vec<String> {
    match action {
        Action::CreateWorkspace { path } => {
            let mut args = vec!["workspace".to_owned(), "create".to_owned()];
            if let Some(path) = path {
                args.push("--cwd".to_owned());
                args.push(path.to_string_lossy().into_owned());
            }
            args.push("--no-focus".to_owned());
            args
        }
        Action::CreateWorktree { workspace_id } => vec![
            "worktree".to_owned(),
            "create".to_owned(),
            "--workspace".to_owned(),
            workspace_id,
            "--no-focus".to_owned(),
        ],
        Action::CloseWorkspace { workspace_id } => {
            vec!["workspace".to_owned(), "close".to_owned(), workspace_id]
        }
        Action::RemoveWorktree { workspace_id } => vec![
            "worktree".to_owned(),
            "remove".to_owned(),
            "--workspace".to_owned(),
            workspace_id,
        ],
        Action::FocusWorkspace { workspace_id } => {
            vec!["workspace".to_owned(), "focus".to_owned(), workspace_id]
        }
        Action::FocusAgent { pane_id } => {
            vec!["agent".to_owned(), "focus".to_owned(), pane_id]
        }
        Action::RenameWorkspace {
            workspace_id,
            label,
        } => vec![
            "workspace".to_owned(),
            "rename".to_owned(),
            workspace_id,
            label,
        ],
    }
}

fn run(args: &[String]) -> Result<Value, String> {
    let output = process::run(
        Command::new("herdr").args(args),
        Limits::new(4 * 1024 * 1024, 256 * 1024, Duration::from_secs(60)),
    )
    .map_err(|error| format!("Herdr unavailable: {error}"))?;
    if output.timed_out {
        return Err("Herdr command timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("Herdr returned more than 4 MiB".to_owned());
    }
    decode_response(&output.stdout, &output.stderr, output.status.success())
}

fn decode_response(stdout: &[u8], stderr: &[u8], success: bool) -> Result<Value, String> {
    if stdout.iter().all(u8::is_ascii_whitespace) {
        if success {
            return Ok(Value::Null);
        }
        return Err(stderr_detail(stderr).unwrap_or_else(|| "Herdr command failed".to_owned()));
    }
    let value: Value = serde_json::from_slice(stdout).map_err(|error| {
        stderr_detail(stderr).unwrap_or_else(|| format!("Could not read Herdr response: {error}"))
    })?;
    if let Some(error) = value.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Herdr command failed")
            .to_owned());
    }
    if !success {
        return Err("Herdr command failed".to_owned());
    }
    Ok(value)
}

fn stderr_detail(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
}

pub(super) fn parse_snapshot(
    value: &Value,
) -> Result<(Vec<HerdrWorkspace>, Vec<HerdrAgent>), String> {
    let snapshot = value
        .get("result")
        .and_then(|result| result.get("snapshot"))
        .ok_or_else(|| "Herdr returned an invalid session snapshot".to_owned())?;
    let mut workspaces: Vec<ParsedWorkspace> = snapshot
        .get("workspaces")
        .and_then(Value::as_array)
        .ok_or_else(|| "Herdr snapshot has no workspaces".to_owned())?
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            parse_workspace(workspace, snapshot)
                .ok_or_else(|| format!("Herdr snapshot workspace {index} is malformed"))
        })
        .collect::<Result<_, _>>()?;
    assign_worktree_parents(&mut workspaces);
    let mut agents = match snapshot.get("agents") {
        None => Vec::new(),
        Some(agents) => agents
            .as_array()
            .ok_or_else(|| "Herdr snapshot agents are malformed".to_owned())?
            .iter()
            .enumerate()
            .map(|(index, agent)| {
                parse_agent(agent)
                    .ok_or_else(|| format!("Herdr snapshot agent {index} is malformed"))
            })
            .collect::<Result<_, _>>()?,
    };
    if let Some(focused_pane_id) = snapshot.get("focused_pane_id").and_then(Value::as_str) {
        for agent in &mut agents {
            agent.focused = agent.pane_id == focused_pane_id;
        }
    }
    Ok((
        workspaces
            .into_iter()
            .map(|parsed| parsed.workspace)
            .collect(),
        agents,
    ))
}

fn parse_workspace(value: &Value, snapshot: &Value) -> Option<ParsedWorkspace> {
    let worktree = value.get("worktree").filter(|value| value.is_object());
    Some(ParsedWorkspace {
        repo_key: worktree
            .and_then(|worktree| worktree.get("repo_key"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        workspace: HerdrWorkspace {
            id: value.get("workspace_id")?.as_str()?.to_owned(),
            label: value.get("label")?.as_str()?.to_owned(),
            path: workspace_path(value, snapshot),
            branch: None,
            parent_workspace_id: None,
            pane_count: value.get("pane_count").and_then(Value::as_u64).unwrap_or(0) as usize,
            focused: value
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            status: parse_agent_status(value.get("agent_status").and_then(Value::as_str)),
            repo_root: worktree
                .and_then(|worktree| worktree.get("repo_root"))
                .and_then(Value::as_str)
                .map(PathBuf::from),
            linked_worktree: worktree
                .and_then(|worktree| worktree.get("is_linked_worktree"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
    })
}

fn assign_worktree_parents(workspaces: &mut [ParsedWorkspace]) {
    let parent_ids = workspaces
        .iter()
        .map(|worktree| {
            if !worktree.workspace.linked_worktree {
                return None;
            }
            let repo_key = worktree.repo_key.as_deref()?;
            let exact_root = workspaces.iter().find(|candidate| {
                !candidate.workspace.linked_worktree
                    && candidate.workspace.path.as_deref()
                        == worktree.workspace.repo_root.as_deref()
            });
            exact_root
                .or_else(|| {
                    workspaces.iter().find(|candidate| {
                        !candidate.workspace.linked_worktree
                            && candidate.repo_key.as_deref() == Some(repo_key)
                    })
                })
                .map(|parent| parent.workspace.id.clone())
        })
        .collect::<Vec<_>>();
    for (workspace, parent_id) in workspaces.iter_mut().zip(parent_ids) {
        workspace.workspace.parent_workspace_id = parent_id;
    }
}

fn workspace_path(workspace: &Value, snapshot: &Value) -> Option<PathBuf> {
    if let Some(path) = workspace
        .get("worktree")
        .and_then(|worktree| worktree.get("checkout_path"))
        .and_then(Value::as_str)
    {
        return Some(PathBuf::from(path));
    }

    let workspace_id = workspace.get("workspace_id")?.as_str()?;
    let active_tab_id = workspace.get("active_tab_id").and_then(Value::as_str);
    let panes = snapshot.get("panes").and_then(Value::as_array)?;
    let focused_pane_id = snapshot
        .get("layouts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|layout| {
            layout.get("workspace_id").and_then(Value::as_str) == Some(workspace_id)
                && layout.get("tab_id").and_then(Value::as_str) == active_tab_id
        })
        .and_then(|layout| layout.get("focused_pane_id"))
        .and_then(Value::as_str);
    let workspace_panes = || {
        panes
            .iter()
            .filter(|pane| pane.get("workspace_id").and_then(Value::as_str) == Some(workspace_id))
    };
    let pane = focused_pane_id
        .and_then(|focused| {
            workspace_panes()
                .find(|pane| pane.get("pane_id").and_then(Value::as_str) == Some(focused))
        })
        .or_else(|| {
            active_tab_id.and_then(|active_tab| {
                workspace_panes()
                    .find(|pane| pane.get("tab_id").and_then(Value::as_str) == Some(active_tab))
            })
        })
        .or_else(|| workspace_panes().next())?;
    pane.get("foreground_cwd")
        .and_then(Value::as_str)
        .or_else(|| pane.get("cwd").and_then(Value::as_str))
        .map(PathBuf::from)
}

fn parse_agent(value: &Value) -> Option<HerdrAgent> {
    let pane_id = value.get("pane_id")?.as_str()?.to_owned();
    Some(HerdrAgent {
        name: value.get("agent")?.as_str()?.to_owned(),
        session_name: parse_agent_session_name(value),
        workspace_id: value.get("workspace_id")?.as_str()?.to_owned(),
        tab_id: value.get("tab_id")?.as_str()?.to_owned(),
        pane_id: pane_id.clone(),
        focused: value
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        status: parse_agent_status(value.get("agent_status").and_then(Value::as_str)),
        timing_key: parse_agent_session_identity(value)
            .map(AgentTimingKey::Session)
            .or_else(|| {
                value
                    .get("terminal_id")
                    .and_then(Value::as_str)
                    .map(|id| AgentTimingKey::Terminal(id.to_owned()))
            })
            .unwrap_or(AgentTimingKey::Pane(pane_id)),
        state_change_seq: value
            .get("state_change_seq")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

fn parse_agent_session_identity(value: &Value) -> Option<AgentSessionIdentity> {
    let session = value.get("agent_session")?;
    Some(AgentSessionIdentity {
        source: session.get("source")?.as_str()?.to_owned(),
        agent: session.get("agent")?.as_str()?.to_owned(),
        kind: session.get("kind")?.as_str()?.to_owned(),
        value: session.get("value")?.as_str()?.to_owned(),
    })
}

fn parse_agent_session_name(value: &Value) -> Option<String> {
    let title = value
        .get("terminal_title_stripped")
        .and_then(Value::as_str)
        .or_else(|| value.get("terminal_title").and_then(Value::as_str))
        .or_else(|| value.get("title").and_then(Value::as_str))?;
    let title = title
        .split_once(" | ")
        .map_or(title, |(_, session_name)| session_name);
    let title = title
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

fn parse_agent_status(value: Option<&str>) -> AgentStatus {
    match value {
        Some("idle") => AgentStatus::Idle,
        Some("working") => AgentStatus::Working,
        Some("blocked") => AgentStatus::Blocked,
        Some("done") => AgentStatus::Done,
        _ => AgentStatus::Unknown,
    }
}

fn workspace_id_in(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => object
            .get("workspace_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| object.values().find_map(workspace_id_in)),
        Value::Array(values) => values.iter().find_map(workspace_id_in),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parses_focus_events() {
        assert_eq!(
            parse_focus_event(&serde_json::json!({
                "event": "pane_focused",
                "data": { "workspace_id": "w2", "pane_id": "w2:p3" }
            })),
            Some(FocusEvent {
                workspace_id: "w2".to_owned(),
                pane_id: "w2:p3".to_owned(),
            })
        );
    }

    #[test]
    fn builds_typed_action_and_restore_arguments() {
        assert_eq!(
            action_args(Action::CreateWorkspace {
                path: Some(PathBuf::from("/tmp/current workspace")),
            }),
            [
                "workspace",
                "create",
                "--cwd",
                "/tmp/current workspace",
                "--no-focus",
            ]
            .map(str::to_owned)
        );
        assert_eq!(
            action_args(Action::CreateWorktree {
                workspace_id: "w1".to_owned(),
            }),
            ["worktree", "create", "--workspace", "w1", "--no-focus"].map(str::to_owned)
        );
        assert_eq!(
            action_args(Action::CloseWorkspace {
                workspace_id: "w1".to_owned(),
            }),
            ["workspace", "close", "w1"].map(str::to_owned)
        );
        assert_eq!(
            action_args(Action::RemoveWorktree {
                workspace_id: "w3".to_owned(),
            }),
            ["worktree", "remove", "--workspace", "w3"].map(str::to_owned)
        );
        assert_eq!(
            action_args(Action::FocusWorkspace {
                workspace_id: "w1".to_owned(),
            }),
            ["workspace", "focus", "w1"].map(str::to_owned)
        );
        assert_eq!(
            action_args(Action::FocusAgent {
                pane_id: "w1:p2".to_owned(),
            }),
            ["agent", "focus", "w1:p2"].map(str::to_owned)
        );
        assert_eq!(
            action_args(Action::RenameWorkspace {
                workspace_id: "w1".to_owned(),
                label: "code".to_owned(),
            }),
            ["workspace", "rename", "w1", "code"].map(str::to_owned)
        );
        assert_eq!(
            restore_args(RestoreRequest {
                path: PathBuf::from("/tmp/code"),
                label: "Code".to_owned(),
                linked_worktree: false,
            }),
            [
                "workspace",
                "create",
                "--cwd",
                "/tmp/code",
                "--label",
                "Code",
                "--no-focus",
            ]
            .map(str::to_owned)
        );
        assert_eq!(
            restore_args(RestoreRequest {
                path: PathBuf::from("/tmp/feature"),
                label: "Feature".to_owned(),
                linked_worktree: true,
            }),
            [
                "worktree",
                "open",
                "--path",
                "/tmp/feature",
                "--label",
                "Feature",
                "--no-focus",
            ]
            .map(str::to_owned)
        );
    }

    #[test]
    fn sends_to_an_existing_pane_below() {
        let mut calls = Vec::new();
        let pane_id = send_command_below_with("cargo test --lib".to_owned(), |args| {
            calls.push(args.to_vec());
            Ok(if calls.len() == 1 {
                serde_json::json!({
                    "result": { "neighbor": { "neighbor_pane_id": "w1:p2" } }
                })
            } else {
                Value::Null
            })
        })
        .unwrap();

        assert_eq!(pane_id, "w1:p2");
        assert_eq!(
            calls,
            vec![
                ["pane", "neighbor", "--direction", "down", "--current"]
                    .map(str::to_owned)
                    .to_vec(),
                ["pane", "run", "w1:p2", "cargo test --lib"]
                    .map(str::to_owned)
                    .to_vec(),
            ]
        );
    }

    #[test]
    fn accepts_an_empty_success_response() {
        assert_eq!(decode_response(b"", b"", true).unwrap(), Value::Null);
    }

    #[test]
    fn creates_a_pane_below_before_sending() {
        let mut calls = Vec::new();
        let pane_id = send_command_below_with("review this".to_owned(), |args| {
            calls.push(args.to_vec());
            Ok(match calls.len() {
                1 => serde_json::json!({ "result": { "neighbor": {} } }),
                2 => serde_json::json!({ "result": { "pane": { "pane_id": "w1:p3" } } }),
                _ => serde_json::json!({ "result": {} }),
            })
        })
        .unwrap();

        assert_eq!(pane_id, "w1:p3");
        assert_eq!(
            calls,
            vec![
                ["pane", "neighbor", "--direction", "down", "--current"]
                    .map(str::to_owned)
                    .to_vec(),
                [
                    "pane",
                    "split",
                    "--current",
                    "--direction",
                    "down",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
                ["pane", "run", "w1:p3", "review this"]
                    .map(str::to_owned)
                    .to_vec(),
            ]
        );
    }

    #[test]
    fn detects_environment_and_nested_workspace_ids() {
        assert!(environment_from(Some("0"), Some("w1".to_owned())).is_none());
        assert_eq!(
            environment_from(Some("1"), Some("w1".to_owned()))
                .unwrap()
                .workspace_id
                .as_deref(),
            Some("w1")
        );
        let response = serde_json::json!({
            "result": { "event": { "workspace": { "workspace_id": "workspace-42" } } }
        });
        assert_eq!(workspace_id_in(&response).as_deref(), Some("workspace-42"));
    }

    #[test]
    fn parses_paths_statuses_and_repo_key_parent_fallback() {
        let value = serde_json::json!({
            "result": { "snapshot": {
                "focused_pane_id": "pane-3",
                "workspaces": [
                    {
                        "workspace_id": "parent",
                        "label": "Parent",
                        "pane_count": 1,
                        "agent_status": "working",
                        "worktree": {
                            "checkout_path": "/repos/project",
                            "repo_key": "project.git",
                            "repo_root": "/repos/project",
                            "is_linked_worktree": false
                        }
                    },
                    {
                        "workspace_id": "child",
                        "label": "Child",
                        "pane_count": 1,
                        "agent_status": "idle",
                        "worktree": {
                            "checkout_path": "/worktrees/feature",
                            "repo_key": "project.git",
                            "repo_root": "/different/root",
                            "is_linked_worktree": true
                        }
                    },
                    {
                        "workspace_id": "pane-path",
                        "label": "Pane path",
                        "active_tab_id": "tab-3",
                        "pane_count": 1,
                        "agent_status": "done"
                    }
                ],
                "agents": [{
                    "agent": "opencode",
                    "agent_session": {
                        "source": "herdr:opencode",
                        "agent": "opencode",
                        "kind": "id",
                        "value": "ses_timer"
                    },
                    "agent_status": "blocked",
                    "state_change_seq": 17,
                    "terminal_title_stripped": "OC | Refine workspace timers",
                    "terminal_id": "term-3",
                    "focused": false,
                    "pane_id": "pane-3",
                    "tab_id": "tab-3",
                    "workspace_id": "pane-path"
                }],
                "panes": [{
                    "pane_id": "pane-3",
                    "tab_id": "tab-3",
                    "workspace_id": "pane-path",
                    "cwd": "/fallback",
                    "foreground_cwd": "/foreground"
                }],
                "layouts": []
            }}
        });

        let (workspaces, agents) = parse_snapshot(&value).unwrap();
        assert_eq!(workspaces[0].status, AgentStatus::Working);
        assert_eq!(workspaces[1].parent_workspace_id.as_deref(), Some("parent"));
        assert_eq!(
            workspaces[2].path.as_deref(),
            Some(Path::new("/foreground"))
        );
        assert_eq!(workspaces[2].status, AgentStatus::Done);
        assert_eq!(agents[0].status, AgentStatus::Blocked);
        assert!(agents[0].focused);
        assert_eq!(agents[0].state_change_seq, 17);
        assert!(matches!(
            &agents[0].timing_key,
            AgentTimingKey::Session(session) if session.value == "ses_timer"
        ));
        assert_eq!(
            agents[0].session_name.as_deref(),
            Some("Refine workspace timers")
        );
    }

    #[test]
    fn rejects_partial_snapshots_instead_of_dropping_records() {
        let value = serde_json::json!({
            "result": {"snapshot": {
                "workspaces": [
                    {"workspace_id": "valid", "label": "Valid"},
                    {"workspace_id": "missing-label"}
                ],
                "agents": []
            }}
        });

        assert_eq!(
            parse_snapshot(&value).unwrap_err(),
            "Herdr snapshot workspace 1 is malformed"
        );
    }
}
