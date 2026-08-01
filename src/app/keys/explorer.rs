use super::super::*;

impl App {
    pub(crate) fn handle_explorer(&mut self, key: KeyEvent) {
        let key = if self.workspace_explorer.editing_path || self.workspace_explorer.naming_favorite
        {
            key
        } else {
            self.settings.shortcuts.remap_explorer(key)
        };
        let command = self
            .workspace_explorer
            .handle_key(key, self.repository().is_some());
        self.apply_explorer_command(command);
    }

    pub(crate) fn handle_file_search(&mut self, key: KeyEvent) {
        if self
            .settings
            .shortcuts
            .matches(ShortcutAction::FindFile, key)
        {
            self.mode = Mode::Normal;
            return;
        }
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => self.activate_file_search_result(),
            KeyCode::Down | KeyCode::Tab => self.file_search.move_selection(1),
            KeyCode::Up | KeyCode::BackTab => self.file_search.move_selection(-1),
            KeyCode::Backspace => {
                if let Some(repo) = self.session.data() {
                    self.file_search.backspace(&repo.files);
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.clear(&repo.files);
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(repo) = self.session.data() {
                    self.file_search.push(character, &repo.files);
                }
            }
            _ => {}
        }
    }

    pub(in crate::app) fn apply_explorer_command(&mut self, command: PickerCommand) {
        match command {
            PickerCommand::None => {}
            PickerCommand::Close => self.mode = Mode::Normal,
            PickerCommand::Open(path) => self.open_repository(path),
            PickerCommand::OpenFile(path) => self.open_workspace_file(path),
        }
    }

    pub(crate) fn open_explorer(&mut self) {
        let start = self
            .repository()
            .map(|repo| repo.root.clone())
            .unwrap_or_else(|| self.workspace_explorer.directory.clone());
        if self.workspace_explorer.directory == start {
            let _ = self.workspace_explorer.poll_index();
        } else {
            self.workspace_explorer.navigate(start);
        }
        self.workspace_explorer.editing_path = false;
        self.explorer_tab = ExplorerTab::Explorer;
        self.mode = Mode::Explorer;
    }

    pub(crate) fn open_file_search(&mut self) {
        let Some(repository) = self.session.data() else {
            return;
        };
        if !repository.details_ready {
            self.notice = Some("Workspace files are still being indexed".to_owned());
            return;
        }
        self.file_search
            .reindex(&repository.files, Some(repository.files_fingerprint));
        self.file_search.open();
        self.mode = Mode::FileSearch;
    }

    pub(crate) fn activate_file_search_result(&mut self) {
        let Some(file_index) = self.file_search.selected_file_index() else {
            return;
        };
        let viewport = self
            .regions
            .explorer_list
            .map_or(0, |rect| usize::from(rect.height));
        if self.session.data().is_none() {
            return;
        }
        self.show_left_pane(LeftPane::Files);
        let repo = self.session.data().expect("repository checked above");
        if self
            .changes
            .select_explorer_file(repo, file_index, viewport)
        {
            self.mode = Mode::Normal;
        }
    }
}
