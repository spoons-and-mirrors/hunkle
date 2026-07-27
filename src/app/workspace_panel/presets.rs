use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::super::atomic_write;
use super::{
    HerdrWorkspace, WorkspaceGroup, WorkspaceSnapshot, WorkspaceSnapshotEntry,
    WorkspaceSnapshotGroup,
};

pub(super) struct PresetStore {
    groups_path: Option<PathBuf>,
    snapshots_path: Option<PathBuf>,
}

impl PresetStore {
    pub(super) fn new(groups_path: Option<PathBuf>, snapshots_path: Option<PathBuf>) -> Self {
        Self {
            groups_path,
            snapshots_path,
        }
    }

    pub(super) fn load(&self) -> (Vec<WorkspaceGroup>, Vec<WorkspaceSnapshot>) {
        let groups = self
            .groups_path
            .as_deref()
            .map(load_groups)
            .unwrap_or_default();
        let snapshots = self
            .snapshots_path
            .as_deref()
            .map(load_snapshots)
            .unwrap_or_default();
        (groups, snapshots)
    }

    pub(super) fn save_groups(&self, groups: &[WorkspaceGroup]) -> Result<(), String> {
        let Some(path) = self.groups_path.as_deref() else {
            return Ok(());
        };
        prepare_parent(path)?;
        let groups = groups
            .iter()
            .map(|group| {
                serde_json::json!({
                    "name": group.name,
                    "expanded": group.expanded,
                    "workspace_ids": group.workspace_ids,
                })
            })
            .collect::<Vec<_>>();
        let content = serde_json::to_string_pretty(&serde_json::json!({ "groups": groups }))
            .map_err(|error| format!("Could not serialize workspace groups: {error}"))?;
        atomic_write(path, format!("{content}\n").as_bytes())
            .map_err(|error| format!("Could not save workspace groups: {error}"))
    }

    pub(super) fn save_snapshots(&self, snapshots: &[WorkspaceSnapshot]) -> Result<(), String> {
        let Some(path) = self.snapshots_path.as_deref() else {
            return Ok(());
        };
        prepare_parent(path)?;
        let snapshots = snapshots
            .iter()
            .map(|snapshot| {
                let entries = snapshot
                    .entries
                    .iter()
                    .map(|entry| {
                        serde_json::json!({
                            "label": entry.label,
                            "path": entry.path,
                            "focused": entry.focused,
                            "linked_worktree": entry.linked_worktree,
                            "group": entry.group,
                        })
                    })
                    .collect::<Vec<_>>();
                let groups = snapshot
                    .groups
                    .iter()
                    .map(|group| {
                        serde_json::json!({
                            "name": group.name,
                            "expanded": group.expanded,
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "name": snapshot.name,
                    "workspaces": entries,
                    "groups": groups,
                })
            })
            .collect::<Vec<_>>();
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "version": 2,
            "snapshots": snapshots,
        }))
        .map_err(|error| format!("Could not serialize workspace snapshots: {error}"))?;
        atomic_write(path, format!("{content}\n").as_bytes())
            .map_err(|error| format!("Could not save workspace snapshots: {error}"))
    }
}

fn prepare_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Hunkle config directory: {error}"))?;
    }
    Ok(())
}

fn load_groups(path: &Path) -> Vec<WorkspaceGroup> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };
    value
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| {
            Some(WorkspaceGroup {
                name: group.get("name")?.as_str()?.to_owned(),
                expanded: group
                    .get("expanded")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                workspace_ids: group
                    .get("workspace_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect()
}

fn load_snapshots(path: &Path) -> Vec<WorkspaceSnapshot> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };
    let mut snapshots = value
        .get("snapshots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|snapshot| {
            let name = snapshot.get("name")?.as_str()?.to_owned();
            let entries = snapshot
                .get("workspaces")?
                .as_array()?
                .iter()
                .filter_map(|entry| {
                    Some(WorkspaceSnapshotEntry {
                        label: entry.get("label")?.as_str()?.to_owned(),
                        path: PathBuf::from(entry.get("path")?.as_str()?),
                        focused: entry
                            .get("focused")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        linked_worktree: entry
                            .get("linked_worktree")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        group: entry
                            .get("group")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect::<Vec<_>>();
            let groups = snapshot
                .get("groups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|group| {
                    Some(WorkspaceSnapshotGroup {
                        name: group.get("name")?.as_str()?.to_owned(),
                        expanded: group
                            .get("expanded")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                    })
                })
                .collect();
            (!entries.is_empty()).then_some(WorkspaceSnapshot {
                name,
                entries,
                groups,
                groups_captured: snapshot.get("groups").is_some(),
            })
        })
        .collect::<Vec<_>>();
    snapshots.sort_by_cached_key(|snapshot| snapshot.name.to_lowercase());
    snapshots
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RecallPlan {
    pub(super) open_count: usize,
    pub(super) close_count: usize,
    pub(super) close_pane_count: usize,
}

pub(super) fn recall_plan(snapshot: &WorkspaceSnapshot, current: &[HerdrWorkspace]) -> RecallPlan {
    let matches = matching_indices(snapshot, current);
    let mut used = vec![false; current.len()];
    for index in matches.iter().flatten() {
        used[*index] = true;
    }
    RecallPlan {
        open_count: matches.iter().filter(|index| index.is_none()).count(),
        close_count: used.iter().filter(|used| !**used).count(),
        close_pane_count: current
            .iter()
            .zip(used)
            .filter(|(_, used)| !used)
            .map(|(workspace, _)| workspace.pane_count)
            .sum(),
    }
}

pub(super) fn matching_indices(
    snapshot: &WorkspaceSnapshot,
    current: &[HerdrWorkspace],
) -> Vec<Option<usize>> {
    let mut used = vec![false; current.len()];
    snapshot
        .entries
        .iter()
        .map(|entry| {
            let index = current.iter().enumerate().find_map(|(index, workspace)| {
                (!used[index]
                    && workspace
                        .path
                        .as_deref()
                        .is_some_and(|path| same_path(path, &entry.path)))
                .then_some(index)
            });
            if let Some(index) = index {
                used[index] = true;
            }
            index
        })
        .collect()
}

pub(super) fn groups_after_recall(
    snapshot: &WorkspaceSnapshot,
    target_ids: &[String],
) -> Vec<WorkspaceGroup> {
    let mut groups = snapshot
        .groups
        .iter()
        .map(|group| WorkspaceGroup {
            name: group.name.clone(),
            expanded: group.expanded,
            workspace_ids: Vec::new(),
        })
        .collect::<Vec<_>>();
    for (entry, workspace_id) in snapshot.entries.iter().zip(target_ids) {
        if entry.linked_worktree {
            continue;
        }
        let Some(group_name) = entry.group.as_deref() else {
            continue;
        };
        if let Some(group) = groups.iter_mut().find(|group| group.name == group_name) {
            group.workspace_ids.push(workspace_id.clone());
        }
    }
    groups
}

pub(super) fn sort_groups(groups: &mut [WorkspaceGroup]) {
    groups.sort_by_cached_key(|group| group.name.to_lowercase());
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
}

impl WorkspaceSnapshot {
    pub(crate) fn workspace_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub(super) fn capture_groups(
        &mut self,
        groups: &[WorkspaceGroup],
        workspaces: &[HerdrWorkspace],
    ) {
        self.groups = groups
            .iter()
            .map(|group| WorkspaceSnapshotGroup {
                name: group.name.clone(),
                expanded: group.expanded,
            })
            .collect();
        for entry in &mut self.entries {
            let Some(workspace) = workspaces.iter().find(|workspace| {
                workspace
                    .path
                    .as_deref()
                    .is_some_and(|path| same_path(path, &entry.path))
            }) else {
                continue;
            };
            let workspace_id = workspace
                .parent_workspace_id
                .as_deref()
                .unwrap_or(&workspace.id);
            entry.group = groups
                .iter()
                .find(|group| group.workspace_ids.iter().any(|id| id == workspace_id))
                .map(|group| group.name.clone());
        }
        self.groups_captured = true;
    }
}
