use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::repo_path::RepoPath;

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

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

#[derive(Debug, Clone)]
pub(crate) enum FileOperation {
    CreateFile { path: RepoPath },
    CreateDirectory { path: RepoPath },
    Rename { from: RepoPath, to: RepoPath },
    Move { from: RepoPath, to: RepoPath },
    Delete { path: RepoPath },
}

impl FileOperation {
    pub(crate) fn selection_after(&self) -> Option<RepoPath> {
        match self {
            Self::CreateFile { path } | Self::CreateDirectory { path } => Some(path.clone()),
            Self::Rename { to, .. } | Self::Move { to, .. } => Some(to.clone()),
            Self::Delete { .. } => None,
        }
    }

    pub(crate) fn success_message(&self) -> String {
        match self {
            Self::CreateFile { path } => format!("Created {path}"),
            Self::CreateDirectory { path } => format!("Created {path}/"),
            Self::Rename { to, .. } => format!("Renamed to {to}"),
            Self::Move { to, .. } => format!("Moved to {to}"),
            Self::Delete { path } => format!("Deleted {path}"),
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
mod tests {
    use super::*;

    #[test]
    fn atomically_replaces_file_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state");
        fs::write(&path, "old").unwrap();

        atomic_write(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn path_identity_uses_lexical_and_canonical_equality() {
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child");
        fs::create_dir(&child).unwrap();

        assert!(same_path(&child, &child));
        assert!(same_path(&child, &directory.path().join("./child")));
        assert!(!same_path(&child, &directory.path().join("missing")));
    }

    #[test]
    fn reads_only_immediate_workspace_children() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::create_dir(root.join("empty")).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join("README.md"), "readme").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let root_entries = read_workspace_directory(root, &RepoPath::default()).unwrap();
        assert_eq!(
            root_entries,
            vec![
                WorkspaceEntry {
                    path: "empty".into(),
                    is_directory: true,
                },
                WorkspaceEntry {
                    path: "src".into(),
                    is_directory: true,
                },
                WorkspaceEntry {
                    path: "README.md".into(),
                    is_directory: false,
                },
            ]
        );
        assert_eq!(
            read_workspace_directory(root, &"src".into()).unwrap(),
            vec![
                WorkspaceEntry {
                    path: "src/nested".into(),
                    is_directory: true,
                },
                WorkspaceEntry {
                    path: "src/main.rs".into(),
                    is_directory: false,
                },
            ]
        );
    }

    #[test]
    fn creates_moves_renames_and_deletes_workspace_entries() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();

        perform(
            root,
            &FileOperation::CreateDirectory {
                path: "docs".into(),
            },
        )
        .unwrap();
        perform(
            root,
            &FileOperation::CreateFile {
                path: "readme.md".into(),
            },
        )
        .unwrap();
        perform(
            root,
            &FileOperation::Move {
                from: "readme.md".into(),
                to: "docs/readme.md".into(),
            },
        )
        .unwrap();
        perform(
            root,
            &FileOperation::Rename {
                from: "docs/readme.md".into(),
                to: "docs/guide.md".into(),
            },
        )
        .unwrap();
        assert!(root.join("docs/guide.md").is_file());

        perform(
            root,
            &FileOperation::Delete {
                path: "docs".into(),
            },
        )
        .unwrap();
        assert!(!root.join("docs").exists());
    }

    #[test]
    fn rejects_traversal_overwrites_and_descendant_moves() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir(root.join("source")).unwrap();
        fs::write(root.join("source/file"), "content").unwrap();
        fs::write(root.join("existing"), "content").unwrap();
        fs::create_dir_all(root.join("nested-repository/.git")).unwrap();

        assert!(
            perform(
                root,
                &FileOperation::Delete {
                    path: "../outside".into()
                }
            )
            .is_err()
        );
        assert!(
            perform(
                root,
                &FileOperation::Move {
                    from: "nested-repository".into(),
                    to: "moved-repository".into(),
                },
            )
            .is_err()
        );
        assert!(
            perform(
                root,
                &FileOperation::Delete {
                    path: ".git/config".into()
                }
            )
            .is_err()
        );
        assert!(
            perform(
                root,
                &FileOperation::Rename {
                    from: "source/file".into(),
                    to: "existing".into(),
                },
            )
            .is_err()
        );
        assert!(
            perform(
                root,
                &FileOperation::Move {
                    from: "source".into(),
                    to: "source/nested/source".into(),
                },
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_to_delete_nested_repositories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("parent/nested/.git")).unwrap();
        fs::write(root.join("parent/file"), "content").unwrap();

        let error = perform(
            root,
            &FileOperation::Delete {
                path: "parent".into(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("nested Git repository"));
        assert!(root.join("parent/file").is_file());
    }

    #[test]
    fn validates_regular_files_inside_the_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        assert_eq!(
            safe_regular_file(root, &RepoPath::from("src/main.rs")).unwrap(),
            root.join("src/main.rs")
        );
        assert!(safe_regular_file(root, &RepoPath::from("../outside")).is_err());
        assert!(safe_regular_file(root, &RepoPath::from("src")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn deletes_a_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("target/keep"), "content").unwrap();
        symlink(root.join("target"), root.join("link")).unwrap();

        perform(
            root,
            &FileOperation::Delete {
                path: "link".into(),
            },
        )
        .unwrap();
        assert!(root.join("target/keep").exists());
        assert!(!root.join("link").exists());
    }

    #[cfg(unix)]
    #[test]
    fn file_operations_preserve_invalid_utf8_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let first = OsString::from_vec(b"entry-\x80".to_vec());
        let second = OsString::from_vec(b"entry-\x81".to_vec());
        let renamed = OsString::from_vec(b"renamed-\x80".to_vec());
        fs::write(root.join(&first), "first").unwrap();
        fs::write(root.join(&second), "second").unwrap();

        perform(
            root,
            &FileOperation::Rename {
                from: RepoPath::from(PathBuf::from(&first)),
                to: RepoPath::from(PathBuf::from(&renamed)),
            },
        )
        .unwrap();

        assert!(!root.join(&first).exists());
        assert_eq!(fs::read(root.join(&renamed)).unwrap(), b"first");
        assert_eq!(fs::read(root.join(&second)).unwrap(), b"second");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_operations_through_a_symlinked_ancestor() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep"), "content").unwrap();
        symlink(outside.path(), workspace.path().join("link")).unwrap();

        assert!(
            perform(
                workspace.path(),
                &FileOperation::Delete {
                    path: "link/keep".into(),
                },
            )
            .is_err()
        );
        assert!(
            perform(
                workspace.path(),
                &FileOperation::CreateFile {
                    path: "link/new".into(),
                },
            )
            .is_err()
        );
        assert!(outside.path().join("keep").exists());
        assert!(!outside.path().join("new").exists());

        let container = tempfile::tempdir().unwrap();
        let root = container.path().join("workspace");
        let original = container.path().join("original");
        fs::create_dir(&root).unwrap();
        fs::rename(&root, &original).unwrap();
        symlink(outside.path(), &root).unwrap();
        assert!(
            perform(
                &root,
                &FileOperation::Delete {
                    path: "keep".into(),
                },
            )
            .is_err()
        );
        assert!(outside.path().join("keep").exists());

        assert!(safe_regular_file(workspace.path(), &RepoPath::from("link/keep")).is_err());
    }
}
