use std::path::PathBuf;

use super::AgentStatus;

pub(crate) struct NormPresence;

impl NormPresence {
    pub(crate) fn new() -> Self {
        Self
    }

    #[cfg(test)]
    pub(crate) fn disabled_for_test(self) -> Self {
        self
    }

    pub(crate) fn poll(&mut self) -> bool {
        false
    }

    pub(crate) fn is_available(&self) -> bool {
        false
    }

    pub(crate) fn agents(&self) -> &[NormAgent] {
        &[]
    }

    pub(crate) fn scroll(&self) -> usize {
        0
    }

    pub(crate) fn scroll_agents(&mut self, _delta: isize) {}
}

pub(crate) struct NormAgent {
    pub(crate) identity: NormAgentIdentity,
    pub(crate) workspace: PathBuf,
    pub(crate) lifecycle: NormLifecycle,
    pub(crate) activity: NormActivity,
    pub(crate) session_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) open_views: u32,
}

impl NormAgent {
    pub(crate) fn status(&self) -> AgentStatus {
        match (self.lifecycle, self.activity) {
            (NormLifecycle::Terminal, _) => AgentStatus::Done,
            (NormLifecycle::Starting, _) => AgentStatus::Unknown,
            (NormLifecycle::Running, NormActivity::Idle) => AgentStatus::Idle,
            (NormLifecycle::Running, NormActivity::Working) => AgentStatus::Working,
            (NormLifecycle::Running, NormActivity::Blocked) => AgentStatus::Blocked,
            (NormLifecycle::Running, NormActivity::Unknown) => AgentStatus::Unknown,
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match (self.lifecycle, self.activity) {
            (NormLifecycle::Terminal, _) => "terminal",
            (NormLifecycle::Starting, _) => "starting",
            (NormLifecycle::Running, NormActivity::Idle) => "idle",
            (NormLifecycle::Running, NormActivity::Working) => "working",
            (NormLifecycle::Running, NormActivity::Blocked) => "blocked",
            (NormLifecycle::Running, NormActivity::Unknown) => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NormAgentIdentity {
    pub(crate) daemon_epoch: String,
    pub(crate) id: u64,
    pub(crate) generation: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum NormLifecycle {
    Starting,
    Running,
    Terminal,
}

#[derive(Clone, Copy)]
pub(crate) enum NormActivity {
    Idle,
    Working,
    Blocked,
    Unknown,
}
