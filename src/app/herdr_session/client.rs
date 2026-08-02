use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use std::collections::HashMap;

use interprocess::local_socket::{Stream, traits::Stream as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::process::{self, Limits};

use super::{
    AgentPaneDirection, AgentSessionIdentity, AgentStatus, AgentTimingKey, HerdrAgent,
    HerdrWorkspace,
};

const EVENT_SUBSCRIPTION_REQUEST: &str = concat!(
    r#"{"id":"hunkle:events","method":"events.subscribe","params":{"subscriptions":["#,
    r#"{"type":"pane.agent_status_changed"}]}}"#,
    "\n"
);

pub(super) struct Environment {
    pub(super) workspace_id: Option<String>,
    pub(super) tab_id: Option<String>,
    pub(super) pane_id: Option<String>,
}

pub(super) struct DisplayAgentRequest {
    pub(super) pane_id: String,
    pub(super) workspace_id: String,
    pub(super) tab_id: String,
    pub(super) host_pane_id: String,
    pub(super) host_workspace_id: String,
    pub(super) host_tab_id: String,
    pub(super) allow_cross_workspace: bool,
    pub(super) saved_layout: Option<AgentLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
enum LayoutDirection {
    Right,
    Down,
}

impl LayoutDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
enum LiveLayoutNode {
    Pane(String),
    Split {
        direction: LayoutDirection,
        ratio: f32,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl LiveLayoutNode {
    fn first_pane(&self) -> &str {
        match self {
            Self::Pane(pane_id) => pane_id,
            Self::Split { first, .. } => first.first_pane(),
        }
    }

    fn contains(&self, pane_id: &str) -> bool {
        match self {
            Self::Pane(candidate) => candidate == pane_id,
            Self::Split { first, second, .. } => {
                first.contains(pane_id) || second.contains(pane_id)
            }
        }
    }

    fn collect_panes<'a>(&'a self, panes: &mut Vec<&'a str>) {
        match self {
            Self::Pane(pane_id) => panes.push(pane_id),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }

    fn pane_count(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    fn without_pane(&self, pane_id: &str) -> Option<Self> {
        match self {
            Self::Pane(candidate) => (candidate != pane_id).then(|| self.clone()),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => match (first.without_pane(pane_id), second.without_pane(pane_id)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    direction: *direction,
                    ratio: *ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }

    fn has_same_panes(&self, other: &Self) -> bool {
        let mut left = Vec::new();
        let mut right = Vec::new();
        self.collect_panes(&mut left);
        other.collect_panes(&mut right);
        left.sort_unstable();
        right.sort_unstable();
        left == right
    }

    fn remap(&self, panes: &HashMap<String, LivePaneLocation>) -> Result<Self, String> {
        match self {
            Self::Pane(pane_id) => panes
                .get(pane_id)
                .map(|location| Self::Pane(location.pane_id.clone()))
                .ok_or_else(|| format!("Saved layout pane {pane_id} is unavailable")),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => Ok(Self::Split {
                direction: *direction,
                ratio: *ratio,
                first: Box::new(first.remap(panes)?),
                second: Box::new(second.remap(panes)?),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(super) struct AgentLayout {
    root: LiveLayoutNode,
}

#[derive(Debug)]
pub(super) struct AgentLayoutMoveResult {
    pub(super) layout: AgentLayout,
    pub(super) pane_locations: HashMap<String, LivePaneLocation>,
}

#[derive(Debug)]
pub(super) struct DisplayAgentResult {
    pub(super) displayed: AgentLayout,
    pub(super) parked: Option<AgentLayout>,
    pub(super) pane_locations: HashMap<String, LivePaneLocation>,
}

#[derive(Debug, Clone, PartialEq)]
struct LiveLayout {
    workspace_id: String,
    tab_id: String,
    zoomed: bool,
    focused_pane_id: String,
    root: LiveLayoutNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LivePaneLocation {
    pub(super) pane_id: String,
    pub(super) tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrPaneLayout {
    pub(crate) workspace_id: String,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) panes: Vec<HerdrPaneRect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HerdrPaneRect {
    pub(crate) pane_id: String,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentStatusEvent {
    pub(super) workspace_id: String,
    pub(super) pane_id: String,
    pub(super) status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Event {
    AgentStatus(AgentStatusEvent),
}

struct EventCoalescer {
    replaying: bool,
}

impl EventCoalescer {
    fn new() -> Self {
        Self { replaying: true }
    }

    fn observe(&mut self, event: Event) -> Option<Event> {
        if self.replaying { None } else { Some(event) }
    }

    fn finish_replay(&mut self) {
        self.replaying = false;
    }
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
        std::env::var("HERDR_TAB_ID").ok(),
        std::env::var("HERDR_PANE_ID").ok(),
    )
}

pub(super) fn session_snapshot() -> Result<(Vec<HerdrWorkspace>, Vec<HerdrAgent>), String> {
    run(&["api".to_owned(), "snapshot".to_owned()]).and_then(|value| parse_snapshot(&value))
}

pub(super) fn display_agent(request: DisplayAgentRequest) -> Result<DisplayAgentResult, String> {
    display_agent_with(request, run, api_request)
}

pub(super) fn park_agent_layout(
    request: DisplayAgentRequest,
    label: String,
) -> Result<AgentLayoutMoveResult, String> {
    park_agent_layout_with(request, label, run, api_request)
}

fn park_agent_layout_with<F, A>(
    request: DisplayAgentRequest,
    label: String,
    mut runner: F,
    mut api: A,
) -> Result<AgentLayoutMoveResult, String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    let host = export_layout(&mut api, &request.host_tab_id)?;
    if host.workspace_id != request.host_workspace_id
        || host.tab_id != request.host_tab_id
        || host.focused_pane_id != request.host_pane_id
    {
        return Err("Hunkle's Herdr tab changed before the layout could be parked".to_owned());
    }
    if host.zoomed {
        return Err("Agent layouts cannot be parked while the Herdr tab is zoomed".to_owned());
    }
    let outgoing = layout_without_host(&host.root, &request.host_pane_id)?
        .ok_or_else(|| "Hunkle has no agent layout to park".to_owned())?;
    if !outgoing.contains(&request.pane_id) {
        return Err("The selected agent is not in Hunkle's displayed layout".to_owned());
    }
    let mut panes = pane_locations(&[(&host.root, host.tab_id.as_str())])?;
    let root = outgoing.first_pane().to_owned();
    let moved = move_pane_to_new_tab(
        &mut runner,
        &mut panes,
        &root,
        &request.host_workspace_id,
        &label,
    )?;
    let park = (|| {
        rebuild_layout(&mut runner, &outgoing, &moved.tab_id, &mut panes)?;
        let parked = export_layout(&mut api, &moved.tab_id)?;
        verify_layout(&parked, &outgoing, &panes, "parked")?;
        let remaining_host = export_layout(&mut api, &host.tab_id)?;
        verify_layout(
            &remaining_host,
            &LiveLayoutNode::Pane(request.host_pane_id.clone()),
            &panes,
            "remaining Hunkle",
        )?;
        if remaining_host.focused_pane_id != request.host_pane_id {
            return Err("Herdr did not preserve focus in Hunkle".to_owned());
        }
        Ok(AgentLayoutMoveResult {
            layout: AgentLayout {
                root: host.root.remap(&panes)?,
            },
            pane_locations: panes.clone(),
        })
    })();
    match park {
        Ok(result) => Ok(result),
        Err(error) => {
            let restore = scatter_layout(
                &mut runner,
                &outgoing,
                &request.host_workspace_id,
                "restore-agent-layout",
                &mut panes,
            )
            .and_then(|_| rebuild_layout(&mut runner, &host.root, &host.tab_id, &mut panes))
            .and_then(|()| export_layout(&mut api, &host.tab_id))
            .and_then(|layout| verify_layout(&layout, &host.root, &panes, "previous displayed"));
            Err(match restore {
                Ok(()) => format!("{error}; restored the previous layout"),
                Err(restore) => {
                    format!("{error}; could not restore the previous layout: {restore}")
                }
            })
        }
    }
}

pub(super) fn restore_agent_layout(
    request: DisplayAgentRequest,
) -> Result<AgentLayoutMoveResult, String> {
    restore_agent_layout_with(request, run, api_request)
}

fn restore_agent_layout_with<F, A>(
    request: DisplayAgentRequest,
    mut runner: F,
    mut api: A,
) -> Result<AgentLayoutMoveResult, String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    if request.workspace_id != request.host_workspace_id && !request.allow_cross_workspace {
        return Err("Cross-workspace agents are disabled in Settings".to_owned());
    }
    let host = export_layout(&mut api, &request.host_tab_id)?;
    let selected = export_layout(&mut api, &request.tab_id)?;
    validate_layouts(&request, &host, &selected)?;
    if layout_without_host(&host.root, &request.host_pane_id)?.is_some() {
        return Err("Hunkle already has an agent layout to replace".to_owned());
    }
    let target = request
        .saved_layout
        .as_ref()
        .filter(|layout| saved_layout_matches(layout, &selected.root, &request.host_pane_id))
        .map(|layout| layout.root.clone())
        .unwrap_or_else(|| {
            host_with_layout(request.host_pane_id.clone(), 0.6, selected.root.clone())
        });
    let mut panes = pane_locations(&[
        (&host.root, host.tab_id.as_str()),
        (&selected.root, selected.tab_id.as_str()),
    ])?;
    let restore = (|| {
        rebuild_layout(&mut runner, &target, &host.tab_id, &mut panes)?;
        let final_host = export_layout(&mut api, &host.tab_id)?;
        verify_layout(&final_host, &target, &panes, "displayed")?;
        if final_host.focused_pane_id != request.host_pane_id {
            return Err("Herdr did not preserve focus in Hunkle".to_owned());
        }
        if selected.root.pane_count() > 1 {
            refresh_rebuilt_agent(&mut api, &request.pane_id, &request.host_pane_id, &panes)?;
        }
        Ok(AgentLayoutMoveResult {
            layout: AgentLayout {
                root: final_host.root,
            },
            pane_locations: panes.clone(),
        })
    })();
    match restore {
        Ok(result) => Ok(result),
        Err(error) => {
            let recovery = scatter_layout(
                &mut runner,
                &selected.root,
                &request.workspace_id,
                "restore-agent-layout",
                &mut panes,
            )
            .and_then(|tab_id| {
                rebuild_layout(&mut runner, &selected.root, &tab_id, &mut panes)?;
                let parked = export_layout(&mut api, &tab_id)?;
                verify_layout(&parked, &selected.root, &panes, "previous parked")
            })
            .and_then(|()| {
                let restored_host = export_layout(&mut api, &host.tab_id)?;
                verify_layout(&restored_host, &host.root, &panes, "previous Hunkle")
            });
            Err(match recovery {
                Ok(()) => format!("{error}; restored the previous layout"),
                Err(recovery) => {
                    format!("{error}; could not restore the previous layout: {recovery}")
                }
            })
        }
    }
}

fn display_agent_with<F, A>(
    request: DisplayAgentRequest,
    mut runner: F,
    mut api: A,
) -> Result<DisplayAgentResult, String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    if request.workspace_id != request.host_workspace_id && !request.allow_cross_workspace {
        return Err("Cross-workspace agents are disabled in Settings".to_owned());
    }
    if request.tab_id == request.host_tab_id {
        let host = export_layout(&mut api, &request.host_tab_id)?;
        validate_layouts(&request, &host, &host)?;
        let layout = AgentLayout { root: host.root };
        let mut pane_ids = Vec::new();
        layout.root.collect_panes(&mut pane_ids);
        let pane_locations = pane_ids
            .into_iter()
            .map(|pane_id| {
                (
                    pane_id.to_owned(),
                    LivePaneLocation {
                        pane_id: pane_id.to_owned(),
                        tab_id: host.tab_id.clone(),
                    },
                )
            })
            .collect();
        return Ok(DisplayAgentResult {
            displayed: layout.clone(),
            parked: Some(layout),
            pane_locations,
        });
    }

    let host = export_layout(&mut api, &request.host_tab_id)?;
    let selected = export_layout(&mut api, &request.tab_id)?;
    validate_layouts(&request, &host, &selected)?;
    let host_ratio = host_layout_ratio(&host.root, &request.host_pane_id);
    let outgoing = layout_without_host(&host.root, &request.host_pane_id)?.ok_or_else(|| {
        "Hunkle needs another pane before an agent layout can be exchanged".to_owned()
    })?;
    let selected_target = request
        .saved_layout
        .as_ref()
        .filter(|layout| saved_layout_matches(layout, &selected.root, &request.host_pane_id))
        .map(|layout| layout.root.clone())
        .unwrap_or_else(|| {
            host_with_layout(
                request.host_pane_id.clone(),
                host_ratio,
                selected.root.clone(),
            )
        });

    let mut panes = HashMap::new();
    for (tree, tab_id) in [
        (&host.root, host.tab_id.as_str()),
        (&selected.root, selected.tab_id.as_str()),
    ] {
        let mut pane_ids = Vec::new();
        tree.collect_panes(&mut pane_ids);
        for pane_id in pane_ids {
            if panes
                .insert(
                    pane_id.to_owned(),
                    LivePaneLocation {
                        pane_id: pane_id.to_owned(),
                        tab_id: tab_id.to_owned(),
                    },
                )
                .is_some()
            {
                return Err("Herdr returned duplicate panes in the saved layouts".to_owned());
            }
        }
    }
    let outgoing_root = outgoing.first_pane().to_owned();
    let selected_root = selected.root.first_pane().to_owned();
    move_layout_pane(
        &mut runner,
        &mut panes,
        &outgoing_root,
        &selected.tab_id,
        &selected_root,
        LayoutDirection::Right,
        0.5,
    )?;

    let exchange = (|| {
        rebuild_layout(&mut runner, &outgoing, &selected.tab_id, &mut panes)?;
        rebuild_layout(&mut runner, &selected_target, &host.tab_id, &mut panes)?;

        let final_host = export_layout(&mut api, &host.tab_id)?;
        verify_layout(&final_host, &selected_target, &panes, "displayed")?;
        if final_host.focused_pane_id != request.host_pane_id {
            return Err("Herdr did not preserve focus in Hunkle".to_owned());
        }
        let parked = export_layout(&mut api, &selected.tab_id)?;
        verify_layout(&parked, &outgoing, &panes, "parked")?;
        if outgoing.pane_count() > 1 || selected.root.pane_count() > 1 {
            refresh_rebuilt_agent(&mut api, &request.pane_id, &request.host_pane_id, &panes)?;
        }
        Ok(DisplayAgentResult {
            displayed: AgentLayout {
                root: final_host.root,
            },
            parked: Some(AgentLayout {
                root: host.root.remap(&panes)?,
            }),
            pane_locations: panes.clone(),
        })
    })();
    match exchange {
        Ok(result) => Ok(result),
        Err(error) => {
            let restore = restore_layouts(
                &mut runner,
                &mut api,
                &request,
                &host,
                &selected,
                &outgoing,
                &mut panes,
            );
            Err(match restore {
                Ok(()) => format!("{error}; restored the previous layouts"),
                Err(restore) => {
                    format!("{error}; could not restore the previous layouts: {restore}")
                }
            })
        }
    }
}

fn host_with_layout(host_pane_id: String, ratio: f32, layout: LiveLayoutNode) -> LiveLayoutNode {
    LiveLayoutNode::Split {
        direction: LayoutDirection::Right,
        ratio,
        first: Box::new(LiveLayoutNode::Pane(host_pane_id)),
        second: Box::new(layout),
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_layouts<F, A>(
    runner: &mut F,
    api: &mut A,
    request: &DisplayAgentRequest,
    host: &LiveLayout,
    selected: &LiveLayout,
    outgoing: &LiveLayoutNode,
    panes: &mut HashMap<String, LivePaneLocation>,
) -> Result<(), String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    let outgoing_root = outgoing.first_pane().to_owned();
    normalize_layout(runner, outgoing, &selected.tab_id, &outgoing_root, panes)?;
    normalize_layout(
        runner,
        &selected.root,
        &host.tab_id,
        &request.host_pane_id,
        panes,
    )?;

    let selected_root = selected.root.first_pane().to_owned();
    move_layout_pane(
        runner,
        panes,
        &selected_root,
        &selected.tab_id,
        &outgoing_root,
        LayoutDirection::Right,
        0.5,
    )?;
    rebuild_layout(runner, &selected.root, &selected.tab_id, panes)?;
    rebuild_layout(runner, &host.root, &host.tab_id, panes)?;

    let restored_host = export_layout(api, &host.tab_id)?;
    verify_layout(&restored_host, &host.root, panes, "previous displayed")?;
    if restored_host.focused_pane_id != request.host_pane_id {
        return Err("Herdr did not restore focus in Hunkle".to_owned());
    }
    let restored_selected = export_layout(api, &selected.tab_id)?;
    verify_layout(&restored_selected, &selected.root, panes, "previous parked")
}

fn normalize_layout<F>(
    runner: &mut F,
    tree: &LiveLayoutNode,
    tab_id: &str,
    target: &str,
    panes: &mut HashMap<String, LivePaneLocation>,
) -> Result<(), String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
{
    let mut pane_ids = Vec::new();
    tree.collect_panes(&mut pane_ids);
    for pane_id in pane_ids {
        if panes
            .get(pane_id)
            .is_some_and(|location| location.tab_id == tab_id)
        {
            continue;
        }
        move_layout_pane(
            runner,
            panes,
            pane_id,
            tab_id,
            target,
            LayoutDirection::Right,
            0.5,
        )?;
    }
    Ok(())
}

fn pane_locations(
    trees: &[(&LiveLayoutNode, &str)],
) -> Result<HashMap<String, LivePaneLocation>, String> {
    let mut panes = HashMap::new();
    for (tree, tab_id) in trees {
        let mut pane_ids = Vec::new();
        tree.collect_panes(&mut pane_ids);
        for pane_id in pane_ids {
            if panes
                .insert(
                    pane_id.to_owned(),
                    LivePaneLocation {
                        pane_id: pane_id.to_owned(),
                        tab_id: (*tab_id).to_owned(),
                    },
                )
                .is_some()
            {
                return Err("Herdr returned duplicate panes in the saved layouts".to_owned());
            }
        }
    }
    Ok(panes)
}

fn move_pane_to_new_tab<F>(
    runner: &mut F,
    panes: &mut HashMap<String, LivePaneLocation>,
    pane: &str,
    workspace_id: &str,
    label: &str,
) -> Result<LivePaneLocation, String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
{
    let source = panes
        .get(pane)
        .ok_or_else(|| format!("Saved layout pane {pane} is unavailable"))?;
    let args = vec![
        "pane".to_owned(),
        "move".to_owned(),
        source.pane_id.clone(),
        "--new-tab".to_owned(),
        "--workspace".to_owned(),
        workspace_id.to_owned(),
        "--label".to_owned(),
        label.to_owned(),
        "--no-focus".to_owned(),
    ];
    let value = runner(&args)?;
    let result = require_changed(&value, "/result/move_result", "pane move")?;
    let location = LivePaneLocation {
        pane_id: result
            .pointer("/pane/pane_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Herdr returned an invalid pane move result".to_owned())?
            .to_owned(),
        tab_id: result
            .pointer("/pane/tab_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Herdr returned an invalid pane move result".to_owned())?
            .to_owned(),
    };
    panes.insert(pane.to_owned(), location.clone());
    Ok(location)
}

fn scatter_layout<F>(
    runner: &mut F,
    tree: &LiveLayoutNode,
    workspace_id: &str,
    label: &str,
    panes: &mut HashMap<String, LivePaneLocation>,
) -> Result<String, String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
{
    let root = tree.first_pane().to_owned();
    let mut pane_ids = Vec::new();
    tree.collect_panes(&mut pane_ids);
    for pane_id in pane_ids {
        move_pane_to_new_tab(runner, panes, pane_id, workspace_id, label)?;
    }
    panes
        .get(&root)
        .map(|location| location.tab_id.clone())
        .ok_or_else(|| format!("Saved layout pane {root} is unavailable"))
}

fn export_layout<A>(api: &mut A, tab_id: &str) -> Result<LiveLayout, String>
where
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    let value = api("layout.export", &serde_json::json!({ "tab_id": tab_id }))?;
    parse_live_layout(
        value
            .pointer("/result/layout")
            .ok_or_else(|| "Herdr returned an invalid layout export".to_owned())?,
    )
}

fn parse_live_layout(value: &Value) -> Result<LiveLayout, String> {
    fn text(value: &Value, field: &str) -> Result<String, String> {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("Herdr layout is missing {field}"))
    }
    fn node(value: &Value) -> Result<LiveLayoutNode, String> {
        match value.get("type").and_then(Value::as_str) {
            Some("pane") => Ok(LiveLayoutNode::Pane(text(value, "pane_id")?)),
            Some("split") => {
                let direction = match value.get("direction").and_then(Value::as_str) {
                    Some("right") => LayoutDirection::Right,
                    Some("down") => LayoutDirection::Down,
                    _ => return Err("Herdr layout has an invalid split direction".to_owned()),
                };
                let ratio = value
                    .get("ratio")
                    .and_then(Value::as_f64)
                    .map(|ratio| ratio as f32)
                    .filter(|ratio| ratio.is_finite())
                    .ok_or_else(|| "Herdr layout has an invalid split ratio".to_owned())?;
                Ok(LiveLayoutNode::Split {
                    direction,
                    ratio,
                    first: Box::new(node(
                        value
                            .get("first")
                            .ok_or_else(|| "Herdr layout is missing its first child".to_owned())?,
                    )?),
                    second: Box::new(node(
                        value
                            .get("second")
                            .ok_or_else(|| "Herdr layout is missing its second child".to_owned())?,
                    )?),
                })
            }
            _ => Err("Herdr layout has an invalid node".to_owned()),
        }
    }

    Ok(LiveLayout {
        workspace_id: text(value, "workspace_id")?,
        tab_id: text(value, "tab_id")?,
        zoomed: value
            .get("zoomed")
            .and_then(Value::as_bool)
            .ok_or_else(|| "Herdr layout is missing zoom state".to_owned())?,
        focused_pane_id: text(value, "focused_pane_id")?,
        root: node(
            value
                .get("root")
                .ok_or_else(|| "Herdr layout is missing its root".to_owned())?,
        )?,
    })
}

fn validate_layouts(
    request: &DisplayAgentRequest,
    host: &LiveLayout,
    selected: &LiveLayout,
) -> Result<(), String> {
    if host.workspace_id != request.host_workspace_id || host.tab_id != request.host_tab_id {
        return Err("Hunkle's Herdr tab changed before the layout could be displayed".to_owned());
    }
    if selected.workspace_id != request.workspace_id || selected.tab_id != request.tab_id {
        return Err("The selected agent moved before its layout could be displayed".to_owned());
    }
    if host.zoomed || selected.zoomed {
        return Err("Agent layouts cannot be exchanged while either tab is zoomed".to_owned());
    }
    if host.focused_pane_id != request.host_pane_id {
        return Err("Hunkle must be focused before displaying an agent layout".to_owned());
    }
    if !selected.root.contains(&request.pane_id) {
        return Err("The selected agent moved before its layout could be displayed".to_owned());
    }
    Ok(())
}

fn layout_without_host(
    root: &LiveLayoutNode,
    host_pane_id: &str,
) -> Result<Option<LiveLayoutNode>, String> {
    if root.first_pane() != host_pane_id || !root.contains(host_pane_id) {
        return Err("Hunkle must be the first pane in the displayed agent layout".to_owned());
    }
    Ok(root.without_pane(host_pane_id))
}

fn host_layout_ratio(root: &LiveLayoutNode, host_pane_id: &str) -> f32 {
    match root {
        LiveLayoutNode::Split {
            direction: LayoutDirection::Right,
            ratio,
            first,
            ..
        } if first.contains(host_pane_id) => *ratio,
        _ => 0.6,
    }
}

fn saved_layout_matches(layout: &AgentLayout, parked: &LiveLayoutNode, host_pane_id: &str) -> bool {
    layout.root.first_pane() == host_pane_id
        && layout
            .root
            .without_pane(host_pane_id)
            .is_some_and(|saved| saved.has_same_panes(parked))
}

fn refresh_rebuilt_agent<A>(
    api: &mut A,
    agent_pane: &str,
    host_pane: &str,
    panes: &HashMap<String, LivePaneLocation>,
) -> Result<(), String>
where
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    let agent_pane = panes
        .get(agent_pane)
        .map(|location| location.pane_id.as_str())
        .ok_or_else(|| format!("Saved layout pane {agent_pane} is unavailable"))?;
    focus_pane(api, agent_pane, "refresh the displayed agent")?;
    focus_pane(api, host_pane, "restore focus in Hunkle")
}

fn focus_pane<A>(api: &mut A, pane_id: &str, operation: &str) -> Result<(), String>
where
    A: FnMut(&str, &Value) -> Result<Value, String>,
{
    let value = api("pane.focus", &serde_json::json!({ "pane_id": pane_id }))?;
    let result = value
        .get("result")
        .ok_or_else(|| format!("Herdr returned an invalid response while trying to {operation}"))?;
    if result.get("type").and_then(Value::as_str) != Some("pane_focused")
        || result.get("pane_id").and_then(Value::as_str) != Some(pane_id)
    {
        return Err(format!(
            "Herdr focused an unexpected pane while trying to {operation}"
        ));
    }
    Ok(())
}

fn rebuild_layout<F>(
    runner: &mut F,
    tree: &LiveLayoutNode,
    tab_id: &str,
    panes: &mut HashMap<String, LivePaneLocation>,
) -> Result<(), String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
{
    let LiveLayoutNode::Split {
        direction,
        ratio,
        first,
        second,
    } = tree
    else {
        return Ok(());
    };
    let target = first.first_pane().to_owned();
    let moved = second.first_pane().to_owned();
    move_layout_pane(runner, panes, &moved, tab_id, &target, *direction, *ratio)?;
    rebuild_layout(runner, first, tab_id, panes)?;
    rebuild_layout(runner, second, tab_id, panes)
}

fn move_layout_pane<F>(
    runner: &mut F,
    panes: &mut HashMap<String, LivePaneLocation>,
    pane: &str,
    tab_id: &str,
    target: &str,
    direction: LayoutDirection,
    ratio: f32,
) -> Result<String, String>
where
    F: FnMut(&[String]) -> Result<Value, String>,
{
    let location = panes
        .get(pane)
        .ok_or_else(|| format!("Saved layout pane {pane} is unavailable"))?
        .clone();
    let target_location = panes
        .get(target)
        .ok_or_else(|| format!("Saved layout pane {target} is unavailable"))?
        .clone();
    if location.tab_id == tab_id {
        return Err(format!(
            "Saved layout pane {} is already in tab {tab_id}",
            location.pane_id
        ));
    }
    if target_location.tab_id != tab_id {
        return Err(format!(
            "Saved layout target {} is not in tab {tab_id}",
            target_location.pane_id
        ));
    }
    let args = vec![
        "pane".to_owned(),
        "move".to_owned(),
        location.pane_id,
        "--tab".to_owned(),
        tab_id.to_owned(),
        "--target-pane".to_owned(),
        target_location.pane_id,
        "--split".to_owned(),
        direction.as_str().to_owned(),
        "--ratio".to_owned(),
        ratio.to_string(),
        "--no-focus".to_owned(),
    ];
    let value = runner(&args)?;
    let result = require_changed(&value, "/result/move_result", "pane move")?;
    let moved = result
        .pointer("/pane/pane_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "Herdr returned an invalid pane move result".to_owned())?;
    if result.pointer("/pane/tab_id").and_then(Value::as_str) != Some(tab_id) {
        return Err("Herdr moved a pane to an unexpected tab".to_owned());
    }
    panes.insert(
        pane.to_owned(),
        LivePaneLocation {
            pane_id: moved.clone(),
            tab_id: tab_id.to_owned(),
        },
    );
    Ok(moved)
}

fn require_changed<'a>(
    value: &'a Value,
    pointer: &str,
    operation: &str,
) -> Result<&'a Value, String> {
    let result = value
        .pointer(pointer)
        .ok_or_else(|| format!("Herdr returned an invalid {operation} result"))?;
    match result.get("changed").and_then(Value::as_bool) {
        Some(true) => Ok(result),
        Some(false) => {
            let reason = result
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unchanged");
            Err(format!("Herdr did not change the layout: {reason}"))
        }
        None => Err(format!("Herdr returned an invalid {operation} result")),
    }
}

fn verify_layout(
    actual: &LiveLayout,
    expected: &LiveLayoutNode,
    panes: &HashMap<String, LivePaneLocation>,
    name: &str,
) -> Result<(), String> {
    fn matches_tree(
        actual: &LiveLayoutNode,
        expected: &LiveLayoutNode,
        panes: &HashMap<String, LivePaneLocation>,
    ) -> bool {
        match (actual, expected) {
            (LiveLayoutNode::Pane(actual), LiveLayoutNode::Pane(expected)) => panes
                .get(expected)
                .is_some_and(|location| location.pane_id == *actual),
            (
                LiveLayoutNode::Split {
                    direction: actual_direction,
                    ratio: actual_ratio,
                    first: actual_first,
                    second: actual_second,
                },
                LiveLayoutNode::Split {
                    direction: expected_direction,
                    ratio: expected_ratio,
                    first: expected_first,
                    second: expected_second,
                },
            ) => {
                actual_direction == expected_direction
                    && (actual_ratio - expected_ratio).abs() < 0.000_01
                    && matches_tree(actual_first, expected_first, panes)
                    && matches_tree(actual_second, expected_second, panes)
            }
            _ => false,
        }
    }

    if matches_tree(&actual.root, expected, panes) {
        Ok(())
    } else {
        Err(format!("Herdr did not restore the {name} agent layout"))
    }
}

pub(super) fn watch_events(mut on_event: impl FnMut(Event) -> bool) -> Result<(), String> {
    let socket_path = std::env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .ok_or_else(|| "Herdr did not provide its API socket path".to_owned())?;
    let mut stream = connect(&socket_path)
        .map_err(|error| format!("Could not subscribe to Herdr events: {error}"))?;
    stream
        .write_all(EVENT_SUBSCRIPTION_REQUEST.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|error| format!("Could not subscribe to Herdr events: {error}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| format!("Herdr event stream failed: {error}"))?
        == 0
    {
        return Err("Herdr event stream closed".to_owned());
    }
    let acknowledgement: Value = serde_json::from_str(&line)
        .map_err(|error| format!("Could not read the Herdr event subscription: {error}"))?;
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
        return Err("Herdr returned an unexpected event subscription response".to_owned());
    }

    // Official Herdr releases replay retained events at 100 ms intervals. Collapse that history
    // at startup, then remove the timeout so future changes pass through immediately. Historical
    // status events have no timestamps, so the initial snapshot establishes timing state instead.
    reader
        .get_ref()
        .set_recv_timeout(Some(Duration::from_millis(200)))
        .map_err(|error| format!("Could not configure the Herdr event stream: {error}"))?;
    let mut events = EventCoalescer::new();
    loop {
        line.clear();
        let read = match reader.read_line(&mut line) {
            Ok(read) => read,
            Err(error)
                if events.replaying
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
            {
                reader.get_ref().set_recv_timeout(None).map_err(|error| {
                    format!("Could not configure the Herdr event stream: {error}")
                })?;
                events.finish_replay();
                continue;
            }
            Err(error) => return Err(format!("Herdr event stream failed: {error}")),
        };
        if read == 0 {
            return Err("Herdr event stream closed".to_owned());
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|error| format!("Could not read a Herdr event: {error}"))?;
        if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
            return Err(error.to_owned());
        }
        let Some(event) = parse_event(&value) else {
            continue;
        };
        if let Some(event) = events.observe(event)
            && !on_event(event)
        {
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

fn api_request(method: &str, params: &Value) -> Result<Value, String> {
    let socket_path = std::env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .ok_or_else(|| "Herdr did not provide its API socket path".to_owned())?;
    let mut stream = connect(&socket_path)
        .map_err(|error| format!("Could not connect to the Herdr API: {error}"))?;
    stream
        .set_recv_timeout(Some(Duration::from_secs(60)))
        .map_err(|error| format!("Could not configure the Herdr API connection: {error}"))?;
    let mut request = serde_json::to_vec(&serde_json::json!({
        "id": format!("hunkle:{method}"),
        "method": method,
        "params": params,
    }))
    .map_err(|error| format!("Could not encode the Herdr API request: {error}"))?;
    request.push(b'\n');
    stream
        .write_all(&request)
        .and_then(|()| stream.flush())
        .map_err(|error| format!("Could not send the Herdr API request: {error}"))?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    if reader
        .read_line(&mut response)
        .map_err(|error| format!("Could not read the Herdr API response: {error}"))?
        == 0
    {
        return Err("Herdr closed the API connection without responding".to_owned());
    }
    let value: Value = serde_json::from_str(&response)
        .map_err(|error| format!("Could not decode the Herdr API response: {error}"))?;
    if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
        Err(message.to_owned())
    } else {
        Ok(value)
    }
}

fn parse_event(value: &Value) -> Option<Event> {
    let workspace_id = value.pointer("/data/workspace_id")?.as_str()?.to_owned();
    let pane_id = value.pointer("/data/pane_id")?.as_str()?.to_owned();
    match value.get("event")?.as_str()? {
        "pane_agent_status_changed" => Some(Event::AgentStatus(AgentStatusEvent {
            workspace_id,
            pane_id,
            status: parse_agent_status(value.pointer("/data/agent_status").and_then(Value::as_str)),
        })),
        _ => None,
    }
}

pub(super) fn pane_layout(pane_id: String) -> Result<HerdrPaneLayout, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Pane layouts are only available inside Herdr".to_owned());
    }
    pane_layout_with(pane_id, run)
}

pub(super) fn toggle_pane_zoom(pane_id: String) -> Result<bool, String> {
    toggle_pane_zoom_with(pane_id, run)
}

fn toggle_pane_zoom_with(
    pane_id: String,
    mut runner: impl FnMut(&[String]) -> Result<Value, String>,
) -> Result<bool, String> {
    let value = runner(&[
        "pane".to_owned(),
        "zoom".to_owned(),
        "--pane".to_owned(),
        pane_id,
        "--toggle".to_owned(),
    ])?;
    value
        .pointer("/result/zoom/zoomed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Herdr did not report the fullscreen state".to_owned())
}

fn pane_layout_with(
    pane_id: String,
    mut runner: impl FnMut(&[String]) -> Result<Value, String>,
) -> Result<HerdrPaneLayout, String> {
    let value = runner(&[
        "pane".to_owned(),
        "layout".to_owned(),
        "--pane".to_owned(),
        pane_id,
    ])?;
    parse_pane_layout(
        value
            .pointer("/result/layout")
            .ok_or_else(|| "Herdr did not return the tab layout".to_owned())?,
    )
}

fn parse_pane_layout(layout: &Value) -> Result<HerdrPaneLayout, String> {
    fn text(value: &Value, field: &str) -> Result<String, String> {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("Herdr layout is missing {field}"))
    }
    fn coordinate(value: &Value, field: &str) -> Result<u16, String> {
        value
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| format!("Herdr layout has an invalid {field}"))
    }

    let area = layout
        .get("area")
        .ok_or_else(|| "Herdr layout is missing its area".to_owned())?;
    let panes = layout
        .get("panes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Herdr layout is missing its panes".to_owned())?
        .iter()
        .map(|pane| {
            let rect = pane
                .get("rect")
                .ok_or_else(|| "Herdr pane is missing its rectangle".to_owned())?;
            Ok(HerdrPaneRect {
                pane_id: text(pane, "pane_id")?,
                x: coordinate(rect, "x")?,
                y: coordinate(rect, "y")?,
                width: coordinate(rect, "width")?,
                height: coordinate(rect, "height")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let width = coordinate(area, "width")?;
    let height = coordinate(area, "height")?;
    if width == 0 || height == 0 || panes.is_empty() {
        return Err("Herdr returned an empty tab layout".to_owned());
    }
    Ok(HerdrPaneLayout {
        workspace_id: text(layout, "workspace_id")?,
        x: coordinate(area, "x")?,
        y: coordinate(area, "y")?,
        width,
        height,
        panes,
    })
}

pub(super) fn send_command_below(command: String) -> Result<String, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Herdr command prompt is only available inside Herdr".to_owned());
    }
    send_command_below_with(command, run)
}

pub(super) fn replace_pane_with_agent(
    path: PathBuf,
    workspace_id: String,
    pane_id: String,
    session_id: Option<String>,
) -> Result<String, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Agents can only be started inside Herdr".to_owned());
    }
    replace_pane_with_agent_with(path, workspace_id, pane_id, session_id, run)
}

pub(super) fn split_pane_with_agent(
    path: PathBuf,
    pane_id: String,
    direction: AgentPaneDirection,
    session_id: Option<String>,
) -> Result<String, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Agents can only be started inside Herdr".to_owned());
    }
    split_pane_with_agent_with(path, pane_id, direction, session_id, run)
}

pub(super) fn close_pane(pane_id: String) -> Result<(), String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Agents can only be managed inside Herdr".to_owned());
    }
    close_pane_with(pane_id, run)
}

fn close_pane_with(
    pane_id: String,
    mut runner: impl FnMut(&[String]) -> Result<Value, String>,
) -> Result<(), String> {
    runner(&["pane".to_owned(), "close".to_owned(), pane_id]).map(|_| ())
}

fn split_pane_with_agent_with(
    path: PathBuf,
    pane_id: String,
    direction: AgentPaneDirection,
    session_id: Option<String>,
    runner: impl FnMut(&[String]) -> Result<Value, String>,
) -> Result<String, String> {
    let mut runner = runner;
    let command = opencode_command(session_id.as_deref())?;
    let split = runner(&[
        "pane".to_owned(),
        "split".to_owned(),
        pane_id,
        "--direction".to_owned(),
        direction.as_str().to_owned(),
        "--cwd".to_owned(),
        path.to_string_lossy().into_owned(),
        "--no-focus".to_owned(),
    ])?;
    let pane_id = split
        .pointer("/result/pane/pane_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "Herdr did not identify the new pane".to_owned())?;
    let run_args = vec![
        "pane".to_owned(),
        "run".to_owned(),
        pane_id.clone(),
        command,
    ];
    if let Err(error) = runner(&run_args) {
        let _ = runner(&["pane".to_owned(), "close".to_owned(), pane_id]);
        return Err(error);
    }
    Ok(pane_id)
}

fn replace_pane_with_agent_with(
    path: PathBuf,
    workspace_id: String,
    pane_id: String,
    session_id: Option<String>,
    runner: impl FnMut(&[String]) -> Result<Value, String>,
) -> Result<String, String> {
    let mut runner = runner;
    let command = opencode_command(session_id.as_deref())?;
    let pane = runner(&["pane".to_owned(), "get".to_owned(), pane_id.clone()])?;
    let parked_tab_label = pane
        .pointer("/result/pane/cwd")
        .and_then(Value::as_str)
        .and_then(|cwd| Path::new(cwd).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "agent".to_owned());
    let split = runner(&[
        "pane".to_owned(),
        "split".to_owned(),
        pane_id.clone(),
        "--direction".to_owned(),
        "right".to_owned(),
        "--cwd".to_owned(),
        path.to_string_lossy().into_owned(),
        "--no-focus".to_owned(),
    ])?;
    let replacement_pane_id = split
        .pointer("/result/pane/pane_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "Herdr did not identify the new pane".to_owned())?;
    let run_args = vec![
        "pane".to_owned(),
        "run".to_owned(),
        replacement_pane_id.clone(),
        command,
    ];
    if let Err(error) = runner(&run_args) {
        let _ = runner(&["pane".to_owned(), "close".to_owned(), replacement_pane_id]);
        return Err(error);
    }
    let move_args = vec![
        "pane".to_owned(),
        "move".to_owned(),
        pane_id.clone(),
        "--new-tab".to_owned(),
        "--workspace".to_owned(),
        workspace_id,
        "--label".to_owned(),
        parked_tab_label,
        "--no-focus".to_owned(),
    ];
    if let Err(error) = runner(&move_args) {
        let _ = runner(&["pane".to_owned(), "close".to_owned(), replacement_pane_id]);
        return Err(error);
    }
    Ok(replacement_pane_id)
}

fn opencode_command(session_id: Option<&str>) -> Result<String, String> {
    let mut command = String::from("opencode");
    if let Some(session_id) = session_id {
        if session_id.is_empty()
            || !session_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("OpenCode reported an invalid session ID".to_owned());
        }
        command.push_str(" --session ");
        command.push_str(session_id);
    }
    Ok(command)
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

fn environment_from(
    enabled: Option<&str>,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    pane_id: Option<String>,
) -> Option<Environment> {
    (enabled == Some("1")).then_some(Environment {
        workspace_id,
        tab_id,
        pane_id,
    })
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
                parse_agent(agent, snapshot)
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

fn parse_agent(value: &Value, snapshot: &Value) -> Option<HerdrAgent> {
    let pane_id = value.get("pane_id")?.as_str()?.to_owned();
    let name = value.get("agent")?.as_str()?.to_owned();
    let session_timing_key = parse_agent_session_identity(value).map(AgentTimingKey::Session);
    let pane = snapshot
        .get("panes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|pane| pane.get("pane_id").and_then(Value::as_str) == Some(pane_id.as_str()));
    Some(HerdrAgent {
        name: name.clone(),
        session_name: parse_agent_session_name(value),
        workspace_id: value.get("workspace_id")?.as_str()?.to_owned(),
        tab_id: value.get("tab_id")?.as_str()?.to_owned(),
        pane_id: pane_id.clone(),
        cwd: pane
            .and_then(|pane| {
                pane.get("foreground_cwd")
                    .and_then(Value::as_str)
                    .or_else(|| pane.get("cwd").and_then(Value::as_str))
            })
            .map(PathBuf::from),
        destination_cwd: pane
            .and_then(|pane| pane.get("cwd").and_then(Value::as_str))
            .map(PathBuf::from),
        focused: value
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        status: parse_agent_status(value.get("agent_status").and_then(Value::as_str)),
        timing_key: value
            .get("terminal_id")
            .and_then(Value::as_str)
            .map(|terminal| AgentTimingKey::Terminal(format!("{name}@{terminal}")))
            .unwrap_or_else(|| AgentTimingKey::Pane(format!("{name}@{pane_id}"))),
        session_timing_key,
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn pane(pane_id: &str) -> Value {
        serde_json::json!({ "type": "pane", "pane_id": pane_id })
    }

    fn split(direction: &str, ratio: f32, first: Value, second: Value) -> Value {
        serde_json::json!({
            "type": "split",
            "direction": direction,
            "ratio": ratio,
            "first": first,
            "second": second,
        })
    }

    fn layout(workspace_id: &str, tab_id: &str, focused_pane_id: &str, root: Value) -> Value {
        serde_json::json!({
            "result": {
                "layout": {
                    "workspace_id": workspace_id,
                    "tab_id": tab_id,
                    "zoomed": false,
                    "focused_pane_id": focused_pane_id,
                    "root": root,
                }
            }
        })
    }

    fn agent_layout(root: Value) -> AgentLayout {
        let value = layout("w1", "w1:t1", "w1:p1", root);
        AgentLayout {
            root: parse_live_layout(value.pointer("/result/layout").unwrap())
                .unwrap()
                .root,
        }
    }

    fn moved(pane_id: &str, tab_id: &str) -> Value {
        serde_json::json!({
            "result": {
                "move_result": {
                    "changed": true,
                    "pane": { "pane_id": pane_id, "tab_id": tab_id }
                }
            }
        })
    }

    fn focused(pane_id: &str) -> Value {
        serde_json::json!({
            "result": {
                "type": "pane_focused",
                "pane_id": pane_id,
            }
        })
    }

    #[test]
    fn parses_agent_status_events() {
        assert_eq!(
            parse_event(&serde_json::json!({
                "event": "pane_agent_status_changed",
                "data": {
                    "workspace_id": "w2",
                    "pane_id": "w2:p3",
                    "agent_status": "blocked"
                }
            })),
            Some(Event::AgentStatus(AgentStatusEvent {
                workspace_id: "w2".to_owned(),
                pane_id: "w2:p3".to_owned(),
                status: AgentStatus::Blocked,
            }))
        );
    }

    #[test]
    fn subscribes_only_to_supported_herdr_events() {
        let request: Value = serde_json::from_str(EVENT_SUBSCRIPTION_REQUEST.trim()).unwrap();
        assert_eq!(
            request.pointer("/params/subscriptions"),
            Some(&serde_json::json!([
                { "type": "pane.agent_status_changed" }
            ]))
        );
    }

    #[test]
    fn discards_replayed_events_before_forwarding_live_events() {
        let mut events = EventCoalescer::new();
        let event = || {
            Event::AgentStatus(AgentStatusEvent {
                workspace_id: "w1".to_owned(),
                pane_id: "w1:p1".to_owned(),
                status: AgentStatus::Working,
            })
        };

        assert_eq!(events.observe(event()), None);
        events.finish_replay();
        assert_eq!(events.observe(event()), Some(event()));
    }

    #[test]
    fn reads_a_pane_layout() {
        let mut calls = Vec::new();
        let layout = pane_layout_with("w2:p3".to_owned(), |args| {
            calls.push(args.to_vec());
            Ok(serde_json::json!({
                "result": {
                    "layout": {
                        "area": { "x": 4, "y": 1, "width": 120, "height": 40 },
                        "panes": [
                            {
                                "pane_id": "w2:p1",
                                "rect": { "x": 4, "y": 1, "width": 72, "height": 40 }
                            },
                            {
                                "pane_id": "w2:p3",
                                "rect": { "x": 76, "y": 1, "width": 48, "height": 40 }
                            }
                        ],
                        "tab_id": "w2:t4",
                        "workspace_id": "w2"
                    }
                }
            }))
        })
        .unwrap();

        assert_eq!(layout.workspace_id, "w2");
        assert_eq!((layout.width, layout.height), (120, 40));
        assert_eq!(layout.panes.len(), 2);
        assert_eq!(layout.panes[1].pane_id, "w2:p3");
        assert_eq!((layout.panes[1].x, layout.panes[1].width), (76, 48));
        assert_eq!(
            calls,
            vec![
                ["pane", "layout", "--pane", "w2:p3"]
                    .map(str::to_owned)
                    .to_vec()
            ]
        );
    }

    #[test]
    fn toggles_hunkle_pane_zoom_and_reads_the_result() {
        let mut calls = Vec::new();
        let zoomed = toggle_pane_zoom_with("w2:p3".to_owned(), |args| {
            calls.push(args.to_vec());
            Ok(serde_json::json!({
                "result": { "zoom": { "zoomed": true } }
            }))
        })
        .unwrap();

        assert!(zoomed);
        assert_eq!(
            calls,
            vec![
                ["pane", "zoom", "--pane", "w2:p3", "--toggle"]
                    .map(str::to_owned)
                    .to_vec()
            ]
        );
    }

    #[test]
    fn selecting_an_agent_in_the_displayed_tab_captures_the_layout() {
        let mut exports = 0;
        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t1".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |_| panic!("An already displayed layout must not move panes"),
            |_, _| {
                exports += 1;
                Ok(layout(
                    "w1",
                    "w1:t1",
                    "w1:p1",
                    split("right", 0.6, pane("w1:p1"), pane("w1:p3")),
                ))
            },
        )
        .unwrap();
        assert_eq!(exports, 1);
        assert_eq!(result.displayed, result.parked.unwrap());
    }

    #[test]
    fn exchanges_complete_saved_layouts_between_tabs() {
        let outgoing = split(
            "right",
            0.7,
            split("down", 0.3, pane("w1:p2"), pane("w1:p6")),
            pane("w1:p4"),
        );
        let selected = split(
            "down",
            0.4,
            pane("w1:p3"),
            split("right", 0.8, pane("w1:p5"), pane("w1:p7")),
        );
        let mut commands = Vec::new();
        let mut exports = 0;

        display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |args| {
                commands.push(args.to_vec());
                Ok(moved(&args[2], &args[4]))
            },
            |method, params| {
                if method == "pane.focus" {
                    return Ok(focused(params["pane_id"].as_str().unwrap()));
                }
                assert_eq!(method, "layout.export");
                exports += 1;
                Ok(match exports {
                    1 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), outgoing.clone()),
                    ),
                    2 => layout("w1", "w1:t2", "w1:p3", selected.clone()),
                    3 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), selected.clone()),
                    ),
                    4 => layout("w1", "w1:t2", "w1:p2", outgoing.clone()),
                    _ => panic!("unexpected layout export for {params}"),
                })
            },
        )
        .unwrap();

        let moves = commands
            .iter()
            .map(|args| {
                (
                    args[2].as_str(),
                    args[4].as_str(),
                    args[6].as_str(),
                    args[8].as_str(),
                    args[10].as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            moves,
            vec![
                ("w1:p2", "w1:t2", "w1:p3", "right", "0.5"),
                ("w1:p4", "w1:t2", "w1:p2", "right", "0.7"),
                ("w1:p6", "w1:t2", "w1:p2", "down", "0.3"),
                ("w1:p3", "w1:t1", "w1:p1", "right", "0.6"),
                ("w1:p5", "w1:t1", "w1:p3", "down", "0.4"),
                ("w1:p7", "w1:t1", "w1:p5", "right", "0.8"),
            ]
        );
        assert!(
            commands
                .iter()
                .all(|args| args.last().unwrap() == "--no-focus")
        );
        assert_eq!(exports, 4);
    }

    #[test]
    fn panes_below_hunkle_belong_to_the_displayed_agent_layout() {
        let outgoing = split(
            "right",
            0.6,
            split("down", 0.4, pane("w1:p1"), pane("w1:p8")),
            pane("w1:p2"),
        );
        let parked_outgoing = split("right", 0.6, pane("w1:p8"), pane("w1:p2"));
        let selected = pane("w1:p3");
        let mut commands = Vec::new();
        let mut exports = 0;
        let mut focuses = Vec::new();

        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |args| {
                commands.push(args.to_vec());
                Ok(moved(&args[2], &args[4]))
            },
            |method, params| {
                if method == "pane.focus" {
                    let pane_id = params["pane_id"].as_str().unwrap();
                    focuses.push(pane_id.to_owned());
                    return Ok(focused(pane_id));
                }
                exports += 1;
                Ok(match exports {
                    1 => layout("w1", "w1:t1", "w1:p1", outgoing.clone()),
                    2 => layout("w1", "w1:t2", "w1:p3", selected.clone()),
                    3 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), selected.clone()),
                    ),
                    4 => layout("w1", "w1:t2", "w1:p8", parked_outgoing.clone()),
                    _ => panic!("unexpected layout export"),
                })
            },
        )
        .unwrap();

        assert_eq!(
            commands,
            vec![
                [
                    "pane",
                    "move",
                    "w1:p8",
                    "--tab",
                    "w1:t2",
                    "--target-pane",
                    "w1:p3",
                    "--split",
                    "right",
                    "--ratio",
                    "0.5",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
                [
                    "pane",
                    "move",
                    "w1:p2",
                    "--tab",
                    "w1:t2",
                    "--target-pane",
                    "w1:p8",
                    "--split",
                    "right",
                    "--ratio",
                    "0.6",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
                [
                    "pane",
                    "move",
                    "w1:p3",
                    "--tab",
                    "w1:t1",
                    "--target-pane",
                    "w1:p1",
                    "--split",
                    "right",
                    "--ratio",
                    "0.6",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
            ]
        );
        assert_eq!(
            result.displayed,
            agent_layout(split("right", 0.6, pane("w1:p1"), pane("w1:p3")))
        );
        assert_eq!(result.parked.unwrap(), agent_layout(outgoing));
        assert_eq!(focuses, ["w1:p3", "w1:p1"]);
    }

    #[test]
    fn restores_a_saved_agent_pane_below_hunkle() {
        let outgoing = pane("w1:p2");
        let selected = split("right", 0.5, pane("w1:p8"), pane("w1:p3"));
        let saved = split(
            "right",
            0.6,
            split("down", 0.4, pane("w1:p1"), pane("w1:p8")),
            pane("w1:p3"),
        );
        let mut commands = Vec::new();
        let mut exports = 0;

        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: Some(agent_layout(saved.clone())),
            },
            |args| {
                commands.push(args.to_vec());
                Ok(moved(&args[2], &args[4]))
            },
            |method, params| {
                if method == "pane.focus" {
                    return Ok(focused(params["pane_id"].as_str().unwrap()));
                }
                exports += 1;
                Ok(match exports {
                    1 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), outgoing.clone()),
                    ),
                    2 => layout("w1", "w1:t2", "w1:p3", selected.clone()),
                    3 => layout("w1", "w1:t1", "w1:p1", saved.clone()),
                    4 => layout("w1", "w1:t2", "w1:p2", outgoing.clone()),
                    _ => panic!("unexpected layout export"),
                })
            },
        )
        .unwrap();

        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0][2], "w1:p2");
        assert_eq!(commands[1][2], "w1:p3");
        assert_eq!(commands[1][8], "right");
        assert_eq!(commands[1][10], "0.6");
        assert_eq!(commands[2][2], "w1:p8");
        assert_eq!(commands[2][8], "down");
        assert_eq!(commands[2][10], "0.4");
        assert_eq!(result.displayed, agent_layout(saved));
    }

    #[test]
    fn restores_an_agent_owned_pane_below_hunkle_when_rebuild_fails() {
        let outgoing = split(
            "right",
            0.6,
            split("down", 0.4, pane("w1:p1"), pane("w1:p8")),
            pane("w1:p2"),
        );
        let selected = pane("w1:p3");
        let mut commands = 0;
        let mut exports = 0;
        let mut tabs = HashMap::from([
            ("w1:p1".to_owned(), "w1:t1".to_owned()),
            ("w1:p2".to_owned(), "w1:t1".to_owned()),
            ("w1:p3".to_owned(), "w1:t2".to_owned()),
            ("w1:p8".to_owned(), "w1:t1".to_owned()),
        ]);

        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |args| {
                commands += 1;
                if commands == 2 {
                    return Err("agent layout rebuild interrupted".to_owned());
                }
                assert_ne!(tabs.get(&args[2]), Some(&args[4]));
                tabs.insert(args[2].clone(), args[4].clone());
                Ok(moved(&args[2], &args[4]))
            },
            |_, _| {
                exports += 1;
                Ok(match exports {
                    1 | 3 => layout("w1", "w1:t1", "w1:p1", outgoing.clone()),
                    2 | 4 => layout("w1", "w1:t2", "w1:p3", selected.clone()),
                    _ => panic!("unexpected layout export"),
                })
            },
        );

        assert_eq!(
            result.unwrap_err(),
            "agent layout rebuild interrupted; restored the previous layouts"
        );
        assert_eq!(commands, 7);
        assert_eq!(exports, 4);
        assert_eq!(tabs.get("w1:p8").map(String::as_str), Some("w1:t1"));
    }

    #[test]
    fn rejects_a_layout_exchange_without_a_layout_to_park() {
        let mut exports = 0;
        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |_| panic!("Preflight failure must not move panes"),
            |_, _| {
                exports += 1;
                Ok(if exports == 1 {
                    layout("w1", "w1:t1", "w1:p1", pane("w1:p1"))
                } else {
                    layout("w1", "w1:t2", "w1:p3", pane("w1:p3"))
                })
            },
        );

        assert_eq!(
            result.unwrap_err(),
            "Hunkle needs another pane before an agent layout can be exchanged"
        );
    }

    #[test]
    fn restores_both_layouts_when_reconstruction_fails() {
        let outgoing = split("down", 0.7, pane("w1:p2"), pane("w1:p4"));
        let selected = pane("w1:p3");
        let mut commands = 0;
        let mut exports = 0;
        let mut tabs = HashMap::from([
            ("w1:p2".to_owned(), "w1:t1".to_owned()),
            ("w1:p3".to_owned(), "w1:t2".to_owned()),
            ("w1:p4".to_owned(), "w1:t1".to_owned()),
        ]);

        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |args| {
                commands += 1;
                if commands == 2 {
                    Err("pane move interrupted".to_owned())
                } else {
                    assert_ne!(tabs.get(&args[2]), Some(&args[4]));
                    tabs.insert(args[2].clone(), args[4].clone());
                    Ok(moved(&args[2], &args[4]))
                }
            },
            |_, _| {
                exports += 1;
                Ok(match exports {
                    1 | 3 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), outgoing.clone()),
                    ),
                    2 | 4 => layout("w1", "w1:t2", "w1:p3", selected.clone()),
                    _ => panic!("unexpected layout export"),
                })
            },
        );

        assert_eq!(
            result.unwrap_err(),
            "pane move interrupted; restored the previous layouts"
        );
        assert_eq!(commands, 7);
        assert_eq!(exports, 4);
    }

    #[test]
    fn restores_both_layouts_when_selected_reconstruction_fails() {
        let outgoing = pane("w1:p2");
        let selected = split("down", 0.4, pane("w1:p3"), pane("w1:p5"));
        let mut commands = 0;
        let mut exports = 0;
        let mut tabs = HashMap::from([
            ("w1:p2".to_owned(), "w1:t1".to_owned()),
            ("w1:p3".to_owned(), "w1:t2".to_owned()),
            ("w1:p5".to_owned(), "w1:t2".to_owned()),
        ]);

        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |args| {
                commands += 1;
                if commands == 3 {
                    Err("selected layout move interrupted".to_owned())
                } else {
                    assert_ne!(tabs.get(&args[2]), Some(&args[4]));
                    tabs.insert(args[2].clone(), args[4].clone());
                    Ok(moved(&args[2], &args[4]))
                }
            },
            |_, _| {
                exports += 1;
                Ok(match exports {
                    1 | 3 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), outgoing.clone()),
                    ),
                    2 | 4 => layout("w1", "w1:t2", "w1:p3", selected.clone()),
                    _ => panic!("unexpected layout export"),
                })
            },
        );

        assert_eq!(
            result.unwrap_err(),
            "selected layout move interrupted; restored the previous layouts"
        );
        assert_eq!(commands, 7);
        assert_eq!(exports, 4);
    }

    #[test]
    fn tracks_reassigned_pane_ids_while_exchanging_workspaces() {
        let outgoing = pane("w1:p2");
        let selected = split("down", 0.4, pane("w2:p3"), pane("w2:p5"));
        let mut commands = Vec::new();
        let mut exports = 0;

        display_agent_with(
            DisplayAgentRequest {
                pane_id: "w2:p3".to_owned(),
                workspace_id: "w2".to_owned(),
                tab_id: "w2:t1".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: true,
                saved_layout: None,
            },
            |args| {
                commands.push(args.to_vec());
                Ok(match commands.len() {
                    1 => moved("w2:p8", "w2:t1"),
                    2 => moved("w1:p8", "w1:t1"),
                    3 => moved("w1:p9", "w1:t1"),
                    _ => panic!("unexpected pane move"),
                })
            },
            |method, params| {
                if method == "pane.focus" {
                    return Ok(focused(params["pane_id"].as_str().unwrap()));
                }
                exports += 1;
                Ok(match exports {
                    1 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), outgoing.clone()),
                    ),
                    2 => layout("w2", "w2:t1", "w2:p3", selected.clone()),
                    3 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split(
                            "right",
                            0.6,
                            pane("w1:p1"),
                            split("down", 0.4, pane("w1:p8"), pane("w1:p9")),
                        ),
                    ),
                    4 => layout("w2", "w2:t1", "w2:p8", pane("w2:p8")),
                    _ => panic!("unexpected layout export"),
                })
            },
        )
        .unwrap();

        assert_eq!(commands[1][2], "w2:p3");
        assert_eq!(commands[2][2], "w2:p5");
        assert_eq!(commands[2][6], "w1:p8");
    }

    #[test]
    fn rejects_an_unchanged_layout_move() {
        let mut commands = Vec::new();
        let mut exports = 0;
        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w1:p3".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |args| {
                commands.push(args.to_vec());
                Ok(serde_json::json!({
                    "result": {
                        "move_result": { "changed": false, "reason": "not_found" }
                    }
                }))
            },
            |method, params| {
                assert_eq!(method, "layout.export");
                exports += 1;
                Ok(match exports {
                    1 => layout(
                        "w1",
                        "w1:t1",
                        "w1:p1",
                        split("right", 0.6, pane("w1:p1"), pane("w1:p2")),
                    ),
                    2 => layout("w1", "w1:t2", "w1:p3", pane("w1:p3")),
                    count => panic!("unexpected layout export {count}: {params}"),
                })
            },
        );

        assert_eq!(
            result.unwrap_err(),
            "Herdr did not change the layout: not_found"
        );
        assert_eq!(exports, 2);
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn refuses_cross_workspace_agents_without_the_setting() {
        let result = display_agent_with(
            DisplayAgentRequest {
                pane_id: "w2:p1".to_owned(),
                workspace_id: "w2".to_owned(),
                tab_id: "w2:t1".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: None,
            },
            |_| panic!("Herdr must not be called"),
            |_, _| panic!("Herdr must not be called"),
        );

        assert_eq!(
            result.unwrap_err(),
            "Cross-workspace agents are disabled in Settings"
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
    fn replaces_a_focused_pane_with_a_resumed_opencode_agent() {
        let mut calls = Vec::new();
        let pane_id = replace_pane_with_agent_with(
            PathBuf::from("/tmp/feature"),
            "w1".to_owned(),
            "w1:p2".to_owned(),
            Some("ses_123".to_owned()),
            |args| {
                calls.push(args.to_vec());
                Ok(match calls.len() {
                    1 => serde_json::json!({
                        "result": { "pane": { "cwd": "/tmp/displaced" } }
                    }),
                    2 => {
                        serde_json::json!({ "result": { "pane": { "pane_id": "w1:p4" } } })
                    }
                    _ => Value::Null,
                })
            },
        )
        .unwrap();

        assert_eq!(pane_id, "w1:p4");
        assert_eq!(
            calls,
            vec![
                ["pane", "get", "w1:p2"].map(str::to_owned).to_vec(),
                [
                    "pane",
                    "split",
                    "w1:p2",
                    "--direction",
                    "right",
                    "--cwd",
                    "/tmp/feature",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
                ["pane", "run", "w1:p4", "opencode --session ses_123"]
                    .map(str::to_owned)
                    .to_vec(),
                [
                    "pane",
                    "move",
                    "w1:p2",
                    "--new-tab",
                    "--workspace",
                    "w1",
                    "--label",
                    "displaced",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
            ]
        );
    }

    #[test]
    fn splits_next_to_a_pane_and_resumes_an_opencode_agent() {
        let mut calls = Vec::new();
        let pane_id = split_pane_with_agent_with(
            PathBuf::from("/tmp/feature"),
            "w1:p2".to_owned(),
            AgentPaneDirection::Up,
            Some("ses_123".to_owned()),
            |args| {
                calls.push(args.to_vec());
                Ok(if calls.len() == 1 {
                    serde_json::json!({ "result": { "pane": { "pane_id": "w1:p4" } } })
                } else {
                    Value::Null
                })
            },
        )
        .unwrap();

        assert_eq!(pane_id, "w1:p4");
        assert_eq!(
            calls,
            vec![
                [
                    "pane",
                    "split",
                    "w1:p2",
                    "--direction",
                    "up",
                    "--cwd",
                    "/tmp/feature",
                    "--no-focus",
                ]
                .map(str::to_owned)
                .to_vec(),
                ["pane", "run", "w1:p4", "opencode --session ses_123"]
                    .map(str::to_owned)
                    .to_vec(),
            ]
        );
    }

    #[test]
    fn rejects_unsafe_opencode_session_ids_before_changing_the_layout() {
        let mut called = false;
        let error = split_pane_with_agent_with(
            PathBuf::from("/tmp/feature"),
            "w1:p2".to_owned(),
            AgentPaneDirection::Up,
            Some("ses_123; rm -rf /".to_owned()),
            |_| {
                called = true;
                Ok(Value::Null)
            },
        )
        .unwrap_err();

        assert_eq!(error, "OpenCode reported an invalid session ID");
        assert!(!called);
    }

    #[test]
    fn closes_a_stashed_agent_pane() {
        let mut calls = Vec::new();

        close_pane_with("w1:p4".to_owned(), |args| {
            calls.push(args.to_vec());
            Ok(Value::Null)
        })
        .unwrap();

        assert_eq!(
            calls,
            vec![["pane", "close", "w1:p4"].map(str::to_owned).to_vec()]
        );
    }

    #[test]
    fn names_each_displaced_layout_from_its_pane_directory() {
        let mut calls = Vec::new();
        replace_pane_with_agent_with(
            PathBuf::from("/tmp/feature"),
            "w1".to_owned(),
            "w1:p2".to_owned(),
            None,
            |args| {
                calls.push(args.to_vec());
                Ok(match calls.len() {
                    1 => serde_json::json!({
                        "result": { "pane": { "cwd": "/home/spoon/code/hunkle" } }
                    }),
                    2 => {
                        serde_json::json!({ "result": { "pane": { "pane_id": "w1:p4" } } })
                    }
                    _ => Value::Null,
                })
            },
        )
        .unwrap();

        assert_eq!(
            calls[3],
            [
                "pane",
                "move",
                "w1:p2",
                "--new-tab",
                "--workspace",
                "w1",
                "--label",
                "hunkle",
                "--no-focus",
            ]
            .map(str::to_owned)
            .to_vec()
        );
    }

    #[test]
    fn leaves_the_focused_pane_in_place_when_agent_dispatch_fails() {
        let mut calls = Vec::new();
        let error = replace_pane_with_agent_with(
            PathBuf::from("/tmp/feature"),
            "w1".to_owned(),
            "w1:p2".to_owned(),
            None,
            |args| {
                calls.push(args.to_vec());
                match calls.len() {
                    1 => Ok(serde_json::json!({
                        "result": { "pane": { "cwd": "/tmp/displaced" } }
                    })),
                    2 => Ok(serde_json::json!({
                        "result": { "pane": { "pane_id": "w1:p4" } }
                    })),
                    3 => Err("agent failed".to_owned()),
                    _ => Ok(Value::Null),
                }
            },
        )
        .unwrap_err();

        assert_eq!(error, "agent failed");
        assert_eq!(
            calls[3],
            ["pane", "close", "w1:p4"].map(str::to_owned).to_vec()
        );
    }

    #[test]
    fn parks_and_restores_complete_agent_layouts_without_focus() {
        let agent = split("down", 0.37, pane("w1:p2"), pane("w1:p6"));
        let displayed = split("right", 0.58, pane("w1:p1"), agent.clone());
        let request = DisplayAgentRequest {
            pane_id: "w1:p2".to_owned(),
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            host_pane_id: "w1:p1".to_owned(),
            host_workspace_id: "w1".to_owned(),
            host_tab_id: "w1:t1".to_owned(),
            allow_cross_workspace: false,
            saved_layout: None,
        };
        let mut park_commands = Vec::new();
        let mut exports = 0;
        let parked = park_agent_layout_with(
            request,
            "hunkle".to_owned(),
            |args| {
                park_commands.push(args.to_vec());
                Ok(if args.contains(&"--new-tab".to_owned()) {
                    moved("w1:p2", "w1:t2")
                } else {
                    moved(&args[2], &args[4])
                })
            },
            |method, _| {
                assert_eq!(method, "layout.export");
                exports += 1;
                Ok(match exports {
                    1 => layout("w1", "w1:t1", "w1:p1", displayed.clone()),
                    2 => layout("w1", "w1:t2", "w1:p2", agent.clone()),
                    3 => layout("w1", "w1:t1", "w1:p1", pane("w1:p1")),
                    _ => panic!("unexpected layout export"),
                })
            },
        )
        .unwrap();
        assert_eq!(parked.layout, agent_layout(displayed.clone()));
        assert_eq!(parked.pane_locations["w1:p6"].tab_id, "w1:t2");

        let mut restore_commands = Vec::new();
        let mut exports = 0;
        let restored = restore_agent_layout_with(
            DisplayAgentRequest {
                pane_id: "w1:p2".to_owned(),
                workspace_id: "w1".to_owned(),
                tab_id: "w1:t2".to_owned(),
                host_pane_id: "w1:p1".to_owned(),
                host_workspace_id: "w1".to_owned(),
                host_tab_id: "w1:t1".to_owned(),
                allow_cross_workspace: false,
                saved_layout: Some(parked.layout),
            },
            |args| {
                restore_commands.push(args.to_vec());
                Ok(moved(&args[2], &args[4]))
            },
            |method, params| {
                if method == "pane.focus" {
                    return Ok(focused(params["pane_id"].as_str().unwrap()));
                }
                exports += 1;
                Ok(match exports {
                    1 => layout("w1", "w1:t1", "w1:p1", pane("w1:p1")),
                    2 => layout("w1", "w1:t2", "w1:p2", agent.clone()),
                    3 => layout("w1", "w1:t1", "w1:p1", displayed.clone()),
                    _ => panic!("unexpected layout export"),
                })
            },
        )
        .unwrap();
        assert_eq!(restored.layout, agent_layout(displayed));
        assert_eq!(
            park_commands
                .iter()
                .chain(&restore_commands)
                .map(|args| args.last().map(String::as_str))
                .collect::<Vec<_>>(),
            vec![Some("--no-focus"); 4]
        );
        assert_eq!(park_commands[1][8], "down");
        assert_eq!(park_commands[1][10], "0.37");
        assert_eq!(restore_commands[0][8], "right");
        assert_eq!(restore_commands[0][10], "0.58");
        assert_eq!(restore_commands[1][8], "down");
        assert_eq!(restore_commands[1][10], "0.37");
    }

    #[test]
    fn detects_environment() {
        assert!(
            environment_from(
                Some("0"),
                Some("w1".to_owned()),
                Some("w1:t1".to_owned()),
                Some("w1:p1".to_owned()),
            )
            .is_none()
        );
        let environment = environment_from(
            Some("1"),
            Some("w1".to_owned()),
            Some("w1:t1".to_owned()),
            Some("w1:p1".to_owned()),
        )
        .unwrap();
        assert_eq!(environment.workspace_id.as_deref(), Some("w1"));
        assert_eq!(environment.tab_id.as_deref(), Some("w1:t1"));
        assert_eq!(environment.pane_id.as_deref(), Some("w1:p1"));
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
        assert_eq!(workspaces[1].parent_workspace_id.as_deref(), Some("parent"));
        assert_eq!(
            workspaces[2].path.as_deref(),
            Some(Path::new("/foreground"))
        );
        assert_eq!(agents[0].status, AgentStatus::Blocked);
        assert!(agents[0].focused);
        assert_eq!(agents[0].state_change_seq, 17);
        assert!(matches!(
            &agents[0].timing_key,
            AgentTimingKey::Terminal(identity) if identity == "opencode@term-3"
        ));
        assert!(matches!(
            &agents[0].session_timing_key,
            Some(AgentTimingKey::Session(session)) if session.value == "ses_timer"
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
