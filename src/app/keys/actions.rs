use super::super::*;

impl App {
    pub(crate) fn handle_action_menu(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('x') => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.actions.move_selection(1);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.actions.move_selection(-1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_action(),
            _ => {}
        }
    }

    pub(crate) fn handle_command(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.mode = Mode::Normal;
            return;
        }
        if self.actions.status != CommandStatus::Running {
            match key.code {
                KeyCode::Enter => {
                    let input = if self.actions.input.trim().is_empty()
                        && matches!(self.actions.status, CommandStatus::Complete { .. })
                    {
                        self.actions.command.clone()
                    } else {
                        self.actions.input.clone()
                    };
                    match parse_git_args(&input) {
                        Ok(args) => self.start_git_command("Git command".to_owned(), args),
                        Err(error) => {
                            self.actions.status = CommandStatus::Input;
                            self.actions.stderr = error;
                        }
                    }
                }
                KeyCode::Down if matches!(self.actions.status, CommandStatus::Complete { .. }) => {
                    self.actions.scroll_by(1);
                }
                KeyCode::Up if matches!(self.actions.status, CommandStatus::Complete { .. }) => {
                    self.actions.scroll_by(-1);
                }
                KeyCode::PageDown
                    if matches!(self.actions.status, CommandStatus::Complete { .. }) =>
                {
                    self.actions.scroll_by(10);
                }
                KeyCode::PageUp
                    if matches!(self.actions.status, CommandStatus::Complete { .. }) =>
                {
                    self.actions.scroll_by(-10);
                }
                KeyCode::Backspace => {
                    self.actions.input.pop();
                    if self.actions.status == CommandStatus::Input {
                        self.actions.stderr.clear();
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.clear();
                    if self.actions.status == CommandStatus::Input {
                        self.actions.stderr.clear();
                    }
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.push(character);
                    if self.actions.status == CommandStatus::Input {
                        self.actions.stderr.clear();
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.actions.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.actions.scroll_by(-1),
            KeyCode::PageDown => self.actions.scroll_by(10),
            KeyCode::PageUp => self.actions.scroll_by(-10),
            KeyCode::Home | KeyCode::Char('g') => self.actions.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.actions.scroll = self.actions.scroll_max,
            _ => {}
        }
    }

    pub(crate) fn handle_herdr_prompt(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc
            || self
                .settings
                .shortcuts
                .matches(ShortcutAction::OpenHerdr, key)
        {
            self.mode = Mode::Normal;
            return;
        }
        if self.herdr_prompt.sending {
            return;
        }

        match key.code {
            KeyCode::Enter => self.herdr_prompt.submit(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.herdr_prompt.input.select_all();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.herdr_prompt.input.clear();
                self.herdr_prompt.error = None;
            }
            KeyCode::Backspace
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.herdr_prompt.input.delete_word();
                self.herdr_prompt.error = None;
            }
            KeyCode::Left => self.herdr_prompt.input.move_left(),
            KeyCode::Right => self.herdr_prompt.input.move_right(),
            KeyCode::Home => self.herdr_prompt.input.move_home(),
            KeyCode::End => self.herdr_prompt.input.move_end(),
            KeyCode::Delete => {
                self.herdr_prompt.input.delete();
                self.herdr_prompt.error = None;
            }
            KeyCode::Backspace => {
                self.herdr_prompt.input.backspace();
                self.herdr_prompt.error = None;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.herdr_prompt.input.insert_char(character);
                self.herdr_prompt.error = None;
            }
            _ => {}
        }
    }

    pub(crate) fn open_actions(&mut self) {
        if self.require_git_repository() {
            self.mode = Mode::ActionMenu;
        }
    }

    pub(crate) fn open_herdr_prompt(&mut self) {
        if !self.herdr.is_enabled() {
            self.notice = Some("Herdr command prompt is only available inside Herdr".to_owned());
            return;
        }
        self.herdr_prompt.open();
        self.mode = Mode::HerdrPrompt;
    }

    pub(crate) fn open_git_command(&mut self) {
        if self.magic_commit_running_for_active_repository() {
            self.notice = Some("Magic Commit is still running".to_owned());
            return;
        }
        if self.require_git_repository() {
            self.actions.begin_input();
            self.mode = Mode::Command;
        }
    }

    pub(crate) fn activate_action(&mut self) {
        let action = self.actions.selected();
        if action == ActionId::Commit {
            self.show_left_pane(LeftPane::Worktree);
            if self.commit_input.text().trim().is_empty() {
                self.focus_commit();
            } else {
                self.start_commit();
            }
            return;
        }
        if action == ActionId::Custom {
            self.open_git_command();
            return;
        }
        if let Some((label, args)) = action_command(action) {
            self.start_git_command(label.to_owned(), args);
        }
    }

    pub(crate) fn start_git_command(&mut self, label: String, args: Vec<String>) {
        if self.magic_commit_running_for_active_repository() {
            self.mode = Mode::Normal;
            self.notice = Some("Magic Commit is still running".to_owned());
            return;
        }
        if !self.require_git_repository() {
            self.mode = Mode::Normal;
            return;
        }
        let display = display_git_command(&args);
        if self.session.start_command(label, args) {
            self.actions.begin_command(display);
            self.mode = Mode::Command;
            self.notice = None;
        } else {
            self.mode = Mode::Normal;
            self.notice = Some("Another Git operation is already running".to_owned());
        }
    }
}
