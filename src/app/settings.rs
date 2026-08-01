use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{filesystem::atomic_write, media::MediaPreviewProtocol};

use super::{GraphColumn, Shortcuts, explorer::MINIMUM_EXPLORER_PANE_WIDTH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTimeDisplay {
    LatestLoop,
    AgentTotal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCodeReasoning {
    Default,
    Minimal,
    Low,
    Medium,
    High,
    Max,
}

impl OpenCodeReasoning {
    const ALL: [Self; 6] = [
        Self::Default,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Max,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Max => "Max",
        }
    }

    pub(crate) fn variant(self) -> Option<&'static str> {
        (self != Self::Default).then(|| self.as_str())
    }

    pub(crate) fn next(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|value| *value == self)
            .unwrap_or(0);
        let count = Self::ALL.len() as isize;
        Self::ALL[(index as isize + delta).rem_euclid(count) as usize]
    }

    fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reasoning| reasoning.as_str() == value)
    }
}

impl AgentTimeDisplay {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LatestLoop => "latest",
            Self::AgentTotal => "all",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::LatestLoop => Self::AgentTotal,
            Self::AgentTotal => Self::LatestLoop,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::LatestLoop => "Latest loop",
            Self::AgentTotal => "Agent total",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub auto_fetch: bool,
    pub fetch_interval_minutes: u16,
    pub format_on_save: bool,
    pub worktree_width: u16,
    pub workspace_panel_enabled: bool,
    pub cross_workspace_agents: bool,
    pub show_agent_harness: bool,
    pub agent_time_display: AgentTimeDisplay,
    pub agents_height: u16,
    pub graph_lane_width: u16,
    pub graph_description_width: u16,
    pub graph_changes_width: u16,
    pub graph_date_width: u16,
    pub graph_author_width: u16,
    pub graph_commit_width: u16,
    pub explorer_left_pane_width: Option<u16>,
    pub editor_command: Option<String>,
    pub opencode_model: String,
    pub opencode_reasoning: OpenCodeReasoning,
    pub media_preview_protocol: MediaPreviewProtocol,
    pub shortcuts: Shortcuts,
}

impl Settings {
    pub(crate) fn fetch_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.fetch_interval_minutes) * 60)
    }

    pub(crate) fn graph_column_width(&self, column: GraphColumn) -> u16 {
        match column {
            GraphColumn::Graph => self.graph_lane_width,
            GraphColumn::Description => self.graph_description_width,
            GraphColumn::Changes => self.graph_changes_width,
            GraphColumn::Date => self.graph_date_width,
            GraphColumn::Author => self.graph_author_width,
            GraphColumn::Commit => self.graph_commit_width,
        }
    }

    pub(crate) fn set_graph_column_width(&mut self, column: GraphColumn, width: u16) {
        let width = width.clamp(column.minimum_width(), 80);
        match column {
            GraphColumn::Graph => self.graph_lane_width = width,
            GraphColumn::Description => self.graph_description_width = width,
            GraphColumn::Changes => self.graph_changes_width = width,
            GraphColumn::Date => self.graph_date_width = width,
            GraphColumn::Author => self.graph_author_width = width,
            GraphColumn::Commit => self.graph_commit_width = width,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_fetch: false,
            fetch_interval_minutes: 5,
            format_on_save: true,
            worktree_width: 38,
            workspace_panel_enabled: true,
            cross_workspace_agents: false,
            show_agent_harness: false,
            agent_time_display: AgentTimeDisplay::LatestLoop,
            agents_height: 7,
            graph_lane_width: 0,
            graph_description_width: 0,
            graph_changes_width: 11,
            graph_date_width: 11,
            graph_author_width: 16,
            graph_commit_width: 7,
            explorer_left_pane_width: None,
            editor_command: None,
            opencode_model: "opencode/deepseek-v4-flash-free".to_owned(),
            opencode_reasoning: OpenCodeReasoning::Max,
            media_preview_protocol: MediaPreviewProtocol::Auto,
            shortcuts: Shortcuts::default(),
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
    pub(crate) fn memory() -> Self {
        Self { path: None }
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
        let mut contents = format!(
            "auto_fetch={}\nfetch_interval_minutes={}\nformat_on_save={}\nworktree_width={}\nworkspace_panel_enabled={}\ncross_workspace_agents={}\nshow_agent_harness={}\nagent_time_display={}\nagents_height={}\ngraph_lane_width={}\ngraph_description_width={}\ngraph_changes_width={}\ngraph_date_width={}\ngraph_author_width={}\ngraph_commit_width={}\nexplorer_left_pane_width={}\neditor_command={}\nopencode_model={}\nopencode_reasoning={}\nmedia_preview_protocol={}\n",
            settings.auto_fetch,
            settings.fetch_interval_minutes,
            settings.format_on_save,
            settings.worktree_width,
            settings.workspace_panel_enabled,
            settings.cross_workspace_agents,
            settings.show_agent_harness,
            settings.agent_time_display.as_str(),
            settings.agents_height,
            settings.graph_lane_width,
            settings.graph_description_width,
            settings.graph_changes_width,
            settings.graph_date_width,
            settings.graph_author_width,
            settings.graph_commit_width,
            settings
                .explorer_left_pane_width
                .map(|width| width.to_string())
                .unwrap_or_default(),
            settings.editor_command.as_deref().unwrap_or_default(),
            settings.opencode_model,
            settings.opencode_reasoning.as_str(),
            settings.media_preview_protocol.as_str(),
        );
        for (id, binding) in settings.shortcuts.serialized() {
            contents.push_str(&format!("shortcut.{id}={binding}\n"));
        }
        atomic_write(path, contents.as_bytes())
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
        let key = key.trim();
        if let Some(id) = key.strip_prefix("shortcut.") {
            settings.shortcuts.load_override(id, value.trim());
            continue;
        }
        match key {
            "auto_fetch" => settings.auto_fetch = value.trim() == "true",
            "fetch_interval_minutes" => {
                if let Ok(minutes) = value.trim().parse::<u16>() {
                    settings.fetch_interval_minutes = minutes.clamp(1, 1440);
                }
            }
            "format_on_save" => settings.format_on_save = value.trim() == "true",
            "worktree_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.worktree_width = width.clamp(24, 4096);
                }
            }
            "workspace_panel_enabled" => {
                settings.workspace_panel_enabled = value.trim() == "true";
            }
            "cross_workspace_agents" => {
                settings.cross_workspace_agents = value.trim() == "true";
            }
            "show_agent_harness" => {
                settings.show_agent_harness = value.trim() == "true";
            }
            "agent_time_display" => {
                settings.agent_time_display = match value.trim() {
                    "all" | "session" => AgentTimeDisplay::AgentTotal,
                    _ => AgentTimeDisplay::LatestLoop,
                };
            }
            "agents_height" | "history_height" => {
                if let Ok(height) = value.trim().parse::<u16>() {
                    settings.agents_height = height.clamp(5, 256);
                }
            }
            "graph_changes_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.set_graph_column_width(GraphColumn::Changes, width);
                }
            }
            "graph_lane_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.graph_lane_width = if width == 0 {
                        0
                    } else {
                        width.clamp(GraphColumn::Graph.minimum_width(), 80)
                    };
                }
            }
            "graph_description_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.graph_description_width = if width == 0 {
                        0
                    } else {
                        width.clamp(GraphColumn::Description.minimum_width(), 80)
                    };
                }
            }
            "graph_date_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.set_graph_column_width(GraphColumn::Date, width);
                }
            }
            "graph_author_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.set_graph_column_width(GraphColumn::Author, width);
                }
            }
            "graph_commit_width" => {
                if let Ok(width) = value.trim().parse::<u16>() {
                    settings.set_graph_column_width(GraphColumn::Commit, width);
                }
            }
            "explorer_left_pane_width" => {
                settings.explorer_left_pane_width = value
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .map(|width| width.clamp(MINIMUM_EXPLORER_PANE_WIDTH, 4096));
            }
            "editor_command" => {
                let command = value.trim();
                settings.editor_command = (!command.is_empty()).then(|| command.to_owned());
            }
            "opencode_model" => {
                let model = value.trim();
                if valid_opencode_model(model) {
                    settings.opencode_model = model.to_owned();
                }
            }
            "opencode_reasoning" => {
                if let Some(reasoning) = OpenCodeReasoning::parse(value.trim()) {
                    settings.opencode_reasoning = reasoning;
                }
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

pub(crate) fn valid_opencode_model(model: &str) -> bool {
    !model.is_empty() && !model.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{KeyChord, ShortcutAction};

    #[test]
    fn saves_loads_and_clamps_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/config");
        let store = SettingsStore::at(path.clone());
        let settings = Settings {
            auto_fetch: true,
            fetch_interval_minutes: 17,
            format_on_save: false,
            worktree_width: 61,
            workspace_panel_enabled: false,
            cross_workspace_agents: true,
            show_agent_harness: true,
            agent_time_display: AgentTimeDisplay::AgentTotal,
            agents_height: 9,
            graph_lane_width: 12,
            graph_description_width: 31,
            graph_changes_width: 13,
            graph_date_width: 18,
            graph_author_width: 21,
            graph_commit_width: 9,
            explorer_left_pane_width: Some(47),
            editor_command: Some("code --wait".to_owned()),
            opencode_model: "anthropic/claude-sonnet-4-5".to_owned(),
            opencode_reasoning: OpenCodeReasoning::High,
            media_preview_protocol: MediaPreviewProtocol::Sixel,
            shortcuts: {
                let mut shortcuts = Shortcuts::default();
                shortcuts
                    .set(
                        ShortcutAction::OpenExplorer,
                        KeyChord::new(
                            crossterm::event::KeyCode::Char('v'),
                            crossterm::event::KeyModifiers::ALT,
                        ),
                    )
                    .unwrap();
                shortcuts
            },
        };

        store.save(&settings).unwrap();
        assert_eq!(store.load(), settings);
        assert_eq!(
            settings.shortcuts.label(ShortcutAction::OpenExplorer),
            "Alt+v"
        );

        fs::write(&path, "agent_time_display=session\n").unwrap();
        assert_eq!(
            store.load().agent_time_display,
            AgentTimeDisplay::AgentTotal
        );

        fs::write(&path, "media_preview_protocol=iterm2\n").unwrap();
        assert_eq!(
            store.load().media_preview_protocol,
            MediaPreviewProtocol::Iterm2
        );

        fs::write(
            path,
            "auto_fetch=true\nfetch_interval_minutes=0\nworktree_width=5\nhistory_height=1\nexplorer_left_pane_width=2\nmedia_preview_protocol=unknown\n",
        )
        .unwrap();
        let loaded = store.load();
        assert!(loaded.format_on_save);
        assert_eq!(loaded.fetch_interval_minutes, 1);
        assert_eq!(loaded.worktree_width, 24);
        assert_eq!(loaded.agents_height, 5);
        assert_eq!(
            loaded.explorer_left_pane_width,
            Some(MINIMUM_EXPLORER_PANE_WIDTH)
        );
        assert_eq!(
            loaded.media_preview_protocol,
            MediaPreviewProtocol::Halfblocks
        );
    }
}
