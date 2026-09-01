use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    filesystem::atomic_write,
    media::{MediaPreviewProtocol, SixelQuality},
};

use super::{GraphColumn, Shortcuts, explorer::MINIMUM_EXPLORER_PANE_WIDTH};

fn wrapped_index(current: usize, count: usize, delta: isize) -> usize {
    if count == 0 {
        return 0;
    }
    if delta >= 0 {
        (current + delta as usize % count) % count
    } else {
        (current + count - delta.unsigned_abs() % count) % count
    }
}

#[derive(Debug)]
pub(crate) struct SettingsState {
    pub(crate) selection: usize,
    pub(crate) page: super::SettingsPage,
    pub(crate) shortcut_selection: usize,
    pub(crate) shortcut_scroll: usize,
    pub(crate) shortcut_capture: bool,
    pub(crate) shortcut_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsEffect {
    Handled,
    ToggleAutoFetch,
    DecreaseFetchInterval,
    IncreaseFetchInterval,
    ToggleFormatOnSave,
    ToggleMediaPreview,
    ToggleSixelQuality,
    OpenEditor,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            selection: 0,
            page: super::SettingsPage::General,
            shortcut_selection: 0,
            shortcut_scroll: 0,
            shortcut_capture: false,
            shortcut_error: None,
        }
    }
}

impl SettingsState {
    pub(crate) fn open(&mut self) {
        self.page = super::SettingsPage::General;
        self.reset_input();
    }

    pub(crate) fn set_page(&mut self, page: super::SettingsPage) {
        self.page = page;
        self.reset_input();
    }

    pub(crate) fn reset_input(&mut self) {
        self.shortcut_capture = false;
        self.shortcut_error = None;
    }

    pub(crate) fn keep_shortcut_visible(&mut self, viewport: usize) {
        let viewport = viewport.max(1);
        if self.shortcut_selection < self.shortcut_scroll {
            self.shortcut_scroll = self.shortcut_selection;
        } else if self.shortcut_selection >= self.shortcut_scroll + viewport {
            self.shortcut_scroll = self.shortcut_selection + 1 - viewport;
        }
    }

    pub(crate) fn cycle_page(&mut self, backward: bool) {
        let page = if backward {
            self.page.previous()
        } else {
            self.page.next()
        };
        self.set_page(page);
    }

    pub(crate) fn move_general_selection(&mut self, settings: &[usize], delta: isize) {
        let current = settings
            .iter()
            .position(|index| *index == self.selection)
            .unwrap_or_default();
        let next = wrapped_index(current, settings.len(), delta);
        self.selection = settings[next];
    }

    pub(crate) fn move_shortcut_selection(&mut self, delta: isize, count: usize) {
        self.shortcut_selection = wrapped_index(self.shortcut_selection, count, delta);
    }

    pub(crate) fn select_shortcut_boundary(&mut self, end: bool, count: usize) {
        self.shortcut_selection = if end { count.saturating_sub(1) } else { 0 };
    }

    pub(crate) fn begin_shortcut_capture(&mut self) {
        self.shortcut_capture = true;
        self.shortcut_error = None;
    }

    pub(crate) fn activate_target(
        &mut self,
        target: super::SettingsHitTarget,
        shortcut_index: Option<usize>,
    ) -> SettingsEffect {
        if let Some(index) = target.general_index() {
            self.selection = index;
        }
        match target {
            super::SettingsHitTarget::Overlay | super::SettingsHitTarget::FetchInterval => {
                SettingsEffect::Handled
            }
            super::SettingsHitTarget::Page(page) => {
                self.set_page(page);
                SettingsEffect::Handled
            }
            super::SettingsHitTarget::Shortcut(_) => {
                if let Some(index) = shortcut_index {
                    self.shortcut_selection = index;
                    self.begin_shortcut_capture();
                }
                SettingsEffect::Handled
            }
            super::SettingsHitTarget::AutoFetch => SettingsEffect::ToggleAutoFetch,
            super::SettingsHitTarget::FetchIntervalDown => SettingsEffect::DecreaseFetchInterval,
            super::SettingsHitTarget::FetchIntervalUp => SettingsEffect::IncreaseFetchInterval,
            super::SettingsHitTarget::FormatOnSave => SettingsEffect::ToggleFormatOnSave,
            super::SettingsHitTarget::MediaPreview => SettingsEffect::ToggleMediaPreview,
            super::SettingsHitTarget::SixelQuality => SettingsEffect::ToggleSixelQuality,
            super::SettingsHitTarget::Editor => SettingsEffect::OpenEditor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub auto_fetch: bool,
    pub fetch_interval_minutes: u16,
    pub format_on_save: bool,
    pub worktree_width: u16,
    pub graph_lane_width: u16,
    pub graph_description_width: u16,
    pub graph_changes_width: u16,
    pub graph_date_width: u16,
    pub graph_author_width: u16,
    pub graph_commit_width: u16,
    pub explorer_left_pane_width: Option<u16>,
    pub editor_command: Option<String>,
    pub media_preview_protocol: MediaPreviewProtocol,
    pub sixel_quality: SixelQuality,
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
            graph_lane_width: 0,
            graph_description_width: 0,
            graph_changes_width: 12,
            graph_date_width: 12,
            graph_author_width: 16,
            graph_commit_width: 7,
            explorer_left_pane_width: None,
            editor_command: None,
            media_preview_protocol: MediaPreviewProtocol::Auto,
            sixel_quality: SixelQuality::Fast,
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

    #[cfg(not(test))]
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
            "auto_fetch={}\nfetch_interval_minutes={}\nformat_on_save={}\nworktree_width={}\ngraph_lane_width={}\ngraph_description_width={}\ngraph_changes_width={}\ngraph_date_width={}\ngraph_author_width={}\ngraph_commit_width={}\nexplorer_left_pane_width={}\neditor_command={}\nmedia_preview_protocol={}\nsixel_quality={}\n",
            settings.auto_fetch,
            settings.fetch_interval_minutes,
            settings.format_on_save,
            settings.worktree_width,
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
            settings.media_preview_protocol.as_str(),
            settings.sixel_quality.as_str(),
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
        let value = value.trim();
        if let Some(id) = key.strip_prefix("shortcut.") {
            settings.shortcuts.load_override(id, value);
            continue;
        }
        match key {
            "auto_fetch" => settings.auto_fetch = value == "true",
            "fetch_interval_minutes" => {
                if let Ok(minutes) = value.parse::<u16>() {
                    settings.fetch_interval_minutes = minutes.clamp(1, 1440);
                }
            }
            "format_on_save" => settings.format_on_save = value == "true",
            "worktree_width" => {
                if let Ok(width) = value.parse::<u16>() {
                    settings.worktree_width = width.clamp(24, 4096);
                }
            }
            "graph_changes_width" => set_graph_width(&mut settings, GraphColumn::Changes, value),
            "graph_lane_width" => {
                set_optional_graph_width(&mut settings.graph_lane_width, GraphColumn::Graph, value)
            }
            "graph_description_width" => set_optional_graph_width(
                &mut settings.graph_description_width,
                GraphColumn::Description,
                value,
            ),
            "graph_date_width" => set_graph_width(&mut settings, GraphColumn::Date, value),
            "graph_author_width" => set_graph_width(&mut settings, GraphColumn::Author, value),
            "graph_commit_width" => set_graph_width(&mut settings, GraphColumn::Commit, value),
            "explorer_left_pane_width" => {
                settings.explorer_left_pane_width = value
                    .parse::<u16>()
                    .ok()
                    .map(|width| width.clamp(MINIMUM_EXPLORER_PANE_WIDTH, 4096));
            }
            "editor_command" => {
                settings.editor_command = (!value.is_empty()).then(|| value.to_owned());
            }
            "media_preview_protocol" => {
                settings.media_preview_protocol = match value {
                    "auto" => MediaPreviewProtocol::Auto,
                    "kitty" => MediaPreviewProtocol::Kitty,
                    "iterm2" => MediaPreviewProtocol::Iterm2,
                    "sixel" => MediaPreviewProtocol::Sixel,
                    _ => MediaPreviewProtocol::Halfblocks,
                };
            }
            "sixel_quality" => {
                settings.sixel_quality = if value == "quality" {
                    SixelQuality::Quality
                } else {
                    SixelQuality::Fast
                };
            }
            _ => {}
        }
    }
    settings
}

fn set_graph_width(settings: &mut Settings, column: GraphColumn, value: &str) {
    if let Ok(width) = value.parse::<u16>() {
        settings.set_graph_column_width(column, width);
    }
}

fn set_optional_graph_width(target: &mut u16, column: GraphColumn, value: &str) {
    if let Ok(width) = value.parse::<u16>() {
        *target = if width == 0 {
            0
        } else {
            width.clamp(column.minimum_width(), 80)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{KeyChord, SettingsHitTarget, SettingsPage, ShortcutAction};

    #[test]
    fn settings_state_owns_navigation_and_page_resets() {
        let mut state = SettingsState::default();
        state.move_general_selection(&[0, 2, 4], -1);
        assert_eq!(state.selection, 4);
        state.move_shortcut_selection(-1, 5);
        assert_eq!(state.shortcut_selection, 4);
        state.shortcut_error = Some("conflict".to_owned());

        let effect = state.activate_target(SettingsHitTarget::Page(SettingsPage::Shortcuts), None);
        assert_eq!(effect, SettingsEffect::Handled);
        assert_eq!(state.page, SettingsPage::Shortcuts);
        assert!(state.shortcut_error.is_none());
    }

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
            graph_lane_width: 12,
            graph_description_width: 31,
            graph_changes_width: 13,
            graph_date_width: 18,
            graph_author_width: 21,
            graph_commit_width: 9,
            explorer_left_pane_width: Some(47),
            editor_command: Some("code --wait".to_owned()),
            media_preview_protocol: MediaPreviewProtocol::Sixel,
            sixel_quality: SixelQuality::Quality,
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

        fs::write(
            path,
            "auto_fetch=true\nfetch_interval_minutes=0\nworktree_width=5\nexplorer_left_pane_width=2\nmedia_preview_protocol=unknown\nsixel_quality=unknown\n",
        )
        .unwrap();
        let loaded = store.load();
        assert!(loaded.format_on_save);
        assert_eq!(loaded.fetch_interval_minutes, 1);
        assert_eq!(loaded.worktree_width, 24);
        assert_eq!(
            loaded.explorer_left_pane_width,
            Some(MINIMUM_EXPLORER_PANE_WIDTH)
        );
        assert_eq!(
            loaded.media_preview_protocol,
            MediaPreviewProtocol::Halfblocks
        );
        assert_eq!(loaded.sixel_quality, SixelQuality::Fast);
    }
}
