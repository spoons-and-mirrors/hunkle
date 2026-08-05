use std::path::PathBuf;

use anyhow::{Context, Result};

pub(crate) fn data_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("hunkle"));
    }
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .filter(|path| !path.is_empty())
    {
        return Ok(PathBuf::from(path).join("hunkle"));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("the home directory is unavailable")?;
    #[cfg(windows)]
    return Ok(PathBuf::from(home)
        .join("AppData")
        .join("Local")
        .join("hunkle"));
    #[cfg(not(windows))]
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("hunkle"))
}
