use std::{
    cell::Cell,
    fs,
    io::ErrorKind,
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
    groups_writable: Cell<bool>,
    snapshots_writable: Cell<bool>,
}

impl PresetStore {
    pub(super) fn new(groups_path: Option<PathBuf>, snapshots_path: Option<PathBuf>) -> Self {
        Self {
            groups_path,
            snapshots_path,
            groups_writable: Cell::new(true),
            snapshots_writable: Cell::new(true),
        }
    }

    pub(super) fn load(&self) -> (Vec<WorkspaceGroup>, Vec<WorkspaceSnapshot>, Option<String>) {
        let groups = self
            .groups_path
            .as_deref()
            .map(load_groups)
            .unwrap_or_else(|| Ok(Vec::new()));
        let snapshots = self
            .snapshots_path
            .as_deref()
            .map(load_snapshots)
            .unwrap_or_else(|| Ok(Vec::new()));
        let mut errors = Vec::new();
        let groups = groups.unwrap_or_else(|error| {
            self.groups_writable.set(false);
            errors.push(error);
            Vec::new()
        });
        let snapshots = snapshots.unwrap_or_else(|error| {
            self.snapshots_writable.set(false);
            errors.push(error);
            Vec::new()
        });
        (
            groups,
            snapshots,
            (!errors.is_empty()).then(|| errors.join("; ")),
        )
    }

    pub(super) fn save_groups(&self, groups: &[WorkspaceGroup]) -> Result<(), String> {
        if !self.groups_writable.get() {
            return Err(
                "Could not save workspace groups: repair or remove the malformed preset file first"
                    .to_owned(),
            );
        }
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
        if !self.snapshots_writable.get() {
            return Err("Could not save workspace snapshots: repair or remove the malformed preset file first".to_owned());
        }
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

fn read_preset(path: &Path, label: &str) -> Result<Option<Value>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read {label}: {error}")),
    };
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("Could not parse {label}: {error}"))
}

fn load_groups(path: &Path) -> Result<Vec<WorkspaceGroup>, String> {
    let Some(value) = read_preset(path, "workspace groups")? else {
        return Ok(Vec::new());
    };
    value
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| "Could not parse workspace groups: missing groups array".to_owned())?
        .iter()
        .enumerate()
        .map(|(index, group)| {
            let workspace_ids = match group.get("workspace_ids") {
                None => Vec::new(),
                Some(ids) => ids
                    .as_array()
                    .ok_or_else(|| format!("Workspace group {index} has malformed workspace IDs"))?
                    .iter()
                    .enumerate()
                    .map(|(workspace_index, id)| {
                        id.as_str().map(str::to_owned).ok_or_else(|| {
                            format!("Workspace group {index} ID {workspace_index} is malformed")
                        })
                    })
                    .collect::<Result<_, _>>()?,
            };
            Ok(WorkspaceGroup {
                name: group
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("Workspace group {index} has no valid name"))?
                    .to_owned(),
                expanded: group
                    .get("expanded")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                workspace_ids,
            })
        })
        .collect()
}

fn load_snapshots(path: &Path) -> Result<Vec<WorkspaceSnapshot>, String> {
    let Some(value) = read_preset(path, "workspace snapshots")? else {
        return Ok(Vec::new());
    };
    let mut snapshots = value
        .get("snapshots")
        .and_then(Value::as_array)
        .ok_or_else(|| "Could not parse workspace snapshots: missing snapshots array".to_owned())?
        .iter()
        .enumerate()
        .map(|(snapshot_index, snapshot)| {
            let name = snapshot
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Workspace snapshot {snapshot_index} has no valid name"))?
                .to_owned();
            let entries = snapshot
                .get("workspaces")
                .ok_or_else(|| {
                    format!("Workspace snapshot {snapshot_index} has no workspaces")
                })?
                .as_array()
                .ok_or_else(|| format!("Workspace snapshot {snapshot_index} has malformed workspaces"))?
                .iter()
                .enumerate()
                .map(|(entry_index, entry)| {
                    Ok(WorkspaceSnapshotEntry {
                        label: entry
                            .get("label")
                            .and_then(Value::as_str)
                            .ok_or_else(|| format!("Workspace snapshot {snapshot_index} entry {entry_index} has no valid label"))?
                            .to_owned(),
                        path: PathBuf::from(
                            entry
                                .get("path")
                                .and_then(Value::as_str)
                                .ok_or_else(|| format!("Workspace snapshot {snapshot_index} entry {entry_index} has no valid path"))?,
                        ),
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
                .collect::<Result<Vec<_>, String>>()?;
            if entries.is_empty() {
                return Err(format!("Workspace snapshot {snapshot_index} has no workspaces"));
            }
            let groups_value = snapshot.get("groups");
            let groups = match groups_value {
                None => Vec::new(),
                Some(groups) => groups
                    .as_array()
                    .ok_or_else(|| format!("Workspace snapshot {snapshot_index} has malformed groups"))?
                    .iter()
                    .enumerate()
                    .map(|(group_index, group)| {
                        Ok(WorkspaceSnapshotGroup {
                            name: group
                                .get("name")
                                .and_then(Value::as_str)
                                .ok_or_else(|| format!("Workspace snapshot {snapshot_index} group {group_index} has no valid name"))?
                                .to_owned(),
                        expanded: group
                            .get("expanded")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        })
                    })
                    .collect::<Result<_, String>>()?,
            };
            Ok(WorkspaceSnapshot {
                name,
                entries,
                groups,
                groups_captured: groups_value.is_some(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    snapshots.sort_by_cached_key(|snapshot| snapshot.name.to_lowercase());
    Ok(snapshots)
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
