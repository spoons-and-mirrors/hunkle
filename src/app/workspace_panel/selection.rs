use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspacePanelRow {
    Header,
    Group(usize),
    Workspace(usize),
    Spacer,
    AgentHeader,
    Agent(usize),
    AgentSession(usize),
    EmptyAgents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceDropTarget {
    Group(usize),
    Ungrouped,
}

pub(super) fn append_agent_cards(rows: &mut Vec<WorkspacePanelRow>, agents: impl IntoIterator<Item = usize>) {
    for (position, index) in agents.into_iter().enumerate() {
        if position > 0 {
            rows.push(WorkspacePanelRow::Spacer);
        }
        rows.push(WorkspacePanelRow::Agent(index));
        rows.push(WorkspacePanelRow::AgentSession(index));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorkspaceDrag {
    pub(super)workspace: usize,
    pub(super)active: bool,
    pub(super)target: Option<WorkspaceDropTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SelectionKey {
    Workspace(String),
    Agent(String),
}

pub(super) enum Completion {
    Snapshot {
        result: Result<(Vec<HerdrWorkspace>, Vec<HerdrAgent>), String>,
        observed_at_ms: u64,
    },
    HerdrEvent {
        event: herdr::Event,
        observed_at_ms: u64,
    },
    WorkspaceFocus {
        request_id: u64,
        result: Result<(), String>,
    },
    Action {
        result: Result<(), String>,
        reopen_path: Option<PathBuf>,
        warning: Option<String>,
        destructive: bool,
    },
    SnapshotRecall {
        name: String,
        result: Result<SnapshotRecallResult, String>,
    },
}
