use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceGroup {
    pub(crate) name: String,
    pub(crate) expanded: bool,
    pub(super) workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) name: String,
    pub(super) entries: Vec<WorkspaceSnapshotEntry>,
    pub(super) groups: Vec<WorkspaceSnapshotGroup>,
    pub(super) groups_captured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceSnapshotEntry {
    pub(super) label: String,
    pub(super) path: PathBuf,
    pub(super) focused: bool,
    pub(super) linked_worktree: bool,
    pub(super) group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceSnapshotGroup {
    pub(super) name: String,
    pub(super) expanded: bool,
}

pub(super) struct SnapshotRecallResult {
    pub(super) groups: Vec<WorkspaceGroup>,
}

pub(super) fn recall_snapshot(
    snapshot: &WorkspaceSnapshot,
    current: &[HerdrWorkspace],
) -> Result<SnapshotRecallResult, String> {
    let matches = presets::matching_indices(snapshot, current);
    let mut used = vec![false; current.len()];
    let mut target_ids = Vec::with_capacity(snapshot.entries.len());
    let mut renames = Vec::new();
    for (entry, matched) in snapshot.entries.iter().zip(matches) {
        if let Some(index) = matched {
            let workspace = &current[index];
            used[index] = true;
            if workspace.label != entry.label {
                renames.push((workspace.id.clone(), entry.label.clone()));
            }
            target_ids.push(workspace.id.clone());
            continue;
        }

        let id = herdr::restore(herdr::RestoreRequest {
            path: entry.path.clone(),
            label: entry.label.clone(),
            linked_worktree: entry.linked_worktree,
        })
        .map_err(|error| {
            format!(
                "Could not load preset '{}' completely: {error}",
                snapshot.name
            )
        })?
        .ok_or_else(|| {
            format!(
                "Could not identify '{}' while loading preset '{}'",
                entry.label, snapshot.name
            )
        })?;
        target_ids.push(id);
    }

    for (workspace_id, label) in renames {
        herdr::perform(herdr::Action::RenameWorkspace {
            workspace_id,
            label,
        })?;
    }

    let focus_index = snapshot
        .entries
        .iter()
        .position(|entry| entry.focused)
        .unwrap_or(0);
    herdr::perform(herdr::Action::FocusWorkspace {
        workspace_id: target_ids[focus_index].clone(),
    })?;

    let mut extras = current
        .iter()
        .enumerate()
        .filter(|(index, _)| !used[*index])
        .map(|(_, workspace)| workspace)
        .collect::<Vec<_>>();
    extras.sort_by_key(|workspace| workspace.focused);
    for workspace in extras {
        herdr::perform(herdr::Action::CloseWorkspace {
            workspace_id: workspace.id.clone(),
        })?;
    }

    let groups = presets::groups_after_recall(snapshot, &target_ids);
    Ok(SnapshotRecallResult { groups })
}

pub(super) fn populate_workspace_branches(workspaces: &mut [HerdrWorkspace]) {
    for workspace in workspaces {
        workspace.branch = workspace.path.as_deref().and_then(workspace_branch);
    }
}

pub(super) fn workspace_branch(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let mut directory = if path.is_dir() { path } else { path.parent()? };
    loop {
        let dot_git = directory.join(".git");
        if dot_git.is_dir() {
            return branch_from_head(&dot_git.join("HEAD"));
        }
        if dot_git.is_file() {
            let git_file = fs::read_to_string(&dot_git).ok()?;
            let git_dir = git_file.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = Path::new(git_dir);
            let git_dir = if git_dir.is_absolute() {
                git_dir.to_path_buf()
            } else {
                directory.join(git_dir)
            };
            return branch_from_head(&git_dir.join("HEAD"));
        }
        directory = directory.parent()?;
    }
}

pub(super) fn branch_from_head(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .trim()
        .strip_prefix("ref: refs/heads/")
        .filter(|branch| !branch.is_empty())
        .map(str::to_owned)
}
