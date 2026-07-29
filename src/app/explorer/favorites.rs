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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExplorerFavorite {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug)]
pub(super) struct FavoriteStore {
    path: Option<PathBuf>,
    load_error: Option<String>,
}

impl FavoriteStore {
    pub(super) fn new(path: Option<PathBuf>) -> (Self, Vec<ExplorerFavorite>) {
        let (favorites, load_error) = match path.as_deref().map(load) {
            Some(Ok(favorites)) => (favorites, None),
            Some(Err(error)) => (Vec::new(), Some(error)),
            None => (Vec::new(), None),
        };
        (Self { path, load_error }, favorites)
    }

    pub(super) fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub(super) fn save(&self, favorites: &[ExplorerFavorite]) -> Result<(), String> {
        if let Some(error) = self.load_error.as_deref() {
            return Err(format!(
                "{error}; refusing to overwrite unreadable Explorer favorites"
            ));
        }
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create Hunkle config directory: {error}"))?;
        }
        let favorites = favorites.iter().map(favorite_value).collect::<Vec<_>>();
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "favorites": favorites,
        }))
        .map_err(|error| format!("Could not serialize Explorer favorites: {error}"))?;
        atomic_write(path, format!("{content}\n").as_bytes())
            .map_err(|error| format!("Could not save Explorer favorites: {error}"))
    }
}

fn load(path: &Path) -> Result<Vec<ExplorerFavorite>, String> {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read Explorer favorites: {error}")),
    };
    let value = serde_json::from_slice::<Value>(&content)
        .map_err(|error| format!("Could not parse Explorer favorites: {error}"))?;
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Explorer favorites use an unsupported version".to_owned());
    }
    let favorites = value
        .get("favorites")
        .and_then(Value::as_array)
        .ok_or_else(|| "Explorer favorites have no favorites list".to_owned())?
        .iter()
        .map(favorite_from_value)
        .collect::<Result<Vec<_>, _>>()?;
    if favorites.iter().enumerate().any(|(index, favorite)| {
        favorites[..index]
            .iter()
            .any(|other| other.path == favorite.path)
    }) {
        return Err("Explorer favorites contain a duplicate path".to_owned());
    }
    Ok(favorites)
}

fn favorite_value(favorite: &ExplorerFavorite) -> Value {
    let mut value = path_value(&favorite.path);
    value["name"] = Value::String(favorite.name.clone());
    value
}

fn favorite_from_value(value: &Value) -> Result<ExplorerFavorite, String> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Explorer favorite has no name".to_owned())?
        .to_owned();
    let path = favorite_path(value)?;
    if !path.is_absolute() {
        return Err("Explorer favorite path is not absolute".to_owned());
    }
    Ok(ExplorerFavorite { name, path })
}

fn path_value(path: &Path) -> Value {
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

fn favorite_path(value: &Value) -> Result<PathBuf, String> {
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
                    .ok_or_else(|| "Explorer favorite path contains an invalid byte".to_owned())
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
                    .ok_or_else(|| "Explorer favorite path contains an invalid unit".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)));
    }
    Err("Explorer favorite has no path".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_named_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("favorites.json");
        let (store, favorites) = FavoriteStore::new(Some(path.clone()));
        assert!(favorites.is_empty());

        store
            .save(&[ExplorerFavorite {
                name: "Projects".to_owned(),
                path: directory.path().to_path_buf(),
            }])
            .unwrap();

        let (_, favorites) = FavoriteStore::new(Some(path));
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].name, "Projects");
        assert_eq!(favorites[0].path, directory.path());
    }

    #[test]
    fn refuses_to_overwrite_malformed_favorites() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("favorites.json");
        fs::write(&path, b"not json").unwrap();
        let (store, favorites) = FavoriteStore::new(Some(path.clone()));

        assert!(favorites.is_empty());
        assert!(store.save(&[]).is_err());
        assert_eq!(fs::read(path).unwrap(), b"not json");
    }
}
