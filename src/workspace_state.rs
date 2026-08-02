use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::filesystem::atomic_write;

#[cfg(unix)]
type EncodedPath = Vec<u8>;
#[cfg(windows)]
type EncodedPath = Vec<u16>;
#[cfg(not(any(unix, windows)))]
type EncodedPath = String;

#[derive(Deserialize, Serialize)]
struct Record {
    base: EncodedPath,
    active: EncodedPath,
}

pub(crate) struct StartupWorkspace {
    pub(crate) path: PathBuf,
    pub(crate) state: Option<WorkspaceState>,
}

pub(crate) struct WorkspaceState {
    path: PathBuf,
    base: PathBuf,
}

impl WorkspaceState {
    pub(crate) fn resolve(explicit: Option<PathBuf>, current: PathBuf) -> StartupWorkspace {
        let Some(path) = state_path() else {
            return StartupWorkspace {
                path: explicit.unwrap_or(current),
                state: None,
            };
        };
        Self::resolve_at(path, explicit, current)
    }

    fn resolve_at(path: PathBuf, explicit: Option<PathBuf>, current: PathBuf) -> StartupWorkspace {
        let record = load(&path).ok();
        let same_base = record
            .as_ref()
            .is_some_and(|record| decode_path(&record.base) == current);
        let restored = record
            .as_ref()
            .filter(|_| same_base)
            .map(|record| decode_path(&record.active))
            .filter(|path| path.exists());
        StartupWorkspace {
            path: explicit.or(restored).unwrap_or_else(|| current.clone()),
            state: Some(Self {
                path,
                base: if same_base {
                    record
                        .as_ref()
                        .map(|record| decode_path(&record.base))
                        .unwrap_or(current)
                } else {
                    current
                },
            }),
        }
    }

    pub(crate) fn save(&self, active: &Path) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(&Record {
            base: encode_path(&self.base),
            active: encode_path(active),
        })
        .map_err(io::Error::other)?;
        atomic_write(&self.path, &bytes)
    }
}

fn load(path: &Path) -> io::Result<Record> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

fn state_path() -> Option<PathBuf> {
    env::var_os("HERDR_ENV")?;
    let workspace = env::var_os("HERDR_WORKSPACE_ID")?;
    let pane = env::var_os("HERDR_PANE_ID")?;
    let root = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })?;
    let key = format!(
        "{}-{}",
        hex(workspace.as_encoded_bytes()),
        hex(pane.as_encoded_bytes())
    );
    Some(
        root.join("hunkle")
            .join("workspaces")
            .join(format!("{key}.json")),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(unix)]
fn encode_path(path: &Path) -> EncodedPath {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn decode_path(path: &EncodedPath) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(path.clone()).into()
}

#[cfg(windows)]
fn encode_path(path: &Path) -> EncodedPath {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}

#[cfg(windows)]
fn decode_path(path: &EncodedPath) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    std::ffi::OsString::from_wide(path).into()
}

#[cfg(not(any(unix, windows)))]
fn encode_path(path: &Path) -> EncodedPath {
    path.to_string_lossy().into_owned()
}

#[cfg(not(any(unix, windows)))]
fn decode_path(path: &EncodedPath) -> PathBuf {
    path.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_only_from_the_same_shell_directory_without_an_explicit_path() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("base");
        let active = directory.path().join("active");
        let other = directory.path().join("other");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(&other).unwrap();
        let state_path = directory.path().join("state.json");

        let initial = WorkspaceState::resolve_at(state_path.clone(), None, base.clone());
        initial.state.unwrap().save(&active).unwrap();

        let restored = WorkspaceState::resolve_at(state_path.clone(), None, base.clone());
        assert_eq!(restored.path, active);
        let moved_shell = WorkspaceState::resolve_at(state_path.clone(), None, other.clone());
        assert_eq!(moved_shell.path, other);
        let explicit =
            WorkspaceState::resolve_at(state_path, Some(directory.path().join("explicit")), base);
        assert_eq!(explicit.path, directory.path().join("explicit"));
    }
}
