use super::super::*;

impl App {
    pub(crate) fn handle_settings(&mut self, key: KeyEvent) {
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
            let action = Shortcuts::definitions()[self.shortcut_selection].action;
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
            self.settings_page = if key.code == KeyCode::BackTab {
                self.settings_page.previous()
            } else {
                self.settings_page.next()
            };
            self.shortcut_error = None;
            self.opencode_error = None;
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
                self.settings_selection = (self.settings_selection + 1) % SETTINGS_ROW_COUNT;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.settings_selection =
                    (self.settings_selection + SETTINGS_ROW_COUNT - 1) % SETTINGS_ROW_COUNT;
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 0 => {
                self.toggle_auto_fetch();
            }
            KeyCode::Left | KeyCode::Char('-') if self.settings_selection == 1 => {
                self.change_fetch_interval(-1);
            }
            KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=')
                if self.settings_selection == 1 =>
            {
                self.change_fetch_interval(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 2 => {
                self.toggle_format_on_save();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 3 => {
                self.toggle_workspace_panel_enabled();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 4 => {
                self.toggle_agent_harness();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 5 => {
                self.toggle_agent_time_display();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 6 => {
                self.clear_agent_timing_history();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 7 => {
                self.toggle_media_preview_protocol();
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.settings_selection == 8 => {
                self.open_editor_setting();
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

    pub(crate) fn change_opencode_reasoning(&mut self, delta: isize) {
        self.settings.opencode_reasoning = self.settings.opencode_reasoning.next(delta);
        self.opencode_error = None;
        self.settings_changed();
    }

    pub(crate) fn open_settings(&mut self) {
        self.mode = Mode::Settings;
        self.settings_page = SettingsPage::General;
        self.shortcut_capture = false;
        self.shortcut_error = None;
        self.opencode_model_input = None;
        self.opencode_error = None;
    }

    pub(crate) fn close_settings(&mut self) {
        self.mode = Mode::Normal;
        self.shortcut_capture = false;
        self.shortcut_error = None;
        self.opencode_model_input = None;
        self.opencode_error = None;
    }

    pub(crate) fn handle_shortcut_settings(&mut self, key: KeyEvent) {
        let count = Shortcuts::definitions().len();
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
                let action = Shortcuts::definitions()[self.shortcut_selection].action;
                if self.settings.shortcuts.reset(action) {
                    self.settings_changed();
                }
                self.shortcut_error = None;
            }
            _ => {}
        }
    }

    pub(crate) fn keep_shortcut_selection_visible(&mut self) {
        let viewport = self.regions.shortcut_rows.len().max(1);
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

    pub(crate) fn toggle_workspace_panel_enabled(&mut self) {
        self.settings.workspace_panel_enabled = !self.settings.workspace_panel_enabled;
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

    pub(crate) fn toggle_agent_time_display(&mut self) {
        self.settings.agent_time_display = self.settings.agent_time_display.next();
        self.settings_changed();
    }

    pub(crate) fn clear_agent_timing_history(&mut self) {
        self.notice = Some(match self.workspace_panel.clear_agent_timing_history() {
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
