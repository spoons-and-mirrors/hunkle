use super::opencode_tabs::{OpenCodeTabs, OpenCodeUpdate};

pub(crate) struct OpenCodePresence {
    pub(crate) tabs: OpenCodeTabs,
}

impl OpenCodePresence {
    pub(crate) fn new() -> Self {
        Self {
            tabs: OpenCodeTabs::default(),
        }
    }

    pub(crate) fn poll(&mut self) -> OpenCodeUpdate {
        OpenCodeUpdate::default()
    }

    pub(crate) fn instance_id(&self) -> Option<&str> {
        None
    }

    pub(crate) fn focus_tab(&self, _instance_id: &str, _session_id: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "OpenCode tab sync requires Unix",
        ))
    }

    pub(crate) fn rename_tab(
        &self,
        _instance_id: &str,
        _session_id: &str,
        _title: &str,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "OpenCode tab sync requires Unix",
        ))
    }
}
