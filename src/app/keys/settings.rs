use super::super::*;

impl App {
    pub(crate) fn handle_settings(&mut self, key: KeyEvent) {
        if self.settings_state.shortcut_capture {
            if key.code == KeyCode::Esc {
                self.settings_state.shortcut_capture = false;
                self.settings_state.shortcut_error = None;
                return;
            }
            let Some(action) = Shortcuts::definitions()
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
            SettingsHitTarget::Shortcut(action) => {
                Shortcuts::definitions().position(|definition| definition.action == action)
            }
            _ => None,
        };
        let effect = self.settings_state.activate_target(target, shortcut_index);
        self.apply_settings_effect(effect);
    }

    fn apply_settings_effect(&mut self, effect: SettingsEffect) {
        match effect {
            SettingsEffect::Handled => {}
            SettingsEffect::ToggleAutoFetch => self.toggle_auto_fetch(),
            SettingsEffect::DecreaseFetchInterval => self.change_fetch_interval(-1),
            SettingsEffect::IncreaseFetchInterval => self.change_fetch_interval(1),
            SettingsEffect::ToggleFormatOnSave => self.toggle_format_on_save(),
            SettingsEffect::ToggleMediaPreview => self.toggle_media_preview_protocol(),
            SettingsEffect::ToggleSixelQuality => self.toggle_sixel_quality(),
            SettingsEffect::OpenEditor => self.open_editor_setting(),
        }
    }

    pub(crate) fn handle_shortcut_settings(&mut self, key: KeyEvent) {
        let count = Shortcuts::definitions().count();
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
                let action = Shortcuts::definitions()
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
        let count = Shortcuts::definitions().count();
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

    pub(crate) fn toggle_media_preview_protocol(&mut self) {
        self.settings.media_preview_protocol = self.settings.media_preview_protocol.next();
        self.reset_media_presentation();
        self.settings_changed();
    }

    pub(crate) fn toggle_sixel_quality(&mut self) {
        self.settings.sixel_quality = self.settings.sixel_quality.next();
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
