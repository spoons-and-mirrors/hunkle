use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceRenameTarget {
    Workspace { workspace_id: String },
    Agent { identity: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceDeleteKind {
    Workspace {
        pane_count: usize,
    },
    Worktree {
        path: Option<PathBuf>,
        parent_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceDeleteDialog {
    pub(crate) workspace_id: String,
    pub(crate) label: String,
    pub(crate) kind: WorkspaceDeleteKind,
}

pub(crate) struct WorkspaceRenameDialog {
    pub(crate) target: WorkspaceRenameTarget,
    pub(crate) original_label: String,
    pub(crate) input: TextInput,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotLoadDialog {
    pub(super) snapshot: WorkspaceSnapshot,
    pub(crate) name: String,
    pub(crate) open_count: usize,
    pub(crate) close_count: usize,
    pub(crate) close_pane_count: usize,
    pub(crate) group_count: usize,
}
