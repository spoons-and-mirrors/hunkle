use super::super::*;

impl App {
    pub(crate) fn handle_settings(&mut self, key: KeyEvent) {
        if self.settings_state.discord_webhook_editor.is_some() {
            self.handle_discord_webhook_editor(key);
            return;
        }
        if self.settings_state.opencode_model_input.is_some() {
            self.handle_opencode_model_input(key);
            return;
        }
        if self.settings_state.shortcut_capture {
            if key.code == KeyCode::Esc {
                self.settings_state.shortcut_capture = false;
                self.settings_state.shortcut_error = None;
                return;
            }
            let Some(action) = Shortcuts::definitions(self.herdr_available())
                .nth(self.settings_state.shortcut_selection)
                .map(|definition| definition.action)
            else {
                self.settings_state.shortcut_capture = false;
                return;
            };
            match self
                .settings
                .shortcuts
                .set(action, KeyChord::from_event(key))
            {
                Ok(()) => {
                    self.settings_state.shortcut_capture = false;
                    self.settings_state.shortcut_error = None;
                    self.settings_changed();
                }
                Err(error) => self.settings_state.shortcut_error = Some(error),
            }
            return;
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            self.settings_state.cycle_page(key.code == KeyCode::BackTab);
            return;
        }
        if self.settings_state.page == SettingsPage::Shortcuts {
            self.handle_shortcut_settings(key);
            return;
        }
        if self.settings_state.page == SettingsPage::OpenCode {
            self.handle_opencode_settings(key);
            return;
        }
        if self.settings_state.page == SettingsPage::Discord {
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
                self.settings_state.move_general_selection(settings, 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let settings = self.general_settings();
                self.settings_state.move_general_selection(settings, -1);
            }
            KeyCode::Left | KeyCode::Char('-') if self.settings_state.selection == 1 => {
                self.change_fetch_interval(-1);
            }
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=')
                if self.settings_state.selection == 1 =>
            {
                self.change_fetch_interval(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(target) =
                    SettingsHitTarget::from_general_index(self.settings_state.selection)
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
                self.settings_state.move_opencode_selection();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_state.move_opencode_selection();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_state.opencode_selection == 0 => {
                self.begin_opencode_model_input();
            }
            KeyCode::Left if self.settings_state.opencode_selection == 1 => {
                self.change_opencode_reasoning(-1);
            }
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                if self.settings_state.opencode_selection == 1 =>
            {
                self.change_opencode_reasoning(1);
            }
            KeyCode::Home if self.settings_state.opencode_selection == 1 => {
                self.settings.opencode_reasoning = OpenCodeReasoning::Default;
                self.settings_changed();
            }
            KeyCode::End if self.settings_state.opencode_selection == 1 => {
                self.settings.opencode_reasoning = OpenCodeReasoning::Max;
                self.settings_changed();
            }
            _ => {}
        }
    }

    pub(crate) fn handle_opencode_model_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = &mut self.settings_state.opencode_model_input {
                    input.clear();
                }
                self.settings_state.opencode_error = None;
            }
            KeyCode::Esc => {
                self.settings_state.opencode_model_input = None;
                self.settings_state.opencode_error = None;
            }
            KeyCode::Enter => {
                let model = self
                    .settings_state
                    .opencode_model_input
                    .as_deref()
                    .unwrap_or_default()
                    .trim();
                if valid_opencode_model(model) {
                    self.settings.opencode_model = model.to_owned();
                    self.settings_state.opencode_model_input = None;
                    self.settings_state.opencode_error = None;
                    self.settings_changed();
                } else {
                    self.settings_state.opencode_error = Some(
                        "Enter a non-empty model ID without spaces, such as provider/model"
                            .to_owned(),
                    );
                }
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut self.settings_state.opencode_model_input {
                    input.pop();
                }
                self.settings_state.opencode_error = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = &mut self.settings_state.opencode_model_input {
                    input.clear();
                }
                self.settings_state.opencode_error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(input) = &mut self.settings_state.opencode_model_input {
                    input.push(character);
                }
                self.settings_state.opencode_error = None;
            }
            _ => {}
        }
    }

    pub(crate) fn begin_opencode_model_input(&mut self) {
        self.settings_state.opencode_model_input = Some(self.settings.opencode_model.clone());
        self.settings_state.opencode_error = None;
    }

    fn handle_discord_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_state.move_discord_selection(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_state.move_discord_selection(-1);
            }
            KeyCode::Left | KeyCode::Char('h') if self.settings_state.discord_selection == 0 => {
                if !self.discord_webhooks.is_empty() {
                    self.settings_state
                        .move_discord_webhook(-1, self.discord_webhooks.len());
                }
            }
            KeyCode::Right | KeyCode::Char('l') if self.settings_state.discord_selection == 0 => {
                if !self.discord_webhooks.is_empty() {
                    self.settings_state
                        .move_discord_webhook(1, self.discord_webhooks.len());
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.settings_state.discord_selection {
                0 => self.edit_selected_discord_webhook(),
                1 => self.begin_discord_webhook_editor(None),
                2 => self.test_discord_webhook(),
                3 => self.remove_discord_webhook(),
                _ => unreachable!(),
            },
            _ => {}
        }
    }

    fn handle_discord_webhook_editor(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.settings_state.discord_webhook_editor = None;
                self.settings_state.discord_webhook_error = None;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let editor = self.settings_state.discord_webhook_editor.as_mut().unwrap();
                let offset = if key.code == KeyCode::BackTab { 3 } else { 1 };
                editor.select((editor.field + offset) % 4);
            }
            KeyCode::Enter => {
                let field = self
                    .settings_state
                    .discord_webhook_editor
                    .as_ref()
                    .unwrap()
                    .field;
                if field == 3 {
                    self.save_discord_webhook();
                } else {
                    self.settings_state
                        .discord_webhook_editor
                        .as_mut()
                        .unwrap()
                        .select(field + 1);
                }
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.save_discord_webhook();
            }
            _ => {
                if self
                    .settings_state
                    .discord_webhook_editor
                    .as_mut()
                    .is_some_and(|editor| {
                        editor.active_input_mut().handle_edit_key(key) != EditOutcome::Unhandled
                    })
                {
                    self.settings_state.discord_webhook_error = None;
                }
            }
        }
    }

    fn begin_discord_webhook_editor(&mut self, webhook: Option<DiscordWebhookConfig>) {
        self.settings_state.discord_webhook_editor =
            Some(DiscordWebhookEditor::new(webhook.as_ref()));
        self.settings_state.discord_webhook_error = None;
    }

    fn edit_selected_discord_webhook(&mut self) {
        let webhook = self
            .discord_webhooks
            .get(self.settings_state.discord_webhook_index)
            .cloned();
        self.begin_discord_webhook_editor(webhook);
    }

    fn save_discord_webhook(&mut self) {
        let webhook = match self
            .settings_state
            .discord_webhook_editor
            .as_ref()
            .unwrap()
            .config()
        {
            Ok(webhook) => webhook,
            Err(error) => {
                self.settings_state.discord_webhook_error = Some(error);
                return;
            }
        };
        let original_id = self
            .settings_state
            .discord_webhook_editor
            .as_ref()
            .and_then(|editor| editor.original_id.as_deref());
        let mut webhooks = self.discord_webhooks.clone();
        let index =
            original_id.and_then(|id| webhooks.iter().position(|existing| existing.id == id));
        if let Some(index) = index {
            webhooks[index] = webhook;
            self.settings_state.discord_webhook_index = index;
        } else {
            webhooks.push(webhook);
            self.settings_state.discord_webhook_index = webhooks.len() - 1;
        }
        if let Err(error) = self.discord_webhook_store.save(&webhooks) {
            self.settings_state.discord_webhook_error =
                Some(format!("Could not save webhook: {error}"));
            return;
        }
        self.discord_webhooks = webhooks;
        self.settings_state.discord_webhook_editor = None;
        self.settings_state.discord_webhook_error = None;
        match self
            .scheduled_tasks
            .configure_discord_webhooks(self.discord_webhooks.clone())
        {
            Ok(()) => self.notice = Some("Discord webhook saved".to_owned()),
            Err(error) => self.notice = Some(format!("Could not configure Discord: {error}")),
        }
    }

    fn test_discord_webhook(&mut self) {
        self.settings_state.discord_webhook_error = None;
        let Some(webhook) = self
            .discord_webhooks
            .get(self.settings_state.discord_webhook_index)
        else {
            self.notice = Some("Add a Discord webhook first".to_owned());
            return;
        };
        match self
            .scheduled_tasks
            .test_discord_webhook(webhook.id.clone())
        {
            Ok(()) => self.notice = Some("Sending Discord test message…".to_owned()),
            Err(error) => self.notice = Some(error),
        }
    }

    fn remove_discord_webhook(&mut self) {
        if self.discord_webhooks.is_empty() {
            return;
        }
        let webhook = &self.discord_webhooks[self
            .settings_state
            .discord_webhook_index
            .min(self.discord_webhooks.len() - 1)];
        if let Some(task) = self
            .scheduled_tasks
            .tasks()
            .iter()
            .find(|task| task.discord_webhook_id == webhook.id)
        {
            self.notice = Some(format!(
                "Set Discord to Off for scheduled task `{}` before removing this webhook",
                task.title
            ));
            return;
        }
        let mut webhooks = self.discord_webhooks.clone();
        webhooks.remove(
            self.settings_state
                .discord_webhook_index
                .min(webhooks.len() - 1),
        );
        if let Err(error) = self.discord_webhook_store.save(&webhooks) {
            self.settings_state.discord_webhook_error =
                Some(format!("Could not remove webhook: {error}"));
            return;
        }
        self.discord_webhooks = webhooks;
        self.settings_state.discord_webhook_index = self
            .settings_state
            .discord_webhook_index
            .min(self.discord_webhooks.len().saturating_sub(1));
        self.settings_state.discord_webhook_editor = None;
        self.settings_state.discord_webhook_error = None;
        match self
            .scheduled_tasks
            .configure_discord_webhooks(self.discord_webhooks.clone())
        {
            Ok(()) => self.notice = Some("Discord webhook removed".to_owned()),
            Err(error) => self.notice = Some(format!("Could not configure Discord: {error}")),
        }
    }

    pub(crate) fn change_opencode_reasoning(&mut self, delta: isize) {
        self.settings.opencode_reasoning = self.settings.opencode_reasoning.next(delta);
        self.settings_state.opencode_error = None;
        self.settings_changed();
    }

    pub(crate) fn open_settings(&mut self) {
        self.mode = Mode::Settings;
        self.settings_state.open();
    }

    pub(crate) fn close_settings(&mut self) {
        self.mode = Mode::Normal;
        self.settings_state.reset_input();
    }

    pub(crate) fn activate_settings_target(&mut self, target: SettingsHitTarget) {
        let shortcut_index = match target {
            SettingsHitTarget::Shortcut(action) => Shortcuts::definitions(self.herdr_available())
                .position(|definition| definition.action == action),
            _ => None,
        };
        let effect = self.settings_state.activate_target(target, shortcut_index);
        self.apply_settings_effect(effect);
    }

    fn apply_settings_effect(&mut self, effect: SettingsEffect) {
        match effect {
            SettingsEffect::Handled => {}
            SettingsEffect::BeginOpenCodeModel => self.begin_opencode_model_input(),
            SettingsEffect::ChangeOpenCodeReasoning => self.change_opencode_reasoning(1),
            SettingsEffect::EditDiscordWebhook => self.edit_selected_discord_webhook(),
            SettingsEffect::AddDiscordWebhook => self.begin_discord_webhook_editor(None),
            SettingsEffect::SaveDiscordWebhook => self.save_discord_webhook(),
            SettingsEffect::TestDiscordWebhook => self.test_discord_webhook(),
            SettingsEffect::RemoveDiscordWebhook => self.remove_discord_webhook(),
            SettingsEffect::ToggleAutoFetch => self.toggle_auto_fetch(),
            SettingsEffect::DecreaseFetchInterval => self.change_fetch_interval(-1),
            SettingsEffect::IncreaseFetchInterval => self.change_fetch_interval(1),
            SettingsEffect::ToggleFormatOnSave => self.toggle_format_on_save(),
            SettingsEffect::ToggleCrossWorkspaceAgents => self.toggle_cross_workspace_agents(),
            SettingsEffect::ToggleAgentHarness => self.toggle_agent_harness(),
            SettingsEffect::ToggleAgentCardClick => self.toggle_agent_card_click_action(),
            SettingsEffect::ToggleAgentTime => self.toggle_agent_time_display(),
            SettingsEffect::ClearAgentTimings => self.clear_agent_timing_history(),
            SettingsEffect::ToggleMediaPreview => self.toggle_media_preview_protocol(),
            SettingsEffect::OpenEditor => self.open_editor_setting(),
        }
    }

    pub(crate) fn handle_shortcut_settings(&mut self, key: KeyEvent) {
        let count = Shortcuts::definitions(self.herdr_available()).count();
        match key.code {
            KeyCode::Esc => self.close_settings(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.settings_state.move_shortcut_selection(1, count);
                self.keep_shortcut_selection_visible();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_state.move_shortcut_selection(-1, count);
                self.keep_shortcut_selection_visible();
            }
            KeyCode::Home => {
                self.settings_state.select_shortcut_boundary(false, count);
                self.keep_shortcut_selection_visible();
            }
            KeyCode::End => {
                self.settings_state.select_shortcut_boundary(true, count);
                self.keep_shortcut_selection_visible();
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.settings_state.begin_shortcut_capture();
            }
            KeyCode::Delete => {
                let action = Shortcuts::definitions(self.herdr_available())
                    .nth(self.settings_state.shortcut_selection)
                    .map(|definition| definition.action);
                if action.is_some_and(|action| self.settings.shortcuts.reset(action)) {
                    self.settings_changed();
                }
                self.settings_state.shortcut_error = None;
            }
            _ => {}
        }
    }

    pub(crate) fn keep_shortcut_selection_visible(&mut self) {
        let count = Shortcuts::definitions(self.herdr_available()).count();
        let viewport = self
            .regions
            .scroll_state(&ScrollTarget::SettingsShortcuts)
            .map_or(1, |state| count.saturating_sub(state.maximum).max(1));
        self.settings_state.keep_shortcut_visible(viewport);
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

    pub(crate) fn toggle_agent_card_click_action(&mut self) {
        self.settings.agent_card_click_action = self.settings.agent_card_click_action.toggled();
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
