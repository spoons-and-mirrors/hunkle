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
    pub(super) load_error: Option<String>,
}

impl KnownRepositoryStore {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        let (repositories, load_error) = match path.as_deref().map(load) {
            Some(Ok(repositories)) => (repositories, None),
            Some(Err(error)) => (Vec::new(), Some(error)),
            None => (Vec::new(), None),
        };
        Self {
            path,
            repositories,
            load_error,
        }
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
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "repositories": repositories,
        }))
        .map_err(|error| format!("Could not serialize known repositories: {error}"))?;
        atomic_write(path, format!("{content}\n").as_bytes())
            .map_err(|error| format!("Could not save known repositories: {error}"))
    }
}

fn load(path: &Path) -> Result<Vec<PathBuf>, String> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read known repositories: {error}")),
    };
    let value = serde_json::from_slice::<Value>(&content)
        .map_err(|error| format!("Could not parse known repositories: {error}"))?;
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Known repositories use an unsupported version".to_owned());
    }
    value
        .get("repositories")
        .and_then(Value::as_array)
        .ok_or_else(|| "Known repositories have no repository list".to_owned())?
        .iter()
        .map(repository_path)
        .collect()
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
