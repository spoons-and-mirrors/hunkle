use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use serde_json::Value;

use crate::filesystem::atomic_write;

pub(super) struct KnownRepositoryStore {
    path: Option<PathBuf>,
    pub(super) repositories: Vec<PathBuf>,
    pub(super) recent: Vec<RecentRepository>,
    pub(super) load_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecentRepository {
    pub(super) common_dir: PathBuf,
    pub(super) root: PathBuf,
}

struct StoredRepositories {
    repositories: Vec<PathBuf>,
    recent: Vec<RecentRepository>,
}

impl KnownRepositoryStore {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        let (stored, load_error) = match path.as_deref().map(load) {
            Some(Ok(stored)) => (stored, None),
            Some(Err(error)) => (
                StoredRepositories {
                    repositories: Vec::new(),
                    recent: Vec::new(),
                },
                Some(error),
            ),
            None => (
                StoredRepositories {
                    repositories: Vec::new(),
                    recent: Vec::new(),
                },
                None,
            ),
        };
        Self {
            path,
            repositories: stored.repositories,
            recent: stored.recent,
            load_error,
        }
    }

    pub(super) fn remember_and_save(
        &mut self,
        common_dir: PathBuf,
        root: PathBuf,
    ) -> Result<(), String> {
        let previous_repositories = self.repositories.clone();
        let previous_recent = self.recent.clone();
        self.insert(common_dir.clone());
        self.recent.retain(|recent| recent.common_dir != common_dir);
        self.recent.insert(0, RecentRepository { common_dir, root });
        if self.repositories == previous_repositories && self.recent == previous_recent {
            return Ok(());
        }
        if let Err(error) = self.save() {
            self.repositories = previous_repositories;
            self.recent = previous_recent;
            return Err(error);
        }
        Ok(())
    }

    fn insert(&mut self, common_dir: PathBuf) -> bool {
        if self.repositories.iter().any(|known| known == &common_dir) {
            return false;
        }
        self.repositories.push(common_dir);
        self.repositories
            .sort_by_cached_key(|path| path.to_string_lossy().to_lowercase());
        true
    }

    fn extend(&mut self, repositories: Vec<PathBuf>) {
        for repository in repositories {
            self.insert(repository);
        }
    }

    pub(super) fn extend_and_save(&mut self, repositories: Vec<PathBuf>) -> Result<(), String> {
        let previous = self.repositories.clone();
        self.extend(repositories);
        if self.repositories == previous {
            return Ok(());
        }
        if let Err(error) = self.save() {
            self.repositories = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn prune_and_save(&mut self, repositories: &[PathBuf]) -> Result<(), String> {
        let previous_repositories = self.repositories.clone();
        let previous_recent = self.recent.clone();
        self.repositories
            .retain(|path| !repositories.iter().any(|pruned| pruned == path));
        self.recent.retain(|recent| {
            !repositories
                .iter()
                .any(|pruned| pruned == &recent.common_dir)
        });
        if self.repositories == previous_repositories && self.recent == previous_recent {
            return Ok(());
        }
        if let Err(error) = self.save() {
            self.repositories = previous_repositories;
            self.recent = previous_recent;
            return Err(error);
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        if let Some(error) = self.load_error.as_deref() {
            return Err(format!(
                "{error}; refusing to overwrite the unreadable repository inventory"
            ));
        }
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create Hunkle config directory: {error}"))?;
        }
        let repositories = self
            .repositories
            .iter()
            .map(|common_dir| repository_value(common_dir))
            .collect::<Vec<_>>();
        let recent = self
            .recent
            .iter()
            .map(|recent| {
                serde_json::json!({
                    "common_dir": stored_path_value(&recent.common_dir),
                    "root": stored_path_value(&recent.root),
                })
            })
            .collect::<Vec<_>>();
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "repositories": repositories,
            "recent": recent,
        }))
        .map_err(|error| format!("Could not serialize known repositories: {error}"))?;
        atomic_write(path, format!("{content}\n").as_bytes())
            .map_err(|error| format!("Could not save known repositories: {error}"))
    }
}

fn load(path: &Path) -> Result<StoredRepositories, String> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(StoredRepositories {
                repositories: Vec::new(),
                recent: Vec::new(),
            });
        }
        Err(error) => return Err(format!("Could not read known repositories: {error}")),
    };
    let value = serde_json::from_slice::<Value>(&content)
        .map_err(|error| format!("Could not parse known repositories: {error}"))?;
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Known repositories use an unsupported version".to_owned());
    }
    let repositories = value
        .get("repositories")
        .and_then(Value::as_array)
        .ok_or_else(|| "Known repositories have no repository list".to_owned())?
        .iter()
        .map(repository_path)
        .collect::<Result<Vec<_>, _>>()?;
    let recent = value
        .get("recent")
        .map(|recent| {
            recent
                .as_array()
                .ok_or_else(|| "Known repositories have an invalid recent list".to_owned())?
                .iter()
                .map(recent_repository)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(StoredRepositories {
        repositories,
        recent,
    })
}

fn repository_value(path: &Path) -> Value {
    if let Some(path) = path.to_str() {
        return serde_json::json!({ "common_dir": path });
    }
    #[cfg(unix)]
    {
        serde_json::json!({ "common_dir_bytes": path.as_os_str().as_bytes() })
    }
    #[cfg(windows)]
    {
        serde_json::json!({
            "common_dir_wide": path.as_os_str().encode_wide().collect::<Vec<_>>()
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        serde_json::json!({ "common_dir": path.to_string_lossy() })
    }
}

fn stored_path_value(path: &Path) -> Value {
    if let Some(path) = path.to_str() {
        return serde_json::json!({ "path": path });
    }
    #[cfg(unix)]
    {
        serde_json::json!({ "path_bytes": path.as_os_str().as_bytes() })
    }
    #[cfg(windows)]
    {
        serde_json::json!({
            "path_wide": path.as_os_str().encode_wide().collect::<Vec<_>>()
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        serde_json::json!({ "path": path.to_string_lossy() })
    }
}

fn recent_repository(value: &Value) -> Result<RecentRepository, String> {
    if let (Some(common_dir), Some(root)) = (value.get("common_dir"), value.get("root")) {
        return Ok(RecentRepository {
            common_dir: stored_path(common_dir)?,
            root: stored_path(root)?,
        });
    }
    let common_dir = repository_path(value)?;
    let root = if common_dir.file_name().is_some_and(|name| name == ".git") {
        common_dir.parent().unwrap_or(&common_dir).to_owned()
    } else {
        common_dir.clone()
    };
    Ok(RecentRepository { common_dir, root })
}

fn stored_path(value: &Value) -> Result<PathBuf, String> {
    if let Some(path) = value.get("path").and_then(Value::as_str) {
        return Ok(PathBuf::from(path));
    }
    #[cfg(unix)]
    if let Some(bytes) = value.get("path_bytes").and_then(Value::as_array) {
        let bytes = bytes
            .iter()
            .map(|byte| {
                byte.as_u64()
                    .and_then(|byte| u8::try_from(byte).ok())
                    .ok_or_else(|| "Recent repository path contains an invalid byte".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
    }
    #[cfg(windows)]
    if let Some(wide) = value.get("path_wide").and_then(Value::as_array) {
        let wide = wide
            .iter()
            .map(|unit| {
                unit.as_u64()
                    .and_then(|unit| u16::try_from(unit).ok())
                    .ok_or_else(|| "Recent repository path contains an invalid unit".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)));
    }
    Err("Recent repository path is malformed".to_owned())
}

fn repository_path(value: &Value) -> Result<PathBuf, String> {
    if let Some(path) = value.get("common_dir").and_then(Value::as_str) {
        return Ok(PathBuf::from(path));
    }
    #[cfg(unix)]
    if let Some(bytes) = value.get("common_dir_bytes").and_then(Value::as_array) {
        let bytes = bytes
            .iter()
            .map(|byte| {
                byte.as_u64()
                    .and_then(|byte| u8::try_from(byte).ok())
                    .ok_or_else(|| "Known repository path contains an invalid byte".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
    }
    #[cfg(windows)]
    if let Some(wide) = value.get("common_dir_wide").and_then(Value::as_array) {
        let wide = wide
            .iter()
            .map(|unit| {
                unit.as_u64()
                    .and_then(|unit| u16::try_from(unit).ok())
                    .ok_or_else(|| "Known repository path contains an invalid unit".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)));
    }
    Err("Known repository entry has no path".to_owned())
}
