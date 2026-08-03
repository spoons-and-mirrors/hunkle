use std::{
    collections::{HashMap, HashSet},
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

pub(super) const MAX_KNOWN_REPOSITORIES: usize = 256;
pub(super) const MAX_RECENT_REPOSITORIES: usize = 64;

pub(super) struct KnownRepositoryStore {
    path: Option<PathBuf>,
    pub(super) repositories: Vec<PathBuf>,
    pub(super) recent: Vec<RecentRepository>,
    pub(super) load_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecentRepository {
    pub(super) common_dir: Option<PathBuf>,
    pub(super) root: PathBuf,
    pub(super) stats: Option<(u64, u64)>,
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
        let mut store = Self {
            path,
            repositories: stored.repositories,
            recent: stored.recent,
            load_error,
        };
        store.enforce_limits(&[]);
        store
    }

    pub(super) fn remember_and_save(
        &mut self,
        common_dir: Option<PathBuf>,
        root: PathBuf,
        relevant: &[PathBuf],
    ) -> Result<(), String> {
        let previous_repositories = self.repositories.clone();
        let previous_recent = self.recent.clone();
        let stats = self
            .recent
            .iter()
            .find(|recent| recent.common_dir == common_dir && recent.root == root)
            .and_then(|recent| recent.stats);
        if let Some(common_dir) = common_dir.as_ref() {
            self.insert(common_dir.clone());
        }
        self.recent.retain(|recent| match common_dir.as_ref() {
            Some(common_dir) => recent.common_dir.as_ref() != Some(common_dir),
            None => recent.common_dir.is_some() || recent.root != root,
        });
        self.recent.insert(
            0,
            RecentRepository {
                common_dir,
                root,
                stats,
            },
        );
        self.enforce_limits(relevant);
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
        true
    }

    fn extend(&mut self, repositories: Vec<PathBuf>) {
        for repository in repositories {
            self.insert(repository);
        }
    }

    pub(super) fn reconcile_and_save(
        &mut self,
        discovered: Vec<PathBuf>,
        pruned: &[PathBuf],
        relevant: &[PathBuf],
    ) -> Result<(), String> {
        let previous_repositories = self.repositories.clone();
        let previous_recent = self.recent.clone();
        self.extend(discovered);
        self.repositories
            .retain(|path| !pruned.iter().any(|pruned| pruned == path));
        self.recent.retain(|recent| {
            recent
                .common_dir
                .as_ref()
                .is_none_or(|common_dir| !pruned.iter().any(|pruned| pruned == common_dir))
        });
        self.enforce_limits(relevant);
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

    fn enforce_limits(&mut self, relevant: &[PathBuf]) {
        let mut seen_common_dirs = HashSet::new();
        let mut seen_local_roots = HashSet::new();
        self.recent
            .retain(|recent| match recent.common_dir.as_ref() {
                Some(common_dir) => seen_common_dirs.insert(common_dir.clone()),
                None => seen_local_roots.insert(recent.root.clone()),
            });
        self.recent.truncate(MAX_RECENT_REPOSITORIES);

        let mut seen = HashSet::new();
        self.repositories
            .retain(|repository| seen.insert(repository.clone()));
        if self.repositories.len() <= MAX_KNOWN_REPOSITORIES {
            return;
        }

        let protected = relevant
            .iter()
            .chain(
                self.recent
                    .iter()
                    .filter_map(|recent| recent.common_dir.as_ref()),
            )
            .collect::<HashSet<_>>();
        let protected_count = self
            .repositories
            .iter()
            .filter(|repository| protected.contains(repository))
            .count();
        let remove_count = self
            .repositories
            .len()
            .saturating_sub(MAX_KNOWN_REPOSITORIES.max(protected_count));
        let removed = self
            .repositories
            .iter()
            .filter(|repository| !protected.contains(repository))
            .take(remove_count)
            .cloned()
            .collect::<HashSet<_>>();
        self.repositories
            .retain(|repository| !removed.contains(repository));
    }

    pub(super) fn update_stats_and_save(
        &mut self,
        stats: &[(PathBuf, (u64, u64))],
    ) -> Result<bool, String> {
        let previous = self.recent.clone();
        let stats = stats
            .iter()
            .map(|(root, stats)| (root.as_path(), *stats))
            .collect::<HashMap<_, _>>();
        for recent in &mut self.recent {
            if let Some(stats) = stats.get(recent.root.as_path()) {
                recent.stats = Some(*stats);
            }
        }
        if self.recent == previous {
            return Ok(false);
        }
        if let Err(error) = self.save() {
            self.recent = previous;
            return Err(error);
        }
        Ok(true)
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
                    "common_dir": recent.common_dir.as_deref().map(stored_path_value),
                    "root": stored_path_value(&recent.root),
                    "stats": recent.stats.map(|(additions, deletions)| serde_json::json!({
                        "additions": additions,
                        "deletions": deletions,
                    })),
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
    if let Some(root) = value.get("root") {
        return Ok(RecentRepository {
            common_dir: value
                .get("common_dir")
                .filter(|common_dir| !common_dir.is_null())
                .map(stored_path)
                .transpose()?,
            root: stored_path(root)?,
            stats: stored_stats(value.get("stats"))?,
        });
    }
    let common_dir = repository_path(value)?;
    let root = if common_dir.file_name().is_some_and(|name| name == ".git") {
        common_dir.parent().unwrap_or(&common_dir).to_owned()
    } else {
        common_dir.clone()
    };
    Ok(RecentRepository {
        common_dir: Some(common_dir),
        root,
        stats: None,
    })
}

fn stored_stats(value: Option<&Value>) -> Result<Option<(u64, u64)>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let additions = value
        .get("additions")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Recent repository stats have invalid additions".to_owned())?;
    let deletions = value
        .get("deletions")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Recent repository stats have invalid deletions".to_owned())?;
    Ok(Some((additions, deletions)))
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
