use super::super::*;

impl App {
    pub(crate) fn handle_settings(&mut self, key: KeyEvent) {
        if self.discord_webhook_input.is_some() {
            self.handle_discord_webhook_input(key);
            return;
        }
        if self.opencode_model_input.is_some() {
            self.handle_opencode_model_input(key);
            return;
        }
        if self.shortcut_capture {
            if key.code == KeyCode::Esc {
                self.shortcut_capture = false;
                self.shortcut_error = None;
                return;
            }
            let Some(action) = Shortcuts::definitions(self.herdr_available())
                .nth(self.shortcut_selection)
                .map(|definition| definition.action)
            else {
                self.shortcut_capture = false;
                return;
            };
            match self
                .settings
                .shortcuts
                .set(action, KeyChord::from_event(key))
            {
                Ok(()) => {
                    self.shortcut_capture = false;
                    self.shortcut_error = None;
                    self.settings_changed();
                }
                Err(error) => self.shortcut_error = Some(error),
            }
            return;
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            let page = if key.code == KeyCode::BackTab {
                self.settings_page.previous()
            } else {
                self.settings_page.next()
            };
            self.set_settings_page(page);
            return;
        }
        if self.settings_page == SettingsPage::Shortcuts {
            self.handle_shortcut_settings(key);
            return;
        }
        if self.settings_page == SettingsPage::OpenCode {
            self.handle_opencode_settings(key);
            return;
        }
        if self.settings_page == SettingsPage::Discord {
            self.handle_discord_settings(key);
            return;
        }
        match key.code {
            KeyCode::Esc => self.close_settings(),
            _ if self
                .settings
                .shortcuts
                .matches(ShortcutAction::OpenSettings, key) =>
            {
                self.close_settings();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let settings = self.general_settings();
                let current = settings
                    .iter()
                    .position(|index| *index == self.settings_selection)
                    .unwrap_or_default();
                self.settings_selection = settings[(current + 1) % settings.len()];
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let settings = self.general_settings();
                let current = settings
                    .iter()
                    .position(|index| *index == self.settings_selection)
                    .unwrap_or_default();
                self.settings_selection = settings[(current + settings.len() - 1) % settings.len()];
            }
            KeyCode::Left | KeyCode::Char('-') if self.settings_selection == 1 => {
                self.change_fetch_interval(-1);
            }
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=')
                if self.settings_selection == 1 =>
            {
                self.change_fetch_interval(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(target) = SettingsHitTarget::from_general_index(self.settings_selection)
                {
                    self.activate_settings_target(target);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_opencode_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.opencode_selection = (self.opencode_selection + 1) % 2;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.opencode_selection = (self.opencode_selection + 1) % 2;
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.opencode_selection == 0 => {
                self.begin_opencode_model_input();
            }
            KeyCode::Left if self.opencode_selection == 1 => {
                self.change_opencode_reasoning(-1);
            }
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                if self.opencode_selection == 1 =>
            {
                self.change_opencode_reasoning(1);
            }
            KeyCode::Home if self.opencode_selection == 1 => {
                self.settings.opencode_reasoning = OpenCodeReasoning::Default;
                self.settings_changed();
            }
            KeyCode::End if self.opencode_selection == 1 => {
                self.settings.opencode_reasoning = OpenCodeReasoning::Max;
                self.settings_changed();
            }
            _ => {}
        }
    }

    pub(crate) fn handle_opencode_model_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.opencode_model_input = None;
                self.opencode_error = None;
            }
            KeyCode::Enter => {
                let model = self
                    .opencode_model_input
                    .as_deref()
                    .unwrap_or_default()
                    .trim();
                if valid_opencode_model(model) {
                    self.settings.opencode_model = model.to_owned();
                    self.opencode_model_input = None;
                    self.opencode_error = None;
                    self.settings_changed();
                } else {
                    self.opencode_error = Some(
                        "Enter a non-empty model ID without spaces, such as provider/model"
                            .to_owned(),
                    );
                }
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut self.opencode_model_input {
                    input.pop();
                }
                self.opencode_error = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = &mut self.opencode_model_input {
                    input.clear();
                }
                self.opencode_error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(input) = &mut self.opencode_model_input {
                    input.push(character);
                }
                self.opencode_error = None;
            }
            _ => {}
        }
    }

    pub(crate) fn begin_opencode_model_input(&mut self) {
        self.opencode_model_input = Some(self.settings.opencode_model.clone());
        self.opencode_error = None;
    }

    fn handle_discord_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.discord_selection = (self.discord_selection + 1) % 3;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.discord_selection = (self.discord_selection + 2) % 3;
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.discord_selection {
                0 => self.begin_discord_webhook_input(),
                1 => self.test_discord_webhook(),
                2 => self.remove_discord_webhook(),
                _ => unreachable!(),
            },
            _ => {}
        }
    }

    fn handle_discord_webhook_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.discord_webhook_input = None;
                self.discord_webhook_error = None;
            }
            KeyCode::Enter => self.save_discord_webhook(),
            _ => {
                if self
                    .discord_webhook_input
                    .as_mut()
                    .is_some_and(|input| input.handle_edit_key(key) != EditOutcome::Unhandled)
                {
                    self.discord_webhook_error = None;
                }
            }
        }
    }

    fn begin_discord_webhook_input(&mut self) {
        let mut input = TextInput::default();
        input.set(self.discord_webhook_url.clone().unwrap_or_default());
        input.focus();
        self.discord_webhook_input = Some(input);
        self.discord_webhook_error = None;
    }

    fn save_discord_webhook(&mut self) {
        let webhook_url = self
            .discord_webhook_input
            .as_ref()
            .map(|input| input.text().trim().to_owned())
            .unwrap_or_default();
        if !valid_discord_webhook_url(&webhook_url) {
            self.discord_webhook_error = Some("Enter a Discord HTTPS webhook URL".to_owned());
            return;
        }
        if let Err(error) = self.discord_webhook_store.save(Some(&webhook_url)) {
            self.discord_webhook_error = Some(format!("Could not save webhook: {error}"));
            return;
        }
        self.discord_webhook_url = Some(webhook_url.clone());
        self.discord_webhook_input = None;
        self.discord_webhook_error = None;
        match self.herdr.configure_discord_webhook(Some(webhook_url)) {
            Ok(()) => self.notice = Some("Discord webhook saved".to_owned()),
            Err(error) => self.notice = Some(format!("Could not configure Discord: {error}")),
        }
    }

    fn test_discord_webhook(&mut self) {
        self.discord_webhook_error = None;
        match self.herdr.test_discord_webhook() {
            Ok(()) => self.notice = Some("Sending Discord test message…".to_owned()),
            Err(error) => self.notice = Some(error),
        }
    }

    fn remove_discord_webhook(&mut self) {
        if let Err(error) = self.discord_webhook_store.save(None) {
            self.discord_webhook_error = Some(format!("Could not remove webhook: {error}"));
            return;
        }
        self.discord_webhook_url = None;
        self.discord_webhook_input = None;
        self.discord_webhook_error = None;
        match self.herdr.configure_discord_webhook(None) {
            Ok(()) => self.notice = Some("Discord webhook removed".to_owned()),
            Err(error) => self.notice = Some(format!("Could not configure Discord: {error}")),
        }
    }

    pub(crate) fn change_opencode_reasoning(&mut self, delta: isize) {
        self.settings.opencode_reasoning = self.settings.opencode_reasoning.next(delta);
        self.opencode_error = None;
        self.settings_changed();
    }

    pub(crate) fn open_settings(&mut self) {
        self.mode = Mode::Settings;
        self.settings_page = SettingsPage::General;
        self.reset_settings_input();
    }

    pub(crate) fn set_settings_page(&mut self, page: SettingsPage) {
        self.settings_page = page;
        self.reset_settings_input();
    }

    fn reset_settings_input(&mut self) {
        self.shortcut_capture = false;
        self.shortcut_error = None;
        self.opencode_model_input = None;
        self.opencode_error = None;
        self.discord_webhook_input = None;
        self.discord_webhook_error = None;
    }

    pub(crate) fn close_settings(&mut self) {
        self.mode = Mode::Normal;
        self.reset_settings_input();
    }

    pub(crate) fn activate_settings_target(&mut self, target: SettingsHitTarget) {
        if let Some(index) = target.general_index() {
            self.settings_selection = index;
        }
        match target {
            SettingsHitTarget::Overlay | SettingsHitTarget::FetchInterval => {}
            SettingsHitTarget::Page(page) => self.set_settings_page(page),
            SettingsHitTarget::Shortcut(action) => {
                if let Some(index) = Shortcuts::definitions(self.herdr_available())
                    .position(|definition| definition.action == action)
                {
                    self.shortcut_selection = index;
                    self.shortcut_capture = true;
                    self.shortcut_error = None;
                }
            }
            SettingsHitTarget::OpenCodeModel => {
                self.opencode_selection = 0;
                self.begin_opencode_model_input();
            }
            SettingsHitTarget::OpenCodeReasoning => {
                self.opencode_selection = 1;
                self.change_opencode_reasoning(1);
            }
            SettingsHitTarget::DiscordWebhook => {
                self.discord_selection = 0;
                self.begin_discord_webhook_input();
            }
            SettingsHitTarget::DiscordTest => {
                self.discord_selection = 1;
                self.test_discord_webhook();
            }
            SettingsHitTarget::DiscordRemove => {
                self.discord_selection = 2;
                self.remove_discord_webhook();
            }
            SettingsHitTarget::AutoFetch => self.toggle_auto_fetch(),
            SettingsHitTarget::FetchIntervalDown => self.change_fetch_interval(-1),
            SettingsHitTarget::FetchIntervalUp => self.change_fetch_interval(1),
            SettingsHitTarget::FormatOnSave => self.toggle_format_on_save(),
            SettingsHitTarget::CrossWorkspaceAgents => self.toggle_cross_workspace_agents(),
            SettingsHitTarget::AgentHarness => self.toggle_agent_harness(),
            SettingsHitTarget::AgentTime => self.toggle_agent_time_display(),
            SettingsHitTarget::ClearAgentTimings => self.clear_agent_timing_history(),
            SettingsHitTarget::MediaPreview => self.toggle_media_preview_protocol(),
            SettingsHitTarget::Editor => self.open_editor_setting(),
        }
    }

    pub(crate) fn handle_shortcut_settings(&mut self, key: KeyEvent) {
        let count = Shortcuts::definitions(self.herdr_available()).count();
        match key.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.shortcut_selection = (self.shortcut_selection + 1) % count;
                self.keep_shortcut_selection_visible();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.shortcut_selection = (self.shortcut_selection + count - 1) % count;
                self.keep_shortcut_selection_visible();
            }
            KeyCode::Home => {
                self.shortcut_selection = 0;
                self.keep_shortcut_selection_visible();
            }
            KeyCode::End => {
                self.shortcut_selection = count.saturating_sub(1);
                self.keep_shortcut_selection_visible();
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.shortcut_capture = true;
                self.shortcut_error = None;
            }
            KeyCode::Delete => {
                let action = Shortcuts::definitions(self.herdr_available())
                    .nth(self.shortcut_selection)
                    .map(|definition| definition.action);
                if action.is_some_and(|action| self.settings.shortcuts.reset(action)) {
                    self.settings_changed();
                }
                self.shortcut_error = None;
            }
            _ => {}
        }
    }

    pub(crate) fn keep_shortcut_selection_visible(&mut self) {
        let viewport = self.regions.settings_shortcut_rows().max(1);
        if self.shortcut_selection < self.shortcut_scroll {
            self.shortcut_scroll = self.shortcut_selection;
        } else if self.shortcut_selection >= self.shortcut_scroll + viewport {
            self.shortcut_scroll = self.shortcut_selection + 1 - viewport;
        }
    }

    pub(crate) fn toggle_auto_fetch(&mut self) {
        self.settings.auto_fetch = !self.settings.auto_fetch;
        self.settings_changed();
    }

    pub(crate) fn toggle_format_on_save(&mut self) {
        self.settings.format_on_save = !self.settings.format_on_save;
        self.settings_changed();
    }

    pub(crate) fn toggle_agent_harness(&mut self) {
        self.settings.show_agent_harness = !self.settings.show_agent_harness;
        self.settings_changed();
    }

    pub(crate) fn toggle_cross_workspace_agents(&mut self) {
        self.settings.cross_workspace_agents = !self.settings.cross_workspace_agents;
        self.herdr
            .set_cross_workspace_agents(self.settings.cross_workspace_agents);
        self.settings_changed();
    }

    pub(crate) fn toggle_agent_time_display(&mut self) {
        self.settings.agent_time_display = self.settings.agent_time_display.next();
        self.settings_changed();
    }

    pub(crate) fn clear_agent_timing_history(&mut self) {
        self.notice = Some(match self.herdr.clear_agent_timing_history() {
            Ok(()) => "Agent timing history cleared".to_owned(),
            Err(error) => error,
        });
    }

    pub(crate) fn toggle_media_preview_protocol(&mut self) {
        self.settings.media_preview_protocol = self.settings.media_preview_protocol.next();
        self.reset_media_presentation();
        self.settings_changed();
    }

    pub(crate) fn change_fetch_interval(&mut self, delta: i16) {
        self.settings.fetch_interval_minutes =
            (self.settings.fetch_interval_minutes as i16 + delta).clamp(1, 1440) as u16;
        self.settings_changed();
    }

    pub(crate) fn settings_changed(&mut self) {
        self.session
            .reset_fetch_deadline(self.settings.fetch_interval());
        self.persist_settings();
    }

    pub(crate) fn persist_settings(&mut self) {
        if let Err(error) = self.settings_store.save(&self.settings) {
            self.notice = Some(format!("Could not save settings: {error}"));
        }
    }
}
