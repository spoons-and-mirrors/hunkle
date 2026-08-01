use super::super::*;

impl App {
    pub(crate) fn handle_file_editor(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if self
            .settings
            .shortcuts
            .matches(ShortcutAction::SaveOrFormat, key)
        {
            self.save_file_editor();
            return;
        }
        if control && key.code == KeyCode::Char('c') {
            if let Some(text) = self.selection.copy_text() {
                self.copy_request = Some(text);
            } else {
                self.notice = Some("Select text to copy".to_owned());
            }
            return;
        }
        let toggle_comment = (control
            && matches!(
                key.code,
                KeyCode::Char('/') | KeyCode::Char('_') | KeyCode::Char(':') | KeyCode::Char(';')
            ))
            || (key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('c'));
        if toggle_comment {
            let lines = self.selected_file_editor_lines().or_else(|| {
                self.file_editor
                    .as_ref()
                    .map(|editor| editor.cursor_position().0)
                    .map(|line| (line, line))
            });
            if let (Some((first, last)), Some(editor)) = (lines, &mut self.file_editor) {
                match editor.toggle_line_comments(first, last) {
                    Ok(()) => self.notice = None,
                    Err(error) => self.notice = Some(format!("Could not toggle comments: {error}")),
                }
            }
            self.selection.clear();
            return;
        }

        if key.code == KeyCode::Esc {
            if self.selection.has_selection() {
                self.selection.clear();
                return;
            }
            let Some(editor) = &mut self.file_editor else {
                self.mode = Mode::Normal;
                return;
            };
            if editor.dirty() && !editor.discard_armed {
                editor.discard_armed = true;
                self.notice = Some("Unsaved edits; press Esc again to discard".to_owned());
                return;
            }
            self.file_editor = None;
            self.file_editor_anchor = None;
            self.mode = Mode::Normal;
            self.restore_file_editor_scroll(true);
            self.notice = None;
            return;
        }
        self.selection.clear();
        if self.file_editor_viewport_too_small() {
            self.notice = Some("Resize the terminal before editing".to_owned());
            return;
        }

        let viewport = self
            .regions
            .preview_body
            .map_or(10, |area| usize::from(area.height).max(1));
        let Some(editor) = &mut self.file_editor else {
            self.mode = Mode::Normal;
            return;
        };
        editor.discard_armed = false;
        let insertion_error = match key.code {
            KeyCode::Enter => editor.insert_newline().err(),
            KeyCode::Tab => editor.insert("\t").err(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                editor.insert_char(character).err()
            }
            _ => None,
        };
        if let Some(error) = insertion_error {
            self.notice = Some(format!("Could not insert text: {error}"));
            return;
        }
        match key.code {
            KeyCode::Left if control => editor.move_home(),
            KeyCode::Right if control => editor.move_end(),
            KeyCode::Home if control => editor.move_document_start(),
            KeyCode::End if control => editor.move_document_end(),
            KeyCode::Left => editor.move_left(),
            KeyCode::Right => editor.move_right(),
            KeyCode::Up => editor.move_vertical(-1),
            KeyCode::Down => editor.move_vertical(1),
            KeyCode::Home => editor.move_home(),
            KeyCode::End => editor.move_end(),
            KeyCode::PageUp => editor.move_vertical(-(viewport as isize)),
            KeyCode::PageDown => editor.move_vertical(viewport as isize),
            KeyCode::Backspace => editor.backspace(),
            KeyCode::Delete => editor.delete(),
            KeyCode::Enter | KeyCode::Tab | KeyCode::Char(_) => {}
            _ => {}
        }
    }

    pub(crate) fn file_editor_viewport_too_small(&self) -> bool {
        self.regions
            .screen
            .is_some_and(|screen| screen.width < 60 || screen.height < 16)
    }

    pub(crate) fn selected_file_editor_lines(&self) -> Option<(usize, usize)> {
        let body = self.regions.preview_body?;
        if body.is_empty() {
            return None;
        }
        let (first, last) = self.selection.selected_rows()?;
        let source_line = |screen_row: u16| {
            let rendered_row = usize::from(
                screen_row
                    .clamp(body.y, body.bottom().saturating_sub(1))
                    .saturating_sub(body.y),
            );
            if self.changes.diff_wrap {
                self.regions
                    .editor_rows
                    .get(rendered_row.min(self.regions.editor_rows.len().saturating_sub(1)))
                    .map(|row| row.line)
            } else {
                self.file_editor
                    .as_ref()
                    .map(|editor| editor.scroll_line.saturating_add(rendered_row))
            }
        };
        Some((source_line(first)?, source_line(last)?))
    }

    pub(crate) fn start_file_editor(
        &mut self,
        path: RepoPath,
        line: usize,
        column: usize,
        anchor: Position,
    ) {
        self.last_file_editor_click = None;
        if self.session.open_running() {
            self.notice = Some("Wait for the workspace to finish opening".to_owned());
            return;
        }
        if self.format_running() {
            self.notice = Some("Wait for the formatter to finish".to_owned());
            return;
        }
        let Some(root) = self.repository().map(|repo| repo.root.clone()) else {
            self.notice = Some("Open a workspace first".to_owned());
            return;
        };
        match FileEditor::open(&root, path, line, column) {
            Ok(editor) => {
                self.file_editor_anchor = Some(anchor);
                self.file_editor_return = Some(FileEditorReturn {
                    path: editor.path().clone(),
                    pane: self.changes.pane,
                    scroll: self.changes.diff_scroll,
                });
                self.file_editor = Some(editor);
                self.mode = Mode::FileEdit;
                self.notice = None;
            }
            Err(error) => self.notice = Some(format!("Could not edit file: {error}")),
        }
    }

    pub(crate) fn save_file_editor(&mut self) {
        let Some(editor) = &self.file_editor else {
            self.mode = Mode::Normal;
            return;
        };
        if let Err(error) = editor.save() {
            self.notice = Some(format!("Could not save file: {error}"));
            return;
        }
        let root = editor.root().to_owned();
        let path = editor.path().clone();
        self.selection.clear();
        self.file_editor = None;
        self.file_editor_anchor = None;
        self.mode = Mode::Normal;

        if !self.settings.format_on_save {
            self.reload(RefreshScope::WORKTREE);
            self.notice = Some(format!("Saved {path}"));
            return;
        }

        match formatter::detect(&root, path.as_path()) {
            Ok(command) => {
                let label = command.label;
                if self.session.start_format(path.clone(), command) {
                    self.notice = Some(format!("Saved {path}; formatting with {label}…"));
                } else {
                    self.reload(RefreshScope::WORKTREE);
                    self.notice = Some(format!("Saved {path}; formatter is busy"));
                }
            }
            Err(error) => {
                self.reload(RefreshScope::WORKTREE);
                self.notice = Some(format!("Saved {path}; {error}"));
            }
        }
    }

    pub(crate) fn restore_file_editor_scroll(&mut self, clear_if_not_matching: bool) {
        let Some((return_path, return_pane, return_scroll)) = self
            .file_editor_return
            .as_ref()
            .map(|state| (state.path.clone(), state.pane, state.scroll))
        else {
            return;
        };
        let selected_path = self.repository().and_then(|repo| match self.changes.pane {
            LeftPane::Worktree => self
                .changes
                .selected_change_index(repo)
                .and_then(|index| repo.changes.get(index))
                .map(|change| &change.path),
            LeftPane::Files => self.changes.selected_explorer_file_path(repo),
        });
        if return_pane == self.changes.pane && selected_path == Some(&return_path) {
            self.changes.diff_scroll = return_scroll;
            self.file_editor_return = None;
        } else if clear_if_not_matching {
            self.file_editor_return = None;
        }
    }

    pub(crate) fn format_selected_file(&mut self) {
        let Some(repo) = self.repository() else {
            self.notice = Some("Open a workspace first".to_owned());
            return;
        };
        let Some(path) = self.changes.selected_explorer_file_path(repo).cloned() else {
            self.notice = Some("Select a file to format".to_owned());
            return;
        };
        let root = repo.root.clone();
        let command = match formatter::detect(&root, path.as_path()) {
            Ok(command) => command,
            Err(error) => {
                self.notice = Some(error.to_string());
                return;
            }
        };
        let label = command.label;
        if self.session.start_format(path.clone(), command) {
            self.notice = Some(format!("Formatting {path} with {label}…"));
        } else {
            self.notice = Some("Another repository operation is still running".to_owned());
        }
    }
}
