use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    filesystem::{atomic_write, atomic_write_private},
    media::MediaPreviewProtocol,
};

use super::{GraphColumn, Shortcuts, TextInput, explorer::MINIMUM_EXPLORER_PANE_WIDTH};

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
    pub(crate) opencode_selection: usize,
    pub(crate) opencode_model_input: Option<String>,
    pub(crate) opencode_error: Option<String>,
    pub(crate) discord_selection: usize,
    pub(crate) discord_webhook_index: usize,
    pub(crate) discord_webhook_editor: Option<DiscordWebhookEditor>,
    pub(crate) discord_webhook_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsEffect {
    Handled,
    BeginOpenCodeModel,
    ChangeOpenCodeReasoning,
    EditDiscordWebhook,
    AddDiscordWebhook,
    SaveDiscordWebhook,
    TestDiscordWebhook,
    RemoveDiscordWebhook,
    ToggleAutoFetch,
    DecreaseFetchInterval,
    IncreaseFetchInterval,
    ToggleFormatOnSave,
    ToggleCrossWorkspaceAgents,
    ToggleAgentHarness,
    ToggleAgentCardClick,
    ToggleAgentTime,
    ClearAgentTimings,
    ToggleMediaPreview,
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
            opencode_selection: 0,
            opencode_model_input: None,
            opencode_error: None,
            discord_selection: 0,
            discord_webhook_index: 0,
            discord_webhook_editor: None,
            discord_webhook_error: None,
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
        self.opencode_model_input = None;
        self.opencode_error = None;
        self.discord_webhook_editor = None;
        self.discord_webhook_error = None;
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

    pub(crate) fn move_opencode_selection(&mut self) {
        self.opencode_selection = (self.opencode_selection + 1) % 2;
    }

    pub(crate) fn move_discord_selection(&mut self, delta: isize) {
        self.discord_selection = wrapped_index(self.discord_selection, 4, delta);
    }

    pub(crate) fn move_discord_webhook(&mut self, delta: isize, count: usize) {
        if count > 0 {
            self.discord_webhook_index = wrapped_index(self.discord_webhook_index, count, delta);
        }
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
                    self.shortcut_capture = true;
                    self.shortcut_error = None;
                }
                SettingsEffect::Handled
            }
            super::SettingsHitTarget::OpenCodeModel => {
                self.opencode_selection = 0;
                SettingsEffect::BeginOpenCodeModel
            }
            super::SettingsHitTarget::OpenCodeReasoning => {
                self.opencode_selection = 1;
                SettingsEffect::ChangeOpenCodeReasoning
            }
            super::SettingsHitTarget::DiscordWebhook => {
                self.discord_selection = 0;
                SettingsEffect::EditDiscordWebhook
            }
            super::SettingsHitTarget::DiscordAdd => {
                self.discord_selection = 1;
                SettingsEffect::AddDiscordWebhook
            }
            super::SettingsHitTarget::DiscordField(field) => {
                if let Some(editor) = self.discord_webhook_editor.as_mut() {
                    editor.select(field);
                }
                SettingsEffect::Handled
            }
            super::SettingsHitTarget::DiscordSave => SettingsEffect::SaveDiscordWebhook,
            super::SettingsHitTarget::DiscordCancel => {
                self.discord_webhook_editor = None;
                self.discord_webhook_error = None;
                SettingsEffect::Handled
            }
            super::SettingsHitTarget::DiscordTest => {
                self.discord_selection = 2;
                SettingsEffect::TestDiscordWebhook
            }
            super::SettingsHitTarget::DiscordRemove => {
                self.discord_selection = 3;
                SettingsEffect::RemoveDiscordWebhook
            }
            super::SettingsHitTarget::AutoFetch => SettingsEffect::ToggleAutoFetch,
            super::SettingsHitTarget::FetchIntervalDown => SettingsEffect::DecreaseFetchInterval,
            super::SettingsHitTarget::FetchIntervalUp => SettingsEffect::IncreaseFetchInterval,
            super::SettingsHitTarget::FormatOnSave => SettingsEffect::ToggleFormatOnSave,
            super::SettingsHitTarget::CrossWorkspaceAgents => {
                SettingsEffect::ToggleCrossWorkspaceAgents
            }
            super::SettingsHitTarget::AgentHarness => SettingsEffect::ToggleAgentHarness,
            super::SettingsHitTarget::AgentCardClick => SettingsEffect::ToggleAgentCardClick,
            super::SettingsHitTarget::AgentTime => SettingsEffect::ToggleAgentTime,
            super::SettingsHitTarget::ClearAgentTimings => SettingsEffect::ClearAgentTimings,
            super::SettingsHitTarget::MediaPreview => SettingsEffect::ToggleMediaPreview,
            super::SettingsHitTarget::Editor => SettingsEffect::OpenEditor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTimeDisplay {
    LatestLoop,
    AgentTotal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardClickAction {
    ChangeLayout,
    OpenPreview,
}

impl AgentCardClickAction {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::ChangeLayout => Self::OpenPreview,
            Self::OpenPreview => Self::ChangeLayout,
        }
    }

    pub(crate) fn opens_preview(self, control: bool) -> bool {
        control == (self == Self::ChangeLayout)
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ChangeLayout => "layout",
            Self::OpenPreview => "preview",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ChangeLayout => "Layout · Ctrl preview",
            Self::OpenPreview => "Preview · Ctrl layout",
        }
    }
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
    pub cross_workspace_agents: bool,
    pub show_agent_harness: bool,
    pub agent_card_click_action: AgentCardClickAction,
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
            cross_workspace_agents: false,
            show_agent_harness: false,
            agent_card_click_action: AgentCardClickAction::ChangeLayout,
            agent_time_display: AgentTimeDisplay::LatestLoop,
            agents_height: 7,
            graph_lane_width: 0,
            graph_description_width: 0,
            graph_changes_width: 12,
            graph_date_width: 12,
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

pub(crate) struct DiscordWebhookStore {
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DiscordWebhookConfig {
    pub(crate) id: String,
    pub(crate) server: String,
    pub(crate) channel: String,
    pub(crate) webhook_name: String,
    pub(crate) url: String,
}

#[derive(Debug)]
pub(crate) struct DiscordWebhookEditor {
    pub(crate) server: TextInput,
    pub(crate) channel: TextInput,
    pub(crate) webhook_name: TextInput,
    pub(crate) url: TextInput,
    pub(crate) field: usize,
    pub(crate) original_id: Option<String>,
}

impl DiscordWebhookEditor {
    pub(crate) fn new(webhook: Option<&DiscordWebhookConfig>) -> Self {
        let mut editor = Self {
            server: TextInput::default(),
            channel: TextInput::default(),
            webhook_name: TextInput::default(),
            url: TextInput::default(),
            field: 0,
            original_id: webhook.map(|webhook| webhook.id.clone()),
        };
        if let Some(webhook) = webhook {
            editor.server.set(&webhook.server);
            editor.channel.set(&webhook.channel);
            editor.webhook_name.set(&webhook.webhook_name);
            editor.url.set(&webhook.url);
        }
        editor.active_input_mut().focus();
        editor
    }

    pub(crate) fn active_input_mut(&mut self) -> &mut TextInput {
        match self.field {
            0 => &mut self.server,
            1 => &mut self.channel,
            2 => &mut self.webhook_name,
            3 => &mut self.url,
            _ => unreachable!(),
        }
    }

    pub(crate) fn select(&mut self, field: usize) {
        self.field = field.min(3);
        self.active_input_mut().focus();
    }

    pub(crate) fn config(&self) -> Result<DiscordWebhookConfig, String> {
        let url = self.url.text().trim().to_owned();
        let id = self
            .original_id
            .clone()
            .unwrap_or_else(|| discord_webhook_id(&url).unwrap_or_default().to_owned());
        let webhook = DiscordWebhookConfig {
            id,
            server: self.server.text().trim().to_owned(),
            channel: self
                .channel
                .text()
                .trim()
                .trim_start_matches('#')
                .to_owned(),
            webhook_name: self.webhook_name.text().trim().to_owned(),
            url,
        };
        validate_discord_webhooks(std::slice::from_ref(&webhook))?;
        Ok(webhook)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DiscordWebhookFile {
    webhooks: Vec<DiscordWebhookConfig>,
}

impl DiscordWebhookStore {
    pub(crate) fn new(config_dir: Option<&Path>) -> Self {
        Self {
            path: config_dir.map(|path| path.join("discord-webhook")),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub(crate) fn load(&self) -> std::io::Result<Vec<DiscordWebhookConfig>> {
        let Some(path) = self.path.as_deref() else {
            return Ok(Vec::new());
        };
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let contents = contents.trim();
        let webhooks = if valid_discord_webhook_url(contents) {
            vec![DiscordWebhookConfig {
                id: discord_webhook_id(contents).unwrap_or_default().to_owned(),
                server: "Unknown server".to_owned(),
                channel: "Unknown channel".to_owned(),
                webhook_name: "Unnamed webhook".to_owned(),
                url: contents.to_owned(),
            }]
        } else {
            serde_json::from_str::<DiscordWebhookFile>(contents)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
                .webhooks
        };
        validate_discord_webhooks(&webhooks)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        Ok(webhooks)
    }

    pub(crate) fn save(&self, webhooks: &[DiscordWebhookConfig]) -> std::io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if webhooks.is_empty() {
            return match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
        }
        validate_discord_webhooks(webhooks)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_vec_pretty(&DiscordWebhookFile {
            webhooks: webhooks.to_vec(),
        })
        .map_err(std::io::Error::other)?;
        atomic_write_private(path, &contents)
    }
}

fn validate_discord_webhooks(webhooks: &[DiscordWebhookConfig]) -> Result<(), String> {
    for (index, webhook) in webhooks.iter().enumerate() {
        for (label, value) in [
            ("server", webhook.server.trim()),
            ("channel", webhook.channel.trim()),
            ("webhook name", webhook.webhook_name.trim()),
        ] {
            if value.is_empty() || value.len() > 64 || value.chars().any(char::is_control) {
                return Err(format!("Discord {label} must be 1-64 characters"));
            }
        }
        if !valid_discord_webhook_url(&webhook.url) {
            return Err(format!(
                "Discord webhook URL for `{}` is invalid",
                webhook.webhook_name
            ));
        }
        if webhook.id.is_empty() || !webhook.id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("Discord webhook ID is invalid".to_owned());
        }
        if webhooks[..index]
            .iter()
            .any(|existing| existing.id == webhook.id)
        {
            return Err(format!(
                "Discord webhook `{}` is duplicated",
                webhook.webhook_name
            ));
        }
    }
    Ok(())
}

pub(crate) fn valid_discord_webhook_url(value: &str) -> bool {
    let value = value.trim();
    if value.chars().any(char::is_whitespace) {
        return false;
    }
    let path = value
        .strip_prefix("https://discord.com/api/webhooks/")
        .or_else(|| value.strip_prefix("https://discordapp.com/api/webhooks/"));
    let Some((id, token)) = path.and_then(|path| path.split_once('/')) else {
        return false;
    };
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) && !token.is_empty()
}

pub(crate) fn discord_webhook_id(value: &str) -> Option<&str> {
    value
        .trim()
        .strip_prefix("https://discord.com/api/webhooks/")
        .or_else(|| {
            value
                .trim()
                .strip_prefix("https://discordapp.com/api/webhooks/")
        })
        .and_then(|path| path.split_once('/'))
        .map(|(id, _)| id)
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
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
            "auto_fetch={}\nfetch_interval_minutes={}\nformat_on_save={}\nworktree_width={}\ncross_workspace_agents={}\nshow_agent_harness={}\nagent_card_click_action={}\nagent_time_display={}\nagents_height={}\ngraph_lane_width={}\ngraph_description_width={}\ngraph_changes_width={}\ngraph_date_width={}\ngraph_author_width={}\ngraph_commit_width={}\nexplorer_left_pane_width={}\neditor_command={}\nopencode_model={}\nopencode_reasoning={}\nmedia_preview_protocol={}\n",
            settings.auto_fetch,
            settings.fetch_interval_minutes,
            settings.format_on_save,
            settings.worktree_width,
            settings.cross_workspace_agents,
            settings.show_agent_harness,
            settings.agent_card_click_action.as_str(),
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
            "cross_workspace_agents" => {
                settings.cross_workspace_agents = value.trim() == "true";
            }
            "show_agent_harness" => {
                settings.show_agent_harness = value.trim() == "true";
            }
            "agent_card_click_action" => {
                settings.agent_card_click_action = match value.trim() {
                    "preview" => AgentCardClickAction::OpenPreview,
                    _ => AgentCardClickAction::ChangeLayout,
                };
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
    use crate::app::{KeyChord, SettingsHitTarget, SettingsPage, ShortcutAction};

    #[test]
    fn settings_state_owns_navigation_and_page_resets() {
        let mut state = SettingsState::default();
        state.move_general_selection(&[0, 2, 4], -1);
        assert_eq!(state.selection, 4);
        state.move_shortcut_selection(-1, 5);
        assert_eq!(state.shortcut_selection, 4);
        state.opencode_model_input = Some("provider/model".to_owned());

        let effect = state.activate_target(SettingsHitTarget::Page(SettingsPage::Discord), None);

        assert_eq!(effect, SettingsEffect::Handled);
        assert_eq!(state.page, SettingsPage::Discord);
        assert!(state.opencode_model_input.is_none());
    }

    #[test]
    fn discord_webhook_store_validates_and_removes_the_secret() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/discord-webhook");
        let store = DiscordWebhookStore::at(path.clone());
        let webhook = "https://discord.com/api/webhooks/123456/token";

        assert!(store.load().unwrap().is_empty());
        let config = DiscordWebhookConfig {
            id: "123456".to_owned(),
            server: "Hunkle".to_owned(),
            channel: "reports".to_owned(),
            webhook_name: "Scheduler".to_owned(),
            url: webhook.to_owned(),
        };
        let second = DiscordWebhookConfig {
            id: "654321".to_owned(),
            server: "Hunkle".to_owned(),
            channel: "reports".to_owned(),
            webhook_name: "Deployments".to_owned(),
            url: "https://discord.com/api/webhooks/654321/other-token".to_owned(),
        };
        store.save(&[config.clone(), second.clone()]).unwrap();
        assert_eq!(store.load().unwrap(), vec![config, second]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        store.save(&[]).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn discord_webhook_store_migrates_the_single_url_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("discord-webhook");
        let store = DiscordWebhookStore::at(path.clone());
        let webhook = "https://discord.com/api/webhooks/123456/token";
        atomic_write_private(&path, webhook.as_bytes()).unwrap();

        let loaded = store.load().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "123456");
        assert_eq!(loaded[0].server, "Unknown server");
        assert_eq!(loaded[0].channel, "Unknown channel");
        assert_eq!(loaded[0].webhook_name, "Unnamed webhook");
        assert_eq!(loaded[0].url, webhook);
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
            cross_workspace_agents: true,
            show_agent_harness: true,
            agent_card_click_action: AgentCardClickAction::OpenPreview,
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

    #[test]
    fn agent_card_click_action_keeps_control_as_the_inverse() {
        assert!(!AgentCardClickAction::ChangeLayout.opens_preview(false));
        assert!(AgentCardClickAction::ChangeLayout.opens_preview(true));
        assert!(AgentCardClickAction::OpenPreview.opens_preview(false));
        assert!(!AgentCardClickAction::OpenPreview.opens_preview(true));
    }
}
