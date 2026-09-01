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
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.clear();
                    self.actions.clear_error();
                }
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
                        Err(error) => self.actions.set_input_error(error),
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
                    self.actions.clear_input_error();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.clear();
                    self.actions.clear_input_error();
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.actions.input.push(character);
                    self.actions.clear_input_error();
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

    pub(crate) fn open_actions(&mut self) {
        if self.require_git_repository() {
            self.mode = Mode::ActionMenu;
        }
    }

    pub(crate) fn open_git_command(&mut self) {
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
