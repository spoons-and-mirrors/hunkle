use std::path::PathBuf;

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

    pub(crate) fn take_workspace_changes(&mut self) -> Vec<NormWorkspaceChange> {
        Vec::new()
    }

    pub(crate) fn open_tab(&mut self, _workspace: PathBuf) -> Result<(), String> {
        Err("Norm tab creation is unavailable on this platform".into())
    }

    pub(crate) fn take_open_tab_completion(&mut self) -> Option<Result<(), String>> {
        None
    }

    pub(crate) fn shutdown(&mut self) {}
}

pub(crate) struct NormWorkspaceChange {
    pub(crate) instance_id: String,
    pub(crate) workspace: PathBuf,
}
