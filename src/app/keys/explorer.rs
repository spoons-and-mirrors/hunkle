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
        match key.code {
            KeyCode::Esc => {
                self.file_search.close();
                self.navigation.close_search();
            }
            KeyCode::Enter => self.activate_file_search_result(),
            KeyCode::Down => self.file_search.move_selection(1),
            KeyCode::Up => self.file_search.move_selection(-1),
            KeyCode::Tab => {
                if let Some(repo) = self.session.data() {
                    self.file_search.move_scope(1, repo);
                }
            }
            KeyCode::BackTab => {
                if let Some(repo) = self.session.data() {
                    self.file_search.move_scope(-1, repo);
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.clear(repo);
                }
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.delete_word(repo);
                }
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.toggle_case(repo);
                }
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.toggle_whole_word(repo);
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.toggle_regex(repo);
                }
            }
            KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(repo) = self.session.data() {
                    self.file_search.toggle_ignored(repo);
                }
            }
            _ => {
                if let Some(repo) = self.session.data() {
                    self.file_search.handle_edit_key(key, repo);
                }
            }
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
        self.file_search.reindex(
            &repository.files,
            &repository.ignored_files,
            Some(repository.files_fingerprint),
        );
        self.file_search.open(repository.inventory_truncated);
        self.navigation.open_search();
    }

    pub(crate) fn activate_file_search_result(&mut self) {
        let Some(destination) = self.file_search.selected_destination() else {
            return;
        };
        let (path, line) = match destination {
            SearchDestination::File(path) => (path, None),
            SearchDestination::Text { path, line } => (path, Some(line)),
        };
        let viewport = self
            .regions
            .explorer_list
            .or(self.regions.file_search_list)
            .map_or(1, |rect| usize::from(rect.height));
        if self.session.data().is_none() {
            return;
        }
        self.show_left_pane(LeftPane::Files);
        let repo = self.session.data().expect("repository checked above");
        if self.changes.select_explorer_path(repo, &path, viewport) {
            if let Some(line) = line {
                self.changes.pin_preview_line(path, line);
            }
            self.file_search.close();
            self.show_detail_panel();
        }
    }
}
