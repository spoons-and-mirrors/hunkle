use std::{
    env, fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
};

/// A one-way handoff from the active OpenCode CLI tab. No OpenCode process or session is owned here.
pub(crate) struct OpenCodePresence {
    path: PathBuf,
    seen: Option<(u64, i64, i64)>,
}

impl OpenCodePresence {
    pub(crate) fn new() -> Self {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!("/tmp/hunkle-{}", unsafe { libc::geteuid() }))
            });
        Self::at(runtime.join("hunkle/opencode-active.json"))
    }

    fn at(path: PathBuf) -> Self {
        let seen = fingerprint(&path);
        Self { path, seen }
    }

    pub(crate) fn poll(&mut self) -> Option<PathBuf> {
        let current = fingerprint(&self.path);
        if current == self.seen {
            return None;
        }
        // A missing file is not a snapshot; the next atomic publication is a change.
        let Some(current) = current else {
            self.seen = None;
            return None;
        };
        self.seen = Some(current);
        let directory = self.path.parent()?;
        let owner = unsafe { libc::geteuid() };
        let dir = fs::symlink_metadata(directory).ok()?;
        if !dir.is_dir() || dir.uid() != owner {
            return None;
        }
        // Hunkle's existing runtime directory can be group-writable. It is still
        // same-user-only when enclosed by a private runtime root (e.g. /run/user/1000).
        let private_parent = fs::symlink_metadata(directory.parent()?).ok()?;
        let enclosed = private_parent.is_dir()
            && private_parent.uid() == owner
            && private_parent.permissions().mode() & 0o077 == 0;
        if dir.permissions().mode() & 0o022 != 0 && !enclosed {
            return None;
        }
        let file = fs::symlink_metadata(&self.path).ok()?;
        if !file.is_file() || file.uid() != owner || file.len() > 4096 {
            return None;
        }
        let bytes = fs::read(&self.path).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        if value.get("version")?.as_u64()? != 1 {
            return None;
        }
        let path = PathBuf::from(value.get("directory")?.as_str()?);
        path.is_absolute().then_some(path)
    }

    #[cfg(test)]
    pub(crate) fn at_for_test(path: PathBuf) -> Self {
        Self::at(path)
    }
}

fn fingerprint(path: &std::path::Path) -> Option<(u64, i64, i64)> {
    let meta = fs::symlink_metadata(path).ok()?;
    Some((meta.ino(), meta.mtime(), meta.mtime_nsec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_publication_is_baseline_and_replacement_follows() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("opencode-active.json");
        fs::write(&path, r#"{"version":1,"directory":"/first"}"#).unwrap();
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        assert_eq!(presence.poll(), None);
        let next = root.path().join("next");
        fs::write(&next, r#"{"version":1,"directory":"/second"}"#).unwrap();
        fs::rename(next, &path).unwrap();
        assert_eq!(presence.poll(), Some(PathBuf::from("/second")));
        assert_eq!(presence.poll(), None);
    }

    #[test]
    fn rejects_invalid_or_untrusted_publications() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("opencode-active.json");
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        fs::write(&path, r#"{"version":1,"directory":"relative"}"#).unwrap();
        assert_eq!(presence.poll(), None);
        fs::remove_file(&path).unwrap();
        presence.poll();
        std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
        assert_eq!(presence.poll(), None);
    }

    #[test]
    fn accepts_existing_group_writable_hunkle_directory_under_private_runtime() {
        let runtime = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let directory = runtime.path().join("hunkle");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o775)).unwrap();
        let path = directory.join("opencode-active.json");
        let mut presence = OpenCodePresence::at_for_test(path.clone());
        fs::write(&path, r#"{"version":1,"directory":"/workspace"}"#).unwrap();
        assert_eq!(presence.poll(), Some(PathBuf::from("/workspace")));
    }
}
