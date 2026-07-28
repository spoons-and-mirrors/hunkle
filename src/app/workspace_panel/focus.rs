use super::HerdrWorkspace;

pub(super) struct PendingWorkspaceFocus {
    pub(super) request_id: u64,
    workspace_id: String,
}

#[derive(Default)]
pub(super) struct WorkspaceFocusState {
    host_workspace_id: Option<String>,
    observed_workspace_id: Option<String>,
    pub(super) pending: Option<PendingWorkspaceFocus>,
    next_request_id: u64,
}

pub(super) enum WorkspaceFocusCompletion {
    Ignored,
    Succeeded,
    Failed(String),
}

impl WorkspaceFocusState {
    pub(super) fn set_host(&mut self, workspace_id: Option<String>) {
        self.host_workspace_id = workspace_id;
    }

    pub(super) fn host(&self) -> Option<&str> {
        self.host_workspace_id.as_deref()
    }

    pub(super) fn apply_snapshot(&mut self, workspaces: &[HerdrWorkspace]) {
        self.observed_workspace_id = workspaces
            .iter()
            .find(|workspace| workspace.focused)
            .map(|workspace| workspace.id.clone());
        if self.pending.as_ref().is_some_and(|pending| {
            !workspaces
                .iter()
                .any(|workspace| workspace.id == pending.workspace_id)
        }) {
            self.pending = None;
        }
    }

    pub(super) fn begin(&mut self, workspace_id: String) -> u64 {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;
        self.pending = Some(PendingWorkspaceFocus {
            request_id,
            workspace_id,
        });
        request_id
    }

    pub(super) fn complete(
        &mut self,
        request_id: u64,
        result: Result<(), String>,
    ) -> WorkspaceFocusCompletion {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return WorkspaceFocusCompletion::Ignored;
        }
        self.pending = None;
        match result {
            Ok(()) => WorkspaceFocusCompletion::Succeeded,
            Err(error) => WorkspaceFocusCompletion::Failed(error),
        }
    }

    pub(super) fn active_workspace_id(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .map(|pending| pending.workspace_id.as_str())
            .or(self.observed_workspace_id.as_deref())
            .or(self.host_workspace_id.as_deref())
    }
}
