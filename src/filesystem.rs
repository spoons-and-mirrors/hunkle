use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::repo_path::RepoPath;

mod clipboard_import;
pub(crate) use clipboard_import::operation as clipboard_import_operation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceEntry {
    pub(crate) path: RepoPath,
    pub(crate) is_directory: bool,
}

pub(crate) fn read_workspace_directory(
    root: &Path,
    relative: &RepoPath,
) -> Result<Vec<WorkspaceEntry>> {
    let directory = safe_directory(root, relative)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("could not read directory {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("could not read an entry in {}", directory.display()))?;
        if entry.file_name() == ".git" {
            continue;
        }
        let file_type = entry
            .file_type()
            .with_context(|| format!("could not inspect {}", entry.path().display()))?;
        entries.push(WorkspaceEntry {
            path: relative.join(&entry.file_name()),
            is_directory: file_type.is_dir() && !file_type.is_symlink(),
        });
    }
    entries.sort_unstable_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
    });
    Ok(entries)
}

pub(crate) fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
    file.write_all(content)?;
    file.commit()
}

pub(crate) fn atomic_write_if_unchanged(
    root: &Path,
    relative: &RepoPath,
    expected: &[u8],
    content: &[u8],
) -> Result<()> {
    let path = safe_regular_file(root, relative)?;
    ensure_writable(&path, relative)?;
    let current = fs::read(&path)
        .with_context(|| format!("could not read {} before saving", path.display()))?;
    if current != expected {
        bail!(
            "{} changed on disk; your edits were not saved",
            relative.display()
        );
    }

    let mut file = atomic_write_file::AtomicWriteFile::open(&path)
        .with_context(|| format!("could not prepare {} for saving", path.display()))?;
    file.write_all(content)
        .with_context(|| format!("could not write {}", path.display()))?;

    let checked_path = safe_regular_file(root, relative)?;
    ensure_writable(&checked_path, relative)?;
    let current = fs::read(&checked_path)
        .with_context(|| format!("could not verify {} before saving", path.display()))?;
    if current != expected {
        bail!(
            "{} changed on disk; your edits were not saved",
            relative.display()
        );
    }
    file.commit()
        .with_context(|| format!("could not save {}", path.display()))
}

fn ensure_writable(path: &Path, relative: &RepoPath) -> Result<()> {
    if path
        .metadata()
        .with_context(|| format!("could not inspect {}", path.display()))?
        .permissions()
        .readonly()
    {
        bail!("{} is read-only", relative.display());
    }
    Ok(())
}

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

#[derive(Debug, Clone)]
pub(crate) enum FileOperation {
    CreateFile {
        path: RepoPath,
    },
    CreateDirectory {
        path: RepoPath,
    },
    Rename {
        from: RepoPath,
        to: RepoPath,
    },
    Move {
        from: RepoPath,
        to: RepoPath,
    },
    Delete {
        path: RepoPath,
    },
    ImportClipboard {
        source: PathBuf,
        destination: RepoPath,
        selection: RepoPath,
    },
}

impl FileOperation {
    pub(crate) fn selection_after(&self) -> Option<RepoPath> {
        match self {
            Self::CreateFile { path } | Self::CreateDirectory { path } => Some(path.clone()),
            Self::Rename { to, .. } | Self::Move { to, .. } => Some(to.clone()),
            Self::Delete { .. } => None,
            Self::ImportClipboard { selection, .. } => Some(selection.clone()),
        }
    }

    pub(crate) fn success_message(&self) -> String {
        match self {
            Self::CreateFile { path } => format!("Created {path}"),
            Self::CreateDirectory { path } => format!("Created {path}/"),
            Self::Rename { to, .. } => format!("Renamed to {to}"),
            Self::Move { to, .. } => format!("Moved to {to}"),
            Self::Delete { path } => format!("Deleted {path}"),
            Self::ImportClipboard { destination, .. } if destination.is_empty() => {
                "Pasted files into the workspace".to_owned()
            }
            Self::ImportClipboard { destination, .. } => {
                format!("Pasted files into {destination}/")
            }
        }
    }
}

pub(crate) fn perform(root: &Path, operation: &FileOperation) -> Result<()> {
    match operation {
        FileOperation::CreateFile { path } => {
            let path = safe_path(root, path)?;
            ensure_parent_directory(&path)?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("could not create {}", path.display()))?;
        }
        FileOperation::CreateDirectory { path } => {
            let path = safe_path(root, path)?;
            ensure_parent_directory(&path)?;
            fs::create_dir(&path)
                .with_context(|| format!("could not create directory {}", path.display()))?;
        }
        FileOperation::Rename { from, to } | FileOperation::Move { from, to } => {
            let from_path = safe_path(root, from)?;
            let to_path = safe_path(root, to)?;
            if from_path == to_path {
                return Ok(());
            }
            let metadata = fs::symlink_metadata(&from_path)
                .with_context(|| format!("could not inspect {}", from_path.display()))?;
            if metadata.is_dir() && fs::symlink_metadata(from_path.join(".git")).is_ok() {
                bail!("moving a nested Git repository or submodule is not supported");
            }
            if fs::symlink_metadata(&to_path).is_ok() {
                bail!("{} already exists", to_path.display());
            }
            if metadata.is_dir() && to_path.starts_with(&from_path) {
                bail!("cannot move a directory into itself");
            }
            ensure_parent_directory(&to_path)?;
            fs::rename(&from_path, &to_path).with_context(|| {
                format!(
                    "could not move {} to {}",
                    from_path.display(),
                    to_path.display()
                )
            })?;
        }
        FileOperation::Delete { path } => {
            let path = safe_path(root, path)?;
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("could not inspect {}", path.display()))?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                if contains_git_repository(&path)? {
                    bail!("deleting a nested Git repository or submodule is not supported");
                }
                fs::remove_dir_all(&path)
                    .with_context(|| format!("could not delete directory {}", path.display()))?;
            } else {
                fs::remove_file(&path)
                    .with_context(|| format!("could not delete file {}", path.display()))?;
            }
        }
        FileOperation::ImportClipboard {
            source,
            destination,
            ..
        } => clipboard_import::perform(root, source, destination)?,
    }
    Ok(())
}

fn contains_git_repository(root: &Path) -> Result<bool> {
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("could not inspect {}", directory.display()))?
        {
            let entry =
                entry.with_context(|| format!("could not inspect {}", directory.display()))?;
            let file_type = entry
                .file_type()
                .with_context(|| format!("could not inspect {}", entry.path().display()))?;
            if entry.file_name() == ".git" {
                return Ok(true);
            }
            if file_type.is_dir() && !file_type.is_symlink() {
                directories.push(entry.path());
            }
        }
    }
    Ok(false)
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Name cannot be empty");
    }
    if name == "." || name == ".." || name == ".git" {
        bail!("That name is not allowed");
    }
    if name.contains(['/', '\\']) {
        bail!("Enter a name, not a path");
    }
    if Path::new(name).components().count() != 1 {
        bail!("That name is not valid");
    }
    Ok(())
}

pub(crate) fn safe_regular_file(root: &Path, relative: &RepoPath) -> Result<PathBuf> {
    let path = safe_path(root, relative)?;
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("{} is not a regular file", path.display());
    }
    Ok(path)
}

fn safe_path(root: &Path, relative: &RepoPath) -> Result<PathBuf> {
    let root_metadata = fs::symlink_metadata(root)
        .with_context(|| format!("could not inspect workspace {}", root.display()))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        bail!("the workspace root is no longer a safe directory");
    }
    let path = relative.as_path();
    if relative.is_empty() || path.is_absolute() {
        bail!("Invalid workspace path");
    }
    let components: Vec<_> = path.components().collect();
    for component in &components {
        match component {
            Component::Normal(value) if *value != ".git" => {}
            _ => bail!("Path must stay inside the workspace"),
        }
    }
    let mut ancestor = root.to_owned();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        let Component::Normal(value) = component else {
            unreachable!("components were validated above")
        };
        ancestor.push(value);
        let metadata = fs::symlink_metadata(&ancestor)
            .with_context(|| format!("could not inspect {}", ancestor.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("{} is not a safe workspace directory", ancestor.display());
        }
    }
    Ok(root.join(path))
}

fn safe_directory(root: &Path, relative: &RepoPath) -> Result<PathBuf> {
    let root_metadata = fs::symlink_metadata(root)
        .with_context(|| format!("could not inspect workspace {}", root.display()))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        bail!("the workspace root is no longer a safe directory");
    }
    if relative.is_empty() {
        return Ok(root.to_owned());
    }
    let path = relative.as_path();
    if path.is_absolute() {
        bail!("Invalid workspace path");
    }
    let mut directory = root.to_owned();
    for component in path.components() {
        let Component::Normal(value) = component else {
            bail!("Path must stay inside the workspace");
        };
        if value == ".git" {
            bail!("Path must stay inside the workspace");
        }
        directory.push(value);
        let metadata = fs::symlink_metadata(&directory)
            .with_context(|| format!("could not inspect {}", directory.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("{} is not a safe workspace directory", directory.display());
        }
    }
    Ok(directory)
}

fn ensure_parent_directory(path: &Path) -> Result<()> {
    let parent = path.parent().context("path has no parent directory")?;
    let metadata = fs::symlink_metadata(parent)
        .with_context(|| format!("could not inspect {}", parent.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("{} is not a directory", parent.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
