use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use anyhow::{Context, Result, bail};

use super::{FileOperation, safe_directory, safe_path, validate_name};
use crate::repo_path::RepoPath;

const STAGING_VERSION: &str = "v1";
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;
const MAX_ROOTS: usize = 64;
const MAX_DEPTH: usize = 64;
const MAX_PATH_BYTES: usize = 256 * 1024;

pub(crate) fn operation(text: &str, destination: RepoPath) -> Result<Option<FileOperation>> {
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
        let staging_root = staging_root();
        if source.parent() != Some(staging_root.as_path()) {
            return Ok(None);
        }
        validate_private_directory(&staging_root, "Herdr clipboard staging root")?;
        validate_private_directory(&source, "Herdr clipboard transfer")?;

        let mut children = read_sorted_directory(&source, MAX_ROOTS)?;
        if children.is_empty() {
            bail!("Herdr clipboard transfer is empty");
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
pub(super) fn staging_root() -> PathBuf {
    let user_id = unsafe { libc::geteuid() };
    std::env::temp_dir().join(format!("herdr-clipboard-files-{STAGING_VERSION}-{user_id}"))
}

struct Entry {
    source: PathBuf,
    relative: PathBuf,
    file_bytes: Option<u64>,
    identity: Option<(u64, u64)>,
}

pub(super) fn perform(root: &Path, source: &Path, destination: &RepoPath) -> Result<()> {
    #[cfg(unix)]
    {
        let staging_root = staging_root();
        if source.parent() != Some(staging_root.as_path()) {
            bail!("clipboard import is not an owned Herdr transfer");
        }
        validate_private_directory(&staging_root, "Herdr clipboard staging root")?;
        validate_private_directory(source, "Herdr clipboard transfer")?;
    }
    #[cfg(not(unix))]
    bail!("Herdr clipboard file import is only supported on Unix");

    safe_directory(root, destination)?;
    let entries = inspect_transfer(source)?;
    let top_level = entries
        .iter()
        .filter(|entry| entry.relative.components().count() == 1)
        .collect::<Vec<_>>();
    if top_level.is_empty() || top_level.len() > MAX_ROOTS {
        bail!("Herdr clipboard transfer has an invalid number of top-level entries");
    }
    for entry in &top_level {
        let target_relative = destination.join(entry.relative.as_os_str());
        let target = safe_path(root, &target_relative)?;
        match fs::symlink_metadata(&target) {
            Ok(_) => bail!("{} already exists", target.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("could not inspect import destination {}", target.display())
                });
            }
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

fn inspect_transfer(root: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    let mut total_path_bytes = 0_usize;
    for child in read_sorted_directory(root, MAX_ROOTS)? {
        let name = child.file_name();
        validate_import_name(&name)?;
        inspect_entry(
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

fn inspect_entry(
    source: &Path,
    relative: PathBuf,
    entries: &mut Vec<Entry>,
    total_bytes: &mut u64,
    total_path_bytes: &mut usize,
) -> Result<()> {
    if entries.len() >= MAX_ENTRIES {
        bail!("Herdr clipboard transfer has too many entries");
    }
    let depth = relative.components().count();
    if depth == 0 || depth > MAX_DEPTH {
        bail!("Herdr clipboard transfer is nested too deeply");
    }
    let path_bytes = relative
        .to_str()
        .context("Herdr clipboard transfer contains a non-UTF-8 path")?
        .len();
    *total_path_bytes = total_path_bytes
        .checked_add(path_bytes)
        .context("Herdr clipboard paths are too large")?;
    if *total_path_bytes > MAX_PATH_BYTES {
        bail!("Herdr clipboard paths are too large");
    }

    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("could not inspect clipboard entry {}", source.display()))?;
    validate_private_metadata(&metadata, source)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        entries.push(Entry {
            source: source.to_owned(),
            relative: relative.clone(),
            file_bytes: None,
            identity: metadata_identity(&metadata),
        });
        let remaining = MAX_ENTRIES.saturating_sub(entries.len());
        for child in read_sorted_directory(source, remaining)? {
            let name = child.file_name();
            validate_import_name(&name)?;
            inspect_entry(
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
    if *total_bytes > MAX_FILE_BYTES {
        bail!("Herdr clipboard files are too large");
    }
    entries.push(Entry {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    struct StagedTransfer(PathBuf);

    #[cfg(unix)]
    impl StagedTransfer {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt as _;

            let staging_root = staging_root();
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

        let operation = operation(transfer.0.to_str().unwrap(), RepoPath::from("docs"))
            .unwrap()
            .unwrap();
        assert_eq!(operation.selection_after(), Some("docs/project".into()));
        super::super::perform(workspace.path(), &operation).unwrap();

        assert_eq!(
            fs::read(workspace.path().join("docs/project/README.md")).unwrap(),
            b"hello"
        );
        assert!(workspace.path().join("docs/project/empty").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn preflights_all_top_level_conflicts() {
        let transfer = StagedTransfer::new();
        transfer.write("a.txt", b"new a");
        transfer.write("b.txt", b"new b");
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("b.txt"), b"old b").unwrap();

        assert!(perform(workspace.path(), &transfer.0, &RepoPath::default()).is_err());
        assert!(!workspace.path().join("a.txt").exists());
        assert_eq!(fs::read(workspace.path().join("b.txt")).unwrap(), b"old b");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_paths_outside_the_staging_namespace() {
        use std::os::unix::fs::symlink;

        let transfer = StagedTransfer::new();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"secret").unwrap();
        symlink(outside.path().join("secret"), transfer.0.join("link")).unwrap();
        let workspace = tempfile::tempdir().unwrap();

        assert!(perform(workspace.path(), &transfer.0, &RepoPath::default()).is_err());
        assert!(!workspace.path().join("link").exists());
        assert!(
            operation(outside.path().to_str().unwrap(), RepoPath::default())
                .unwrap()
                .is_none()
        );
    }
}
