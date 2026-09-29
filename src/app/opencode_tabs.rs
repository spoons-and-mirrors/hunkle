use std::path::PathBuf;

use serde::Deserialize;

/// Display-only data published by the OpenCode CLI. OpenCode owns tab identity and order.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenCodeTab {
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub title: String,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub directory: Option<PathBuf>,
    pub active: bool,
    pub busy: bool,
    pub attention: bool,
    pub unread: Option<String>,
}

#[derive(Default)]
pub(crate) struct OpenCodeTabs {
    pub items: Vec<OpenCodeTab>,
    pub scroll: usize,
    pub reveal_active: bool,
    pub viewport_width: u16,
}

impl OpenCodeTabs {
    pub(super) fn replace(&mut self, items: Vec<OpenCodeTab>) -> bool {
        if self.items == items {
            return false;
        }
        // Status changes must not interrupt a user scrolling through the strip.
        self.reveal_active |= self
            .items
            .iter()
            .map(|tab| (&tab.session_id, tab.active))
            .ne(items.iter().map(|tab| (&tab.session_id, tab.active)));
        self.items = items;
        true
    }
}

#[derive(Default)]
pub(crate) struct OpenCodeUpdate {
    pub changed: bool,
    pub workspace: Option<PathBuf>,
}
