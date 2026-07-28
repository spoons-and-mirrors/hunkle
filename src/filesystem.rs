use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

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
        } => import_clipboard(root, source, destination)?,
    }
    Ok(())
}

const HERDR_CLIPBOARD_STAGING_VERSION: &str = "v1";
const MAX_CLIPBOARD_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CLIPBOARD_FILE_ENTRIES: usize = 512;
const MAX_CLIPBOARD_FILE_ROOTS: usize = 64;
const MAX_CLIPBOARD_FILE_DEPTH: usize = 64;
const MAX_CLIPBOARD_FILE_PATH_BYTES: usize = 256 * 1024;

pub(crate) fn clipboard_import_operation(
    text: &str,
    destination: RepoPath,
) -> Result<Option<FileOperation>> {
    #[cfg(not(unix))]
    {
        let _ = (text, destination);
        Ok(None)
    }
    #[cfg(unix)]
    {
        if text.is_empty() || text.contains(['\r', '\n', '\0']) {
            return Ok(None);
        }
        let source = PathBuf::from(text);
        let staging_root = herdr_clipboard_staging_root();
        if source.parent() != Some(staging_root.as_path()) {
            return Ok(None);
        }
        validate_private_directory(&staging_root, "Herdr clipboard staging root")?;
        validate_private_directory(&source, "Herdr clipboard transfer")?;

        let mut children = read_sorted_directory(&source, MAX_CLIPBOARD_FILE_ROOTS)?;
        if children.is_empty() {
            bail!("Herdr clipboard transfer is empty");
        }
        if children.len() > MAX_CLIPBOARD_FILE_ROOTS {
            bail!("Herdr clipboard transfer has too many top-level entries");
        }
        let first_name = children.remove(0).file_name();
        validate_import_name(&first_name)?;
        let selection = destination.join(&first_name);
        Ok(Some(FileOperation::ImportClipboard {
            source,
            destination,
            selection,
        }))
    }
}

#[cfg(unix)]
fn herdr_clipboard_staging_root() -> PathBuf {
    let user_id = unsafe { libc::geteuid() };
    std::env::temp_dir().join(format!(
        "herdr-clipboard-files-{HERDR_CLIPBOARD_STAGING_VERSION}-{user_id}"
    ))
}

struct ClipboardImportEntry {
    source: PathBuf,
    relative: PathBuf,
    file_bytes: Option<u64>,
    identity: Option<(u64, u64)>,
}

fn import_clipboard(root: &Path, source: &Path, destination: &RepoPath) -> Result<()> {
    #[cfg(unix)]
    {
        let staging_root = herdr_clipboard_staging_root();
        if source.parent() != Some(staging_root.as_path()) {
            bail!("clipboard import is not an owned Herdr transfer");
        }
        validate_private_directory(&staging_root, "Herdr clipboard staging root")?;
        validate_private_directory(source, "Herdr clipboard transfer")?;
    }
    #[cfg(not(unix))]
    bail!("Herdr clipboard file import is only supported on Unix");

    safe_directory(root, destination)?;
    let entries = inspect_clipboard_transfer(source)?;
    let top_level = entries
        .iter()
        .filter(|entry| entry.relative.components().count() == 1)
        .collect::<Vec<_>>();
    if top_level.is_empty() || top_level.len() > MAX_CLIPBOARD_FILE_ROOTS {
        bail!("Herdr clipboard transfer has an invalid number of top-level entries");
    }
    for entry in &top_level {
        let target_relative = destination.join(entry.relative.as_os_str());
        let target = safe_path(root, &target_relative)?;
        if fs::symlink_metadata(&target).is_ok() {
            bail!("{} already exists", target.display());
        }
    }

    let mut created_roots = Vec::new();
    let result = (|| {
        for entry in &entries {
            let current_metadata = fs::symlink_metadata(&entry.source).with_context(|| {
                format!(
                    "could not inspect clipboard entry {}",
                    entry.source.display()
                )
            })?;
            validate_private_metadata(&current_metadata, &entry.source)?;
            if metadata_identity(&current_metadata) != entry.identity {
                bail!("clipboard entry changed while it was being copied");
            }
            let target_relative = destination.join(entry.relative.as_os_str());
            let target = safe_path(root, &target_relative)?;
            if let Some(file_bytes) = entry.file_bytes {
                copy_regular_file(&entry.source, &target, file_bytes, entry.identity)?;
            } else {
                if !current_metadata.is_dir() || current_metadata.file_type().is_symlink() {
                    bail!("clipboard directory changed while it was being copied");
                }
                fs::create_dir(&target)
                    .with_context(|| format!("could not create directory {}", target.display()))?;
            }
            if entry.relative.components().count() == 1 {
                created_roots.push(target);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        for path in created_roots.into_iter().rev() {
            let _ = remove_imported_path(&path);
        }
        return Err(error);
    }
    Ok(())
}

fn inspect_clipboard_transfer(root: &Path) -> Result<Vec<ClipboardImportEntry>> {
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    let mut total_path_bytes = 0_usize;
    for child in read_sorted_directory(root, MAX_CLIPBOARD_FILE_ROOTS)? {
        let name = child.file_name();
        validate_import_name(&name)?;
        inspect_clipboard_entry(
            &child.path(),
            PathBuf::from(name),
            &mut entries,
            &mut total_bytes,
            &mut total_path_bytes,
        )?;
    }
    if entries.is_empty() {
        bail!("Herdr clipboard transfer is empty");
    }
    Ok(entries)
}

fn inspect_clipboard_entry(
    source: &Path,
    relative: PathBuf,
    entries: &mut Vec<ClipboardImportEntry>,
    total_bytes: &mut u64,
    total_path_bytes: &mut usize,
) -> Result<()> {
    if entries.len() >= MAX_CLIPBOARD_FILE_ENTRIES {
        bail!("Herdr clipboard transfer has too many entries");
    }
    let depth = relative.components().count();
    if depth == 0 || depth > MAX_CLIPBOARD_FILE_DEPTH {
        bail!("Herdr clipboard transfer is nested too deeply");
    }
    let path_bytes = relative
        .to_str()
        .context("Herdr clipboard transfer contains a non-UTF-8 path")?
        .len();
    *total_path_bytes = total_path_bytes
        .checked_add(path_bytes)
        .context("Herdr clipboard paths are too large")?;
    if *total_path_bytes > MAX_CLIPBOARD_FILE_PATH_BYTES {
        bail!("Herdr clipboard paths are too large");
    }

    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("could not inspect clipboard entry {}", source.display()))?;
    validate_private_metadata(&metadata, source)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        entries.push(ClipboardImportEntry {
            source: source.to_owned(),
            relative: relative.clone(),
            file_bytes: None,
            identity: metadata_identity(&metadata),
        });
        let remaining = MAX_CLIPBOARD_FILE_ENTRIES.saturating_sub(entries.len());
        for child in read_sorted_directory(source, remaining)? {
            let name = child.file_name();
            validate_import_name(&name)?;
            inspect_clipboard_entry(
                &child.path(),
                relative.join(name),
                entries,
                total_bytes,
                total_path_bytes,
            )?;
        }
        return Ok(());
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("{} is not a regular clipboard file", source.display());
    }
    *total_bytes = total_bytes
        .checked_add(metadata.len())
        .context("Herdr clipboard files are too large")?;
    if *total_bytes > MAX_CLIPBOARD_FILE_BYTES {
        bail!("Herdr clipboard files are too large");
    }
    entries.push(ClipboardImportEntry {
        source: source.to_owned(),
        relative,
        file_bytes: Some(metadata.len()),
        identity: metadata_identity(&metadata),
    });
    Ok(())
}

fn read_sorted_directory(path: &Path, max_entries: usize) -> Result<Vec<fs::DirEntry>> {
    let mut entries = Vec::with_capacity(max_entries.min(32));
    for entry in fs::read_dir(path)
        .with_context(|| format!("could not read clipboard directory {}", path.display()))?
    {
        if entries.len() >= max_entries {
            bail!("Herdr clipboard transfer has too many entries");
        }
        entries.push(
            entry.with_context(|| {
                format!("could not read clipboard directory {}", path.display())
            })?,
        );
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn validate_import_name(name: &std::ffi::OsStr) -> Result<()> {
    let name = name
        .to_str()
        .context("Herdr clipboard transfer contains a non-UTF-8 file name")?;
    validate_name(name)?;
    if name.eq_ignore_ascii_case(".git") {
        bail!("Herdr clipboard transfer cannot contain .git");
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect {label} {}", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("{label} is not a real directory");
    }
    validate_private_metadata(&metadata, path)
}

fn validate_private_metadata(metadata: &fs::Metadata, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let user_id = unsafe { libc::geteuid() };
        if metadata.uid() != user_id || metadata.mode() & 0o077 != 0 {
            bail!("{} is not private to the current user", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = (metadata, path);
    Ok(())
}

fn metadata_identity(metadata: &fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    return Some((metadata.dev(), metadata.ino()));
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn copy_regular_file(
    source: &Path,
    target: &Path,
    expected_bytes: u64,
    expected_identity: Option<(u64, u64)>,
) -> Result<()> {
    let mut source_options = OpenOptions::new();
    source_options.read(true);
    #[cfg(unix)]
    source_options.custom_flags(libc::O_NOFOLLOW);
    let source_file = source_options
        .open(source)
        .with_context(|| format!("could not open clipboard file {}", source.display()))?;
    let source_metadata = source_file
        .metadata()
        .with_context(|| format!("could not inspect clipboard file {}", source.display()))?;
    validate_private_metadata(&source_metadata, source)?;
    if !source_metadata.is_file()
        || source_metadata.len() != expected_bytes
        || metadata_identity(&source_metadata) != expected_identity
    {
        bail!("clipboard file changed while it was being copied");
    }

    let mut target_options = OpenOptions::new();
    target_options.write(true).create_new(true);
    let mut target_file = target_options
        .open(target)
        .with_context(|| format!("could not create {}", target.display()))?;
    let copied = std::io::copy(
        &mut source_file.take(expected_bytes.saturating_add(1)),
        &mut target_file,
    );
    let copied = match copied {
        Ok(copied) => copied,
        Err(error) => {
            drop(target_file);
            let _ = fs::remove_file(target);
            return Err(error).with_context(|| format!("could not copy {}", source.display()));
        }
    };
    if copied != expected_bytes {
        let _ = fs::remove_file(target);
        bail!("clipboard file changed while it was being copied");
    }
    Ok(())
}

fn remove_imported_path(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
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

    #[cfg(unix)]
    struct StagedTransfer(PathBuf);

    #[cfg(unix)]
    impl StagedTransfer {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt as _;

            let staging_root = herdr_clipboard_staging_root();
            fs::create_dir_all(&staging_root).unwrap();
            fs::set_permissions(&staging_root, fs::Permissions::from_mode(0o700)).unwrap();
            let path = staging_root.join(format!(
                "hunkle-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn create_dir(&self, relative: &str) {
            use std::os::unix::fs::PermissionsExt as _;

            let path = self.0.join(relative);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }

        fn write(&self, relative: &str, contents: &[u8]) {
            use std::os::unix::fs::PermissionsExt as _;

            let path = self.0.join(relative);
            fs::write(&path, contents).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[cfg(unix)]
    impl Drop for StagedTransfer {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn imports_a_private_herdr_clipboard_tree() {
        let transfer = StagedTransfer::new();
        transfer.create_dir("project");
        transfer.create_dir("project/empty");
        transfer.write("project/README.md", b"hello");
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir(workspace.path().join("docs")).unwrap();

        let operation =
            clipboard_import_operation(transfer.0.to_str().unwrap(), RepoPath::from("docs"))
                .unwrap()
                .unwrap();
        assert_eq!(operation.selection_after(), Some("docs/project".into()));
        perform(workspace.path(), &operation).unwrap();

        assert_eq!(
            fs::read(workspace.path().join("docs/project/README.md")).unwrap(),
            b"hello"
        );
        assert!(workspace.path().join("docs/project/empty").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_import_preflights_all_top_level_conflicts() {
        let transfer = StagedTransfer::new();
        transfer.write("a.txt", b"new a");
        transfer.write("b.txt", b"new b");
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("b.txt"), b"old b").unwrap();
        let operation =
            clipboard_import_operation(transfer.0.to_str().unwrap(), RepoPath::default())
                .unwrap()
                .unwrap();

        assert!(perform(workspace.path(), &operation).is_err());
        assert!(!workspace.path().join("a.txt").exists());
        assert_eq!(fs::read(workspace.path().join("b.txt")).unwrap(), b"old b");
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_import_rejects_symlinks_and_unowned_paths() {
        use std::os::unix::fs::symlink;

        let transfer = StagedTransfer::new();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"secret").unwrap();
        symlink(outside.path().join("secret"), transfer.0.join("link")).unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let operation =
            clipboard_import_operation(transfer.0.to_str().unwrap(), RepoPath::default())
                .unwrap()
                .unwrap();

        assert!(perform(workspace.path(), &operation).is_err());
        assert!(!workspace.path().join("link").exists());
        assert!(
            clipboard_import_operation(outside.path().to_str().unwrap(), RepoPath::default())
                .unwrap()
                .is_none()
        );
    }
}
