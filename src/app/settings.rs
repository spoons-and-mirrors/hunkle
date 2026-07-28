use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{filesystem::atomic_write, media::MediaPreviewProtocol};

use super::{DEFAULT_WORKSPACE_PANEL_WIDTH, MINIMUM_WORKSPACE_PANEL_WIDTH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub auto_fetch: bool,
    pub fetch_interval_minutes: u16,
    pub worktree_width: u16,
    pub workspace_panel_enabled: bool,
    pub show_agent_harness: bool,
    pub workspace_panel_width: u16,
    pub history_height: u16,
    pub editor_command: Option<String>,
    pub media_preview_protocol: MediaPreviewProtocol,
}

impl Settings {
    pub(crate) fn fetch_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.fetch_interval_minutes) * 60)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_fetch: false,
            fetch_interval_minutes: 5,
            worktree_width: 38,
            workspace_panel_enabled: true,
            show_agent_harness: false,
            workspace_panel_width: DEFAULT_WORKSPACE_PANEL_WIDTH,
            history_height: 7,
            editor_command: None,
            media_preview_protocol: MediaPreviewProtocol::Auto,
        }
    }
}

pub(crate) struct SettingsStore {
    path: Option<PathBuf>,
}

impl SettingsStore {
    pub(crate) fn discover() -> (Self, Settings) {
        let path = config_path("hunkle");
        let settings = path
            .as_deref()
            .map(|path| {
                if path.exists() {
                    load(path)
                } else {
                    config_path("gitui")
                        .as_deref()
                        .map(load)
                        .unwrap_or_default()
                }
            })
            .unwrap_or_default();
        (Self { path }, settings)
    }

    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub(crate) fn config_dir(&self) -> Option<&Path> {
        self.path.as_deref()?.parent()
    }

    #[cfg(test)]
    pub(crate) fn load(&self) -> Settings {
        self.path.as_deref().map(load).unwrap_or_default()
    }

    pub(crate) fn save(&self, settings: &Settings) -> std::io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(
            path,
            format!(
                "auto_fetch={}\nfetch_interval_minutes={}\nworktree_width={}\nworkspace_panel_enabled={}\nshow_agent_harness={}\nworkspace_panel_width={}\nhistory_height={}\neditor_command={}\nmedia_preview_protocol={}\n",
                settings.auto_fetch,
                settings.fetch_interval_minutes,
                settings.worktree_width,
                settings.workspace_panel_enabled,
                settings.show_agent_harness,
                settings.workspace_panel_width,
                settings.history_height,
                settings.editor_command.as_deref().unwrap_or_default(),
                settings.media_preview_protocol.as_str(),
            )
            .as_bytes(),
        )
    }
}

fn config_path(app_name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(path).join(app_name).join("config"));
    }
    if let Some(path) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(path).join(app_name).join("config"));
    }
    home_directory().map(|home| home.join(".config").join(app_name).join("config"))
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn load(path: &Path) -> Settings {
    let Ok(contents) = fs::read_to_string(path) else {
        return Settings::default();
    };
    let mut settings = Settings::default();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "auto_fetch" => settings.auto_fetch = value.trim() == "true",
            "fetch_interval_minutes" => {
                if let Ok(minutes) = value.trim().parse::<u16>() {
                    settings.fetch_interval_minutes = minutes.clamp(1, 1440);
                }
            }
            "worktree_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.worktree_width = width.clamp(24, 4096);
                }
            }
            "workspace_panel_enabled" => {
                settings.workspace_panel_enabled = value.trim() == "true";
            }
            "show_agent_harness" => {
                settings.show_agent_harness = value.trim() == "true";
            }
            "workspace_panel_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.workspace_panel_width =
                        width.clamp(MINIMUM_WORKSPACE_PANEL_WIDTH, 4096);
                }
            }
            "history_height" => {
                if let Ok(height) = value.trim().parse::<u16>() {
                    settings.history_height = height.clamp(3, 256);
                }
            }
            "editor_command" => {
                let command = value.trim();
                settings.editor_command = (!command.is_empty()).then(|| command.to_owned());
            }
            "media_preview_protocol" => {
                settings.media_preview_protocol = match value.trim() {
                    "auto" => MediaPreviewProtocol::Auto,
                    "kitty" => MediaPreviewProtocol::Kitty,
                    "iterm2" => MediaPreviewProtocol::Iterm2,
                    "sixel" => MediaPreviewProtocol::Sixel,
                    _ => MediaPreviewProtocol::Halfblocks,
                };
            }
            _ => {}
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_loads_and_clamps_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/config");
        let store = SettingsStore::at(path.clone());
        let settings = Settings {
            auto_fetch: true,
            fetch_interval_minutes: 17,
            worktree_width: 61,
            workspace_panel_enabled: false,
            show_agent_harness: true,
            workspace_panel_width: 33,
            history_height: 9,
            editor_command: Some("code --wait".to_owned()),
            media_preview_protocol: MediaPreviewProtocol::Sixel,
        };

        store.save(&settings).unwrap();
        assert_eq!(store.load(), settings);

        fs::write(&path, "media_preview_protocol=iterm2\n").unwrap();
        assert_eq!(
            store.load().media_preview_protocol,
            MediaPreviewProtocol::Iterm2
        );

        fs::write(
            path,
            "auto_fetch=true\nfetch_interval_minutes=0\nworktree_width=5\nworkspace_panel_width=2\nhistory_height=1\nmedia_preview_protocol=unknown\n",
        )
        .unwrap();
        let loaded = store.load();
        assert_eq!(loaded.fetch_interval_minutes, 1);
        assert_eq!(loaded.worktree_width, 24);
        assert_eq!(loaded.workspace_panel_width, MINIMUM_WORKSPACE_PANEL_WIDTH);
        assert_eq!(loaded.history_height, 3);
        assert_eq!(
            loaded.media_preview_protocol,
            MediaPreviewProtocol::Halfblocks
        );
    }
}
