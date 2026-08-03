use super::super::*;

impl App {
    pub(crate) fn handle_file_editor(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let extend = key.modifiers.contains(KeyModifiers::SHIFT);
        if self
            .settings
            .shortcuts
            .matches(ShortcutAction::SaveOrFormat, key)
        {
            self.save_file_editor(false);
            return;
        }
        if control && key.code == KeyCode::Enter {
            self.save_file_editor(true);
            return;
        }

        if key.code == KeyCode::Esc {
            if self
                .file_editor
                .as_ref()
                .is_some_and(FileEditor::has_selection)
            {
                if let Some(editor) = &mut self.file_editor {
                    editor.clear_selection();
                }
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
            self.file_editor_dragging = false;
            self.mode = Mode::Normal;
            self.restore_file_editor_scroll(true);
            self.notice = None;
            return;
        }

        if self.format_running() {
            self.notice = Some("Wait for the formatter to finish".to_owned());
            return;
        }

        if control && matches!(key.code, KeyCode::Char('c' | 'C')) {
            if let Some(text) = self
                .file_editor
                .as_ref()
                .and_then(FileEditor::selected_text)
            {
                self.copy_request = Some(text.to_owned());
            } else {
                self.notice = Some("Select text to copy".to_owned());
            }
            return;
        }
        if control && matches!(key.code, KeyCode::Char('x' | 'X')) {
            if let Some(text) = self
                .file_editor
                .as_ref()
                .and_then(FileEditor::selected_text)
            {
                self.copy_request = Some(text.to_owned());
                if let Some(editor) = &mut self.file_editor {
                    editor.backspace();
                }
            } else {
                self.notice = Some("Select text to cut".to_owned());
            }
            return;
        }
        if control && matches!(key.code, KeyCode::Char('a' | 'A')) {
            if let Some(editor) = &mut self.file_editor {
                editor.select_all();
            }
            return;
        }
        if control && matches!(key.code, KeyCode::Char('z' | 'Z')) {
            let changed = if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.file_editor.as_mut().is_some_and(FileEditor::redo)
            } else {
                self.file_editor.as_mut().is_some_and(FileEditor::undo)
            };
            if !changed {
                self.notice = Some(
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        "Nothing to redo"
                    } else {
                        "Nothing to undo"
                    }
                    .to_owned(),
                );
            }
            return;
        }
        if control && matches!(key.code, KeyCode::Char('y' | 'Y')) {
            if !self.file_editor.as_mut().is_some_and(FileEditor::redo) {
                self.notice = Some("Nothing to redo".to_owned());
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
                    Ok(()) => {
                        editor.clear_selection();
                        self.notice = None;
                    }
                    Err(error) => self.notice = Some(format!("Could not toggle comments: {error}")),
                }
            }
            return;
        }

        let is_backtab = key.code == KeyCode::BackTab;
        let is_tab = key.code == KeyCode::Tab;
        if !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && (is_backtab
                || (is_tab
                    && (extend
                        || self
                            .file_editor
                            .as_ref()
                            .is_some_and(FileEditor::has_selection))))
        {
            let lines = self.selected_file_editor_lines().or_else(|| {
                self.file_editor
                    .as_ref()
                    .map(|editor| editor.cursor_position().0)
                    .map(|line| (line, line))
            });
            if let (Some((first, last)), Some(editor)) = (lines, &mut self.file_editor) {
                let outdent = is_backtab || extend;
                if let Err(error) = editor.indent_lines(first, last, outdent) {
                    self.notice = Some(format!("Could not indent text: {error}"));
                } else {
                    editor.clear_selection();
                    self.notice = None;
                }
            }
            return;
        }
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
            KeyCode::Left if control => editor.move_home_with_selection(extend),
            KeyCode::Right if control => editor.move_end_with_selection(extend),
            KeyCode::Home if control => editor.move_document_start_with_selection(extend),
            KeyCode::End if control => editor.move_document_end_with_selection(extend),
            KeyCode::Left => editor.move_left_with_selection(extend),
            KeyCode::Right => editor.move_right_with_selection(extend),
            KeyCode::Up => editor.move_vertical_with_selection(-1, extend),
            KeyCode::Down => editor.move_vertical_with_selection(1, extend),
            KeyCode::Home => editor.move_home_with_selection(extend),
            KeyCode::End => editor.move_end_with_selection(extend),
            KeyCode::PageUp => editor.move_vertical_with_selection(-(viewport as isize), extend),
            KeyCode::PageDown => editor.move_vertical_with_selection(viewport as isize, extend),
            KeyCode::Backspace => editor.backspace(),
            KeyCode::Delete => editor.delete(),
            KeyCode::BackTab | KeyCode::Enter | KeyCode::Tab | KeyCode::Char(_) => {}
            _ => {}
        }
    }

    pub(crate) fn file_editor_viewport_too_small(&self) -> bool {
        self.regions
            .screen
            .is_some_and(|screen| screen.width < APP_MIN_WIDTH || screen.height < 16)
    }

    pub(crate) fn selected_file_editor_lines(&self) -> Option<(usize, usize)> {
        self.file_editor
            .as_ref()
            .and_then(FileEditor::selected_line_range)
    }

    pub(crate) fn start_file_editor(
        &mut self,
        path: RepoPath,
        line: usize,
        column: usize,
        anchor: Position,
    ) {
        self.last_file_editor_click = None;
        self.file_editor_dragging = false;
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
                self.selection.clear();
                self.file_editor_anchor = Some(anchor);
                self.file_editor_return = Some(FileEditorReturn {
                    path: editor.path().clone(),
                    pane: self.changes.preview.pane(),
                    scroll: self.changes.diff_scroll,
                });
                self.file_editor = Some(editor);
                self.mode = Mode::FileEdit;
                self.notice = None;
            }
            Err(error) => self.notice = Some(format!("Could not edit file: {error}")),
        }
    }

    pub(crate) fn save_file_editor(&mut self, close: bool) {
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
        if close {
            self.selection.clear();
            self.file_editor = None;
            self.file_editor_anchor = None;
            self.file_editor_dragging = false;
            self.mode = Mode::Normal;
        } else if let Some(editor) = &mut self.file_editor {
            editor.mark_saved();
            editor.clear_selection();
            self.notice = Some(format!("Saved {path}"));
        }

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
        let selected_path = self.repository().and_then(|repo| match return_pane {
            LeftPane::Worktree => self
                .changes
                .selected_change_index(repo)
                .and_then(|index| repo.changes.get(index))
                .map(|change| &change.path),
            LeftPane::Files => self.changes.selected_explorer_file_path(repo),
        });
        if return_pane == self.changes.preview.pane() && selected_path == Some(&return_path) {
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
