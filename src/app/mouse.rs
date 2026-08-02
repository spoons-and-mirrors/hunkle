use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::time::{Duration, Instant};

use crate::{repo_path::RepoPath, selection::SelectionOutcome};

use super::{
    ACTION_ITEMS, App, CloneField, ExplorerHitTarget, GraphColumnDrag, GraphHitTarget,
    HeaderPickerKind, HitTarget, LeftPane, Mode, SettingsPage, Shortcuts, View,
    WorktreePickerField, changes::ChangesEffect, file_editor::FileEditor, scroll_table,
};

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);
const AGENT_PREVIEW_BUTTON_FLASH: Duration = Duration::from_millis(150);

impl App {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        if self.agent_preview_picker_open
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && !matches!(
                self.regions.hit_target_at(point),
                Some(HitTarget::AgentPreviewPicker(_) | HitTarget::AgentPreviewPickerItem(_))
            )
        {
            self.agent_preview_picker_open = false;
        }
        if mouse.kind == MouseEventKind::Moved {
            self.hovered_hit_target = self.regions.hit_target_at(point);
            if let Some(target) = self.hovered_hit_target {
                let agent = match target {
                    HitTarget::Agent(index) => Some(index),
                    HitTarget::AgentPreviewPicker(agent)
                    | HitTarget::AgentPreviewPickerItem(agent)
                    | HitTarget::AgentPreviewPrevious(agent)
                    | HitTarget::AgentPreviewNext(agent)
                    | HitTarget::AgentTooltip { agent, .. }
                    | HitTarget::AgentMessage { agent, .. } => Some(agent),
                    _ => None,
                };
                if let Some(index) = agent {
                    self.herdr.request_agent_latest_user_message(index);
                }
            }
        }
        if self.herdr_prompt.agent_pane_picker_open() {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                match self.regions.hit_target_at(point) {
                    Some(HitTarget::AgentPane(index)) => {
                        match self.herdr_prompt.select_agent_pane(index) {
                            Ok(()) => {
                                self.notice = Some("Starting agent in selected pane".to_owned())
                            }
                            Err(error) => self.notice = Some(error),
                        }
                    }
                    Some(HitTarget::AgentPaneSplit(index, direction)) => {
                        match self.herdr_prompt.split_agent_pane(index, direction) {
                            Ok(()) => self.notice = Some("Starting agent in new pane".to_owned()),
                            Err(error) => self.notice = Some(error),
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        if self.dragging_splitter {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => self.resize_worktree(mouse.column),
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resize_worktree(mouse.column);
                    self.dragging_splitter = false;
                    self.persist_settings();
                }
                _ => {}
            }
            return;
        }
        if self.dragging_agents {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => self.resize_agents(mouse.row),
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resize_agents(mouse.row);
                    self.dragging_agents = false;
                    self.persist_settings();
                }
                _ => {}
            }
            return;
        }
        if let Some(drag) = self.dragging_graph_column {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.resize_graph_column(drag, mouse.column);
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resize_graph_column(drag, mouse.column);
                    self.dragging_graph_column = None;
                    self.persist_settings();
                }
                _ => {}
            }
            return;
        }
        if self.dragging_diff_scrollbar {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => self.scroll_diff_to(mouse.row),
                MouseEventKind::Up(MouseButton::Left) => {
                    self.scroll_diff_to(mouse.row);
                    self.dragging_diff_scrollbar = false;
                }
                _ => {}
            }
            return;
        }
        if self.workspace_explorer.dragging_splitter {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.resize_explorer_panes(mouse.column);
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resize_explorer_panes(mouse.column);
                    self.workspace_explorer.dragging_splitter = false;
                    self.persist_settings();
                }
                _ => {}
            }
            return;
        }

        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            match self.regions.hit_target_at(point) {
                Some(HitTarget::HeaderRepository) => {
                    self.toggle_header_picker(HeaderPickerKind::Repositories);
                    return;
                }
                Some(HitTarget::HeaderWorktrees) => {
                    self.toggle_header_picker(HeaderPickerKind::Worktrees);
                    return;
                }
                Some(HitTarget::HeaderBranch) => {
                    self.toggle_header_picker(HeaderPickerKind::Branches);
                    return;
                }
                Some(HitTarget::HeaderDiff) => {
                    self.toggle_header_picker(HeaderPickerKind::DiffTargets);
                    return;
                }
                Some(HitTarget::HeaderAgent) => {
                    self.toggle_header_picker(HeaderPickerKind::AgentDestinations);
                    return;
                }
                _ => {}
            }
        }
        if self.header_picker.is_open() {
            match mouse.kind {
                MouseEventKind::ScrollDown
                    if matches!(
                        self.regions.hit_target_at(point),
                        Some(HitTarget::HeaderPickerOverlay | HitTarget::HeaderPickerItem(_))
                    ) =>
                {
                    self.hovered_hit_target = None;
                    self.header_picker.scroll_by(3);
                }
                MouseEventKind::ScrollUp
                    if matches!(
                        self.regions.hit_target_at(point),
                        Some(HitTarget::HeaderPickerOverlay | HitTarget::HeaderPickerItem(_))
                    ) =>
                {
                    self.hovered_hit_target = None;
                    self.header_picker.scroll_by(-3);
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    match self.regions.hit_target_at(point) {
                        Some(HitTarget::HeaderPickerItem(index)) => {
                            self.activate_header_picker(index)
                        }
                        Some(HitTarget::HeaderPickerNewBranch) => {
                            self.begin_header_branch_creation()
                        }
                        Some(HitTarget::HeaderPickerClone) => self.begin_repository_clone(),
                        Some(HitTarget::HeaderPickerCloneDirectory) => {
                            self.header_picker.set_clone_field(CloneField::Directory)
                        }
                        Some(HitTarget::HeaderPickerCloneUrl) => {
                            self.header_picker.set_clone_field(CloneField::Url)
                        }
                        Some(HitTarget::HeaderPickerNewWorktree) => {
                            self.begin_header_worktree_creation()
                        }
                        Some(HitTarget::HeaderPickerWorktreeName) => self
                            .header_picker
                            .set_worktree_field(WorktreePickerField::Name),
                        Some(HitTarget::HeaderPickerWorktreeBase) => self
                            .header_picker
                            .set_worktree_field(WorktreePickerField::Base),
                        Some(HitTarget::HeaderPickerOverlay) => {}
                        _ => self.header_picker.close(),
                    }
                }
                _ => {}
            }
            return;
        }
        if self.mode == Mode::FileEdit {
            self.handle_file_editor_mouse(mouse, point);
            return;
        }

        if matches!(
            mouse.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) && self.cycle_agent_message(point, mouse.kind == MouseEventKind::ScrollUp)
        {
            return;
        }

        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(HitTarget::Agent(index)) = self.regions.hit_target_at(point)
        {
            self.selection.clear();
            self.activate_agent(index);
            return;
        }

        if self.file_drag.is_some() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => self.update_file_drag(point),
                MouseEventKind::Up(MouseButton::Left) => self.finish_file_drag(point),
                _ => {}
            }
            return;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && !mouse.modifiers.contains(KeyModifiers::SHIFT)
            && self.begin_file_drag(point)
        {
            return;
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.selection.clear();
                if self.begin_mouse_control(point) {
                    return;
                }
                let region = self.selection_region(point);
                self.selection.begin(point, region);
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.selection.is_active() => {
                self.selection.update(point);
                return;
            }
            MouseEventKind::Up(MouseButton::Left) if self.selection.is_active() => {
                match self.selection.finish(point) {
                    SelectionOutcome::Click => self.handle_left_click(point),
                    SelectionOutcome::Selected(Some(text)) => self.copy_request = Some(text),
                    SelectionOutcome::Selected(None) => {}
                }
                return;
            }
            _ => {}
        }

        if self.mode == Mode::ActionMenu {
            self.handle_action_mouse(mouse);
            return;
        }
        if self.mode == Mode::AuthorFilter {
            self.handle_author_filter_mouse(mouse);
            return;
        }
        if self.mode == Mode::Explorer {
            self.handle_explorer_mouse(mouse);
            return;
        }
        if self.mode == Mode::Command {
            self.handle_command_mouse(mouse);
            return;
        }
        if self.mode == Mode::HerdrPrompt {
            return;
        }
        if self.mode == Mode::Editor {
            return;
        }
        if self.mode == Mode::Files {
            return;
        }
        if self.mode == Mode::FileSearch {
            self.handle_file_search_mouse(mouse);
            return;
        }
        if self.mode == Mode::Settings {
            self.handle_settings_mouse(mouse);
            return;
        }
        if self.mode == Mode::Help {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                self.mode = Mode::Normal;
            }
            return;
        }
        if mouse.kind == MouseEventKind::Moved {
            if self.select_graph_row(point) {
                return;
            }
            if self.changes.hunk_selection.is_some() {
                if let Some(hunk) = self
                    .regions
                    .diff_hunks
                    .iter()
                    .find(|hunk| hunk.rect.contains(point))
                {
                    self.changes.select_hunk(hunk.index);
                }
                return;
            }
        }

        if self.mode == Mode::Commit
            && mouse.kind == MouseEventKind::Down(MouseButton::Right)
            && !self.regions.commit.is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Normal;
            self.flush_commit_draft();
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_at(point, 1),
            MouseEventKind::ScrollUp => self.scroll_at(point, -1),
            MouseEventKind::Down(MouseButton::Right) => {
                let effect = self
                    .regions
                    .hit_target_at(point)
                    .and_then(|target| match target {
                        HitTarget::Changes(target) => self
                            .session
                            .data()
                            .and_then(|repo| self.changes.stage_target(target, repo)),
                        _ => None,
                    });
                self.apply_changes_effect(effect);
            }
            _ => {}
        }
    }

    fn handle_file_editor_mouse(&mut self, mouse: MouseEvent, point: Position) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.last_file_editor_click = None;
                let height = self
                    .regions
                    .preview_body
                    .map_or(1, |body| usize::from(body.height).max(1));
                let wrapped = self.changes.diff_wrap;
                if let Some(editor) = &mut self.file_editor {
                    editor.scroll_viewport(3, height, wrapped);
                }
            }
            MouseEventKind::ScrollUp => {
                self.last_file_editor_click = None;
                let height = self
                    .regions
                    .preview_body
                    .map_or(1, |body| usize::from(body.height).max(1));
                let wrapped = self.changes.diff_wrap;
                if let Some(editor) = &mut self.file_editor {
                    editor.scroll_viewport(-3, height, wrapped);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let in_editor = self
                    .regions
                    .preview_body
                    .is_some_and(|body| body.contains(point));
                if in_editor {
                    self.place_file_editor_cursor(point, false);
                    if let Some(editor) = &mut self.file_editor {
                        editor.begin_selection();
                        self.file_editor_dragging = true;
                    }
                } else {
                    self.file_editor_dragging = false;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.file_editor_dragging => {
                self.place_file_editor_cursor(point, true);
            }
            MouseEventKind::Up(MouseButton::Left) if self.file_editor_dragging => {
                self.place_file_editor_cursor(point, true);
                self.file_editor_dragging = false;
                let in_editor = self
                    .regions
                    .preview_body
                    .is_some_and(|body| body.contains(point));
                let has_selection = self
                    .file_editor
                    .as_ref()
                    .is_some_and(FileEditor::has_selection);
                let double_click = in_editor
                    && !has_selection
                    && self.last_file_editor_click.is_some_and(|(previous, at)| {
                        previous == point && at.elapsed() <= DOUBLE_CLICK_INTERVAL
                    });
                if double_click {
                    self.last_file_editor_click = None;
                    if let Some(editor) = &mut self.file_editor
                        && !editor.select_word_at_cursor()
                    {
                        editor.clear_selection();
                    }
                } else if has_selection {
                    self.last_file_editor_click = None;
                } else {
                    self.last_file_editor_click = in_editor.then(|| (point, Instant::now()));
                    if let Some(editor) = &mut self.file_editor {
                        editor.clear_selection();
                    }
                }
            }
            _ => {}
        }
    }

    fn begin_mouse_control(&mut self, point: Position) -> bool {
        if self.mode == Mode::Explorer
            && matches!(
                self.regions.hit_target_at(point),
                Some(HitTarget::Explorer(ExplorerHitTarget::Splitter))
            )
        {
            self.workspace_explorer.dragging_splitter = true;
            self.resize_explorer_panes(point.x);
            return true;
        }
        if !matches!(self.mode, Mode::Normal | Mode::Commit) {
            return false;
        }
        if let Some(column) = self
            .regions
            .graph_columns
            .iter()
            .find(|column| column.splitter.contains(point))
            .copied()
        {
            let drag = GraphColumnDrag {
                left: column.left,
                right: column.right,
                origin_x: point.x,
                left_width: column.left_width,
                right_width: column.right_width,
            };
            self.dragging_graph_column = Some(drag);
            self.resize_graph_column(drag, point.x);
            return true;
        }
        if self
            .regions
            .splitter
            .is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Normal;
            self.dragging_splitter = true;
            self.resize_worktree(point.x);
            return true;
        }
        if self
            .regions
            .agents_splitter
            .is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Normal;
            self.dragging_agents = true;
            self.resize_agents(point.y);
            return true;
        }
        if self
            .regions
            .diff_scrollbar
            .is_some_and(|rect| rect.contains(point))
            && self.regions.diff_scroll_max > 0
        {
            self.mode = Mode::Normal;
            self.dragging_diff_scrollbar = true;
            self.diff_scroll_drag_offset = self
                .regions
                .diff_scroll_thumb
                .filter(|thumb| thumb.contains(point))
                .map_or_else(
                    || {
                        self.regions
                            .diff_scroll_thumb
                            .map_or(0, |thumb| thumb.height / 2)
                    },
                    |thumb| point.y.saturating_sub(thumb.y),
                );
            self.scroll_diff_to(point.y);
            return true;
        }
        false
    }

    fn selection_region(&self, point: Position) -> Rect {
        [
            self.regions.command_overlay,
            self.regions.herdr_prompt_overlay,
            self.regions.editor_overlay,
            self.regions.file_search_overlay,
            self.regions.file_dialog_overlay,
            self.regions
                .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::Overlay)),
            self.regions.settings_overlay,
            self.regions.action_menu,
            self.regions
                .hit_target_rect(HitTarget::Graph(GraphHitTarget::FilterOverlay)),
            self.regions.diff,
            self.regions.worktree,
            self.regions.graph_table,
        ]
        .into_iter()
        .flatten()
        .find(|region| region.contains(point))
        .or(self.regions.screen)
        .or_else(|| self.selection.screen_area())
        .unwrap_or(Rect::new(point.x, point.y, 1, 1))
    }

    pub(super) fn handle_left_click(&mut self, point: Position) {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: point.x,
            row: point.y,
            modifiers: KeyModifiers::NONE,
        };
        match self.mode {
            Mode::ActionMenu => self.handle_action_mouse(mouse),
            Mode::Command => self.handle_command_mouse(mouse),
            Mode::HerdrPrompt => {}
            Mode::FileEdit => self.place_file_editor_cursor(point, false),
            Mode::Explorer => self.handle_explorer_mouse(mouse),
            Mode::FileSearch => self.handle_file_search_mouse(mouse),
            Mode::Settings => self.handle_settings_mouse(mouse),
            Mode::AuthorFilter => self.handle_author_filter_mouse(mouse),
            Mode::Help => self.mode = Mode::Normal,
            Mode::Editor => {}
            Mode::Files => self.handle_file_dialog_click(point),
            Mode::Normal | Mode::Commit => self.handle_primary_left_click(point),
        }
    }

    pub(super) fn handle_primary_left_click(&mut self, point: Position) {
        if !self
            .regions
            .worktree_list
            .is_some_and(|rect| rect.contains(point))
        {
            self.last_worktree_file_click = None;
        }
        if self.mode == Mode::Commit
            && !self.regions.commit.is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Normal;
            self.flush_commit_draft();
        }
        match self.regions.hit_target_at(point) {
            Some(HitTarget::Changes(target)) => {
                let effect = self
                    .session
                    .data()
                    .and_then(|repo| self.changes.activate_target(target, repo));
                self.apply_changes_effect(effect);
                return;
            }
            Some(HitTarget::CommitMessageGenerate) => {
                self.generate_commit_message();
                return;
            }
            Some(HitTarget::MarkdownPreviewToggle) => {
                self.toggle_markdown_preview();
                return;
            }
            Some(HitTarget::Graph(GraphHitTarget::AuthorHeader)) => {
                self.open_author_filter();
                return;
            }
            Some(HitTarget::Agent(index)) => {
                self.activate_agent(index);
                return;
            }
            Some(HitTarget::AgentPreviewPicker(index)) => {
                self.agent_preview_selection = Some(index);
                self.agent_preview_picker_open = !self.agent_preview_picker_open;
                return;
            }
            Some(HitTarget::AgentPreviewPickerItem(index)) => {
                self.select_agent_preview(index);
                return;
            }
            Some(HitTarget::AgentPreviewPrevious(index)) => {
                self.cycle_agent_preview(index, false);
                return;
            }
            Some(HitTarget::AgentPreviewNext(index)) => {
                self.cycle_agent_preview(index, true);
                return;
            }
            Some(HitTarget::AgentTooltip { .. } | HitTarget::AgentMessage { .. }) => return,
            _ => {}
        }
        if self
            .regions
            .files_add
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_add_dialog();
            return;
        }
        if self
            .regions
            .actions
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_actions();
            return;
        }
        if self
            .regions
            .left_pane_toggle
            .is_some_and(|rect| rect.contains(point))
        {
            self.toggle_left_pane();
            return;
        }
        if self
            .regions
            .changes
            .is_some_and(|rect| rect.contains(point))
        {
            self.show_main_pane();
        } else if self.regions.graph.is_some_and(|rect| rect.contains(point)) {
            self.toggle_graph();
        } else if self
            .regions
            .explorer
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_explorer();
        } else if self
            .regions
            .settings
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_settings();
        } else if self.regions.help.is_some_and(|rect| rect.contains(point)) {
            self.mode = Mode::Help;
        } else if self.select_explorer_row(point) {
            if self.changes.selected_explorer_directory_path().is_some() {
                let repo = self.session.data();
                self.changes.toggle_selected_explorer_directory(repo);
            } else {
                self.show_main_pane();
            }
        } else if self.select_agents_row(point) {
        } else if self.select_graph_row(point) {
            self.open_selected_graph_commit();
        } else if self
            .regions
            .preview_body
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_file_editor_at(point);
        } else if let Some(commit) = self.regions.commit.filter(|rect| rect.contains(point)) {
            let scroll = self.regions.commit_scroll;
            self.focus_commit();
            let width = usize::from(commit.width.saturating_sub(2)).max(1);
            let row = scroll + usize::from(point.y.saturating_sub(commit.y));
            let column = usize::from(point.x.saturating_sub(commit.x.saturating_add(1)))
                .min(width.saturating_sub(1));
            self.commit_input
                .set_cursor_at_visual_position(width, row, column);
            self.commit_scroll = Some(scroll.min(self.regions.commit_scroll_max));
        }
    }

    fn open_file_editor_at(&mut self, point: Position) {
        let Some(body) = self.regions.preview_body else {
            return;
        };
        if self.regions.preview_generation != self.changes.preview_content_generation {
            self.notice = Some("Preview changed; click again to edit".to_owned());
            return;
        }
        let rendered_row = self
            .regions
            .preview_scroll
            .saturating_add(usize::from(point.y.saturating_sub(body.y)));
        let width = usize::from(body.width);
        let rendered_column = usize::from(point.x.saturating_sub(body.x));

        if self.changes.pane == LeftPane::Files {
            let Some(path) = self.regions.preview_path.clone() else {
                return;
            };
            let gutter = usize::from(width >= 72) * 7;
            let Some((line, column)) = self
                .changes
                .preview_presentation
                .source_position_at_rendered_position(
                    &self.changes.diff,
                    rendered_row,
                    rendered_column,
                    gutter,
                )
            else {
                return;
            };
            self.start_file_editor(path, line, column, point);
            return;
        }

        if self.regions.preview_untracked {
            let Some(path) = self.regions.preview_path.clone() else {
                return;
            };
            let Some((display_line, column)) = self
                .changes
                .preview_presentation
                .source_position_at_rendered_position(
                    &self.changes.diff,
                    rendered_row,
                    rendered_column,
                    0,
                )
            else {
                return;
            };
            let Some(source_line) = display_line.checked_sub(2) else {
                self.notice = Some("Click a source line to edit this file".to_owned());
                return;
            };
            let displayed = self
                .changes
                .diff
                .lines()
                .nth(display_line.saturating_sub(1));
            let next = self.changes.diff.lines().nth(display_line);
            if displayed.is_some_and(|line| line.starts_with("[Preview truncated"))
                || displayed.is_some_and(str::is_empty)
                    && next.is_some_and(|line| line.starts_with("[Preview truncated"))
            {
                self.notice = Some("Click a source line to edit this file".to_owned());
                return;
            }
            self.start_file_editor(path, source_line, column, point);
            return;
        }
        let gutter = if width >= 72 { 7 } else { 1 };
        let position = self.regions.preview_path.clone().and_then(|path| {
            self.changes
                .preview_presentation
                .diff_position_at_rendered_position(
                    &self.changes.diff,
                    rendered_row,
                    rendered_column,
                    gutter,
                )
                .map(|(line, column)| (path, line, column))
        });
        let position = position.or_else(|| {
            self.changes
                .preview_presentation
                .diff_file_position_at_rendered_position(
                    &self.changes.diff,
                    rendered_row,
                    rendered_column,
                    gutter,
                )
        });
        let Some((path, line, column)) = position else {
            self.notice = Some("Click an added or context line to edit this file".to_owned());
            return;
        };
        self.start_file_editor(path, line, column, point);
    }

    fn place_file_editor_cursor(&mut self, point: Position, extend: bool) {
        let Some(body) = self
            .regions
            .preview_body
            .filter(|body| body.contains(point))
        else {
            return;
        };
        if let Some(editor) = &mut self.file_editor {
            if self.changes.diff_wrap {
                let row = usize::from(point.y.saturating_sub(body.y));
                if let Some(rendered) = self.regions.editor_rows.get(row) {
                    let column = usize::from(point.x.saturating_sub(body.x));
                    if extend {
                        editor.extend_cursor(rendered.line, rendered.source_column_at(column));
                    } else {
                        editor.set_cursor(rendered.line, rendered.source_column_at(column));
                    }
                }
                return;
            }
            let line = editor
                .scroll_line
                .saturating_add(usize::from(point.y.saturating_sub(body.y)));
            let column = editor
                .scroll_column
                .saturating_add(usize::from(point.x.saturating_sub(body.x)));
            if extend {
                editor.extend_cursor(line, column);
            } else {
                editor.set_cursor(line, column);
            }
        }
    }

    fn apply_changes_effect(&mut self, effect: Option<ChangesEffect>) {
        match effect {
            Some(ChangesEffect::SidebarPaneActivated) => {
                self.dismiss_agent_preview();
                self.last_worktree_file_click = None;
                self.mode = Mode::Normal;
            }
            Some(ChangesEffect::PaneActivated) => {
                self.dismiss_agent_preview();
                self.last_worktree_file_click = None;
                self.mode = Mode::Normal;
                self.show_main_pane();
            }
            Some(ChangesEffect::AgentsPaneActivated) => {
                self.show_agents_pane();
                self.mode = Mode::Normal;
            }
            Some(ChangesEffect::WorktreeDirectoryActivated) => {
                self.last_worktree_file_click = None;
            }
            Some(ChangesEffect::ToggleAllStaging) => self.toggle_all_staging(),
            Some(ChangesEffect::ToggleSelectedStage) => {
                self.last_worktree_file_click = None;
                self.toggle_stage();
            }
            Some(ChangesEffect::StageHunk(index)) => self.stage_hunk(index, false),
            Some(ChangesEffect::OpenDiffFileHeader(index)) => {
                let Some(header) = self.regions.diff_file_headers.get(index).cloned() else {
                    return;
                };
                self.start_file_editor(
                    header.path,
                    header.line,
                    0,
                    Position::new(header.rect.x, header.rect.y),
                );
            }
            Some(ChangesEffect::WorktreeFileSelected { path, staged }) => {
                if self.register_worktree_file_click(&path, staged)
                    && self.open_worktree_file_in_files(&path)
                {
                    return;
                }
                self.show_main_pane();
            }
            None => {}
        }
    }

    fn register_worktree_file_click(&mut self, path: &RepoPath, staged: bool) -> bool {
        let double_click = self.last_worktree_file_click.as_ref().is_some_and(
            |(previous_path, previous_staged, at)| {
                previous_path == path
                    && *previous_staged == staged
                    && at.elapsed() <= DOUBLE_CLICK_INTERVAL
            },
        );
        self.last_worktree_file_click =
            (!double_click).then(|| (path.clone(), staged, Instant::now()));
        double_click
    }

    fn open_worktree_file_in_files(&mut self, path: &RepoPath) -> bool {
        let viewport = self
            .regions
            .worktree_list
            .map_or(0, |rect| usize::from(rect.height));
        let Some(repo) = self.session.data() else {
            return false;
        };
        if !self.changes.select_explorer_path(repo, path, viewport) {
            return false;
        }
        self.show_left_pane(LeftPane::Files);
        self.mode = Mode::Normal;
        true
    }

    fn activate_agent(&mut self, index: usize) {
        if let Some(pane_id) = self
            .herdr
            .agents
            .get(index)
            .map(|agent| agent.pane_id.clone())
        {
            self.hovered_hit_target = None;
            self.agents_pane_pinned = false;
            self.agent_preview_selection = None;
            self.agent_preview_picker_open = false;
            self.herdr.display_agent(pane_id);
        }
    }

    fn cycle_agent_preview(&mut self, current: usize, forward: bool) {
        let count = self.herdr.agents.len();
        if count == 0 {
            return;
        }
        let current = current.min(count - 1);
        let index = if forward {
            (current + 1) % count
        } else if current == 0 {
            count - 1
        } else {
            current - 1
        };
        self.agent_preview_picker_open = false;
        self.agent_preview_button_flash =
            Some((forward, Instant::now() + AGENT_PREVIEW_BUTTON_FLASH));
        self.select_agent_preview(index);
    }

    fn select_agent_preview(&mut self, index: usize) {
        if index >= self.herdr.agents.len() {
            return;
        }
        self.agent_preview_selection = Some(index);
        self.agent_preview_picker_open = false;
        self.hovered_hit_target = self
            .herdr
            .agent_user_messages(index)
            .filter(|messages| !messages.is_empty())
            .map_or(Some(HitTarget::Agent(index)), |messages| {
                Some(HitTarget::AgentTooltip {
                    agent: index,
                    message: messages.len() - 1,
                })
            });
        self.herdr.request_agent_latest_user_message(index);
    }

    fn handle_action_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.actions.move_selection(1),
            MouseEventKind::ScrollUp => self.actions.move_selection(-1),
            MouseEventKind::Moved => {
                if let Some(index) = self.action_at(point) {
                    self.actions.selection = index;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self
                    .regions
                    .actions
                    .is_some_and(|rect| rect.contains(point))
                {
                    self.mode = Mode::Normal;
                    return;
                }
                let Some(index) = self.action_at(point) else {
                    self.mode = Mode::Normal;
                    return;
                };
                self.actions.selection = index;
                self.activate_action();
            }
            _ => {}
        }
    }

    fn handle_author_filter_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.author_filter.move_selection(1),
            MouseEventKind::ScrollUp => self.author_filter.move_selection(-1),
            MouseEventKind::Moved => {
                if let Some(HitTarget::Graph(GraphHitTarget::FilterItem(index))) =
                    self.regions.hit_target_at(point)
                {
                    self.author_filter.select(index);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => match self.regions.hit_target_at(point) {
                Some(HitTarget::Graph(GraphHitTarget::FilterItem(index))) => {
                    self.author_filter.select(index);
                    if self.author_filter.toggle(index) {
                        self.reconcile_graph_selection();
                    }
                }
                Some(HitTarget::Graph(GraphHitTarget::FilterOverlay)) => {}
                _ => self.mode = Mode::Normal,
            },
            _ => {}
        }
    }

    fn action_at(&self, point: Position) -> Option<usize> {
        let list = self
            .regions
            .action_list
            .filter(|rect| rect.contains(point))?;
        let index = usize::from(point.y.saturating_sub(list.y));
        (index < ACTION_ITEMS.len()).then_some(index)
    }

    fn handle_command_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.actions.scroll_by(3),
            MouseEventKind::ScrollUp => self.actions.scroll_by(-3),
            _ => {}
        }
    }

    fn handle_explorer_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if mouse.kind == MouseEventKind::ScrollDown {
                    1
                } else {
                    -1
                };
                if self.workspace_explorer.editing_path {
                    self.workspace_explorer.move_match_selection(delta);
                } else if matches!(
                    self.regions.hit_target_at(point),
                    Some(HitTarget::Explorer(
                        ExplorerHitTarget::SurroundingsPane | ExplorerHitTarget::Surrounding { .. }
                    ))
                ) {
                    self.workspace_explorer.move_surrounding_selection(delta);
                } else {
                    self.workspace_explorer.move_selection(delta);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => match self.regions.hit_target_at(point) {
                Some(HitTarget::Explorer(target)) => {
                    let command = self.workspace_explorer.activate_target(target);
                    self.apply_explorer_command(command);
                }
                _ if self.repository().is_some() => {
                    self.mode = Mode::Normal;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn handle_file_search_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.file_search.move_selection(1),
            MouseEventKind::ScrollUp => self.file_search.move_selection(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if self
                    .regions
                    .file_search_overlay
                    .is_some_and(|rect| !rect.contains(point))
                {
                    self.mode = Mode::Normal;
                    return;
                }
                let Some(list) = self
                    .regions
                    .file_search_list
                    .filter(|rect| rect.contains(point))
                else {
                    return;
                };
                let index = self.file_search.state.offset() + usize::from(point.y - list.y);
                if self.file_search.select(index) {
                    self.activate_file_search_result();
                }
            }
            _ => {}
        }
    }

    fn handle_settings_mouse(&mut self, mouse: MouseEvent) {
        if self.settings_page == SettingsPage::Shortcuts
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            let key = if mouse.kind == MouseEventKind::ScrollUp {
                KeyCode::Up
            } else {
                KeyCode::Down
            };
            for _ in 0..3 {
                self.handle_shortcut_settings(KeyEvent::new(key, KeyModifiers::NONE));
            }
            return;
        }
        let point = Position::new(mouse.column, mouse.row);
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        if self
            .regions
            .settings_overlay
            .is_some_and(|rect| !rect.contains(point))
        {
            self.close_settings();
        } else if self
            .regions
            .settings_general_tab
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_page = SettingsPage::General;
            self.shortcut_capture = false;
            self.shortcut_error = None;
            self.opencode_model_input = None;
            self.opencode_error = None;
        } else if self
            .regions
            .settings_shortcuts_tab
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_page = SettingsPage::Shortcuts;
            self.shortcut_capture = false;
            self.shortcut_error = None;
            self.opencode_model_input = None;
            self.opencode_error = None;
        } else if self
            .regions
            .settings_opencode_tab
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_page = SettingsPage::OpenCode;
            self.shortcut_capture = false;
            self.shortcut_error = None;
            self.opencode_model_input = None;
            self.opencode_error = None;
        } else if self
            .regions
            .opencode_model_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.opencode_selection = 0;
            self.begin_opencode_model_input();
        } else if self
            .regions
            .opencode_reasoning_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.opencode_selection = 1;
            self.change_opencode_reasoning(1);
        } else if let Some((action, _)) = self
            .regions
            .shortcut_rows
            .iter()
            .find(|(_, rect)| rect.contains(point))
            .copied()
        {
            if let Some(index) = Shortcuts::definitions()
                .iter()
                .position(|definition| definition.action == action)
            {
                self.shortcut_selection = index;
                self.shortcut_capture = true;
                self.shortcut_error = None;
            }
        } else if self
            .regions
            .auto_fetch
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 0;
            self.toggle_auto_fetch();
        } else if self
            .regions
            .fetch_interval_down
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 1;
            self.change_fetch_interval(-1);
        } else if self
            .regions
            .fetch_interval_up
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 1;
            self.change_fetch_interval(1);
        } else if self
            .regions
            .fetch_interval
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 1;
        } else if self
            .regions
            .format_on_save_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 2;
            self.toggle_format_on_save();
        } else if self
            .regions
            .cross_workspace_agents_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 3;
            self.toggle_cross_workspace_agents();
        } else if self
            .regions
            .agent_harness_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 4;
            self.toggle_agent_harness();
        } else if self
            .regions
            .agent_time_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 5;
            self.toggle_agent_time_display();
        } else if self
            .regions
            .clear_agent_timings_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 6;
            self.clear_agent_timing_history();
        } else if self
            .regions
            .media_preview_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 7;
            self.toggle_media_preview_protocol();
        } else if self
            .regions
            .editor_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 8;
            self.open_editor_setting();
        }
    }

    fn select_explorer_row(&mut self, point: Position) -> bool {
        if self.changes.pane != LeftPane::Files {
            return false;
        }
        let Some(rect) = self
            .regions
            .explorer_list
            .filter(|rect| rect.contains(point))
        else {
            return false;
        };
        let index = self.changes.explorer_scroll + usize::from(point.y - rect.y);
        let Some(repo) = self.session.data() else {
            return false;
        };
        self.changes.select_explorer_row(repo, index)
    }

    fn select_agents_row(&mut self, point: Position) -> bool {
        if !self
            .regions
            .agents_list
            .is_some_and(|rect| rect.contains(point))
        {
            return false;
        }
        // Agent cards have semantic hit targets. Consume clicks on the padding
        // between them without translating visual rows into agent indexes.
        true
    }

    fn select_graph_row(&mut self, point: Position) -> bool {
        if self.visible_view() != View::Graph {
            return false;
        }
        let Some(rect) = self.regions.graph_table.filter(|rect| rect.contains(point)) else {
            return false;
        };
        let index = self.graph_state.offset() + usize::from(point.y - rect.y);
        let len = self.visible_graph_len();
        if index >= len {
            return false;
        }
        self.graph_state.select(Some(index));
        self.graph_scroll_to_selection = false;
        true
    }

    fn cycle_agent_message(&mut self, point: Position, older: bool) -> bool {
        let Some(target) = self.regions.hit_target_at(point) else {
            return false;
        };
        let (agent, pointed_message) = match target {
            HitTarget::Agent(agent) => (agent, None),
            HitTarget::AgentTooltip { agent, message }
            | HitTarget::AgentMessage { agent, message } => (agent, Some(message)),
            _ => return false,
        };
        let Some(message_count) = self
            .herdr
            .agent_user_messages(agent)
            .map(|messages| messages.len())
            .filter(|count| *count > 0)
        else {
            self.herdr.request_agent_latest_user_message(agent);
            return true;
        };
        let current = pointed_message
            .or(match self.hovered_hit_target {
                Some(HitTarget::AgentTooltip {
                    agent: hovered_agent,
                    message,
                }) if hovered_agent == agent => Some(message),
                Some(HitTarget::AgentMessage {
                    agent: hovered_agent,
                    message,
                }) if hovered_agent == agent => Some(message),
                _ => None,
            })
            .unwrap_or(message_count - 1)
            .min(message_count - 1);
        let message = if older {
            current.checked_sub(1).unwrap_or(message_count - 1)
        } else {
            (current + 1) % message_count
        };
        self.hovered_hit_target = Some(HitTarget::AgentTooltip { agent, message });
        true
    }

    fn scroll_at(&mut self, point: Position, delta: isize) {
        if self.regions.commit.is_some_and(|rect| rect.contains(point)) {
            let amount = delta.saturating_mul(2);
            let current = self.regions.commit_scroll;
            let next = if amount < 0 {
                current.saturating_sub(amount.unsigned_abs())
            } else {
                current.saturating_add(amount as usize)
            }
            .min(self.regions.commit_scroll_max);
            self.commit_scroll = Some(next);
        } else if self
            .regions
            .sqlite_objects
            .is_some_and(|rect| rect.contains(point))
        {
            let viewport = self
                .regions
                .sqlite_objects
                .map_or(0, |rect| usize::from(rect.height));
            self.changes
                .scroll_sqlite_objects(viewport, delta.saturating_mul(3));
        } else if self
            .regions
            .sqlite_rows
            .is_some_and(|rect| rect.contains(point))
        {
            let viewport = self
                .regions
                .sqlite_rows
                .map_or(0, |rect| usize::from(rect.height));
            self.changes
                .scroll_sqlite_rows(viewport, delta.saturating_mul(3));
        } else if self.regions.diff.is_some_and(|rect| rect.contains(point)) {
            self.changes
                .scroll_diff_by(self.regions.diff_scroll_max, delta.saturating_mul(3));
        } else if self
            .regions
            .explorer_list
            .is_some_and(|rect| rect.contains(point))
        {
            self.scroll_explorer(delta.saturating_mul(3));
        } else if self
            .regions
            .agents_list
            .is_some_and(|rect| rect.contains(point))
        {
            self.herdr.scroll_agents(delta);
        } else if self
            .regions
            .worktree_list
            .is_some_and(|rect| rect.contains(point))
        {
            self.scroll_worktree(delta.saturating_mul(3));
        } else if self
            .regions
            .graph_table
            .is_some_and(|rect| rect.contains(point))
        {
            self.scroll_graph(delta.saturating_mul(3));
        }
    }

    fn scroll_graph(&mut self, delta: isize) {
        let viewport = self
            .regions
            .graph_table
            .map_or(0, |rect| usize::from(rect.height));
        let len = self.visible_graph_len();
        scroll_table(&mut self.graph_state, len, viewport, delta);
        self.graph_scroll_to_selection = false;
    }

    fn scroll_worktree(&mut self, delta: isize) {
        let viewport = self
            .regions
            .worktree_list
            .map_or(0, |rect| usize::from(rect.height));
        self.changes
            .scroll_worktree(self.session.data(), viewport, delta);
    }

    fn scroll_explorer(&mut self, delta: isize) {
        let viewport = self
            .regions
            .explorer_list
            .map_or(0, |rect| usize::from(rect.height));
        self.changes.scroll_explorer(viewport, delta);
    }

    pub(super) fn scroll_diff_by(&mut self, delta: isize) {
        self.changes
            .scroll_diff_by(self.regions.diff_scroll_max, delta);
    }

    fn scroll_diff_to(&mut self, row: u16) {
        let Some(track) = self.regions.diff_scrollbar else {
            return;
        };
        let Some(thumb) = self.regions.diff_scroll_thumb else {
            return;
        };
        self.changes.set_diff_scroll_from_track(
            row,
            track.y,
            track.height,
            thumb.height,
            self.diff_scroll_drag_offset,
            self.regions.diff_scroll_max,
        );
    }

    fn resize_worktree(&mut self, column: u16) {
        let Some(bounds) = self.regions.split_bounds else {
            return;
        };
        let minimum = bounds.x.saturating_add(24);
        let maximum = bounds.right().saturating_sub(25).max(minimum);
        let position = column.clamp(minimum, maximum);
        self.settings.worktree_width = position.saturating_sub(bounds.x);
    }

    fn resize_explorer_panes(&mut self, column: u16) {
        let Some(bounds) = self
            .regions
            .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::Overlay))
        else {
            return;
        };
        self.workspace_explorer.resize_panes(
            column,
            bounds.x.saturating_add(2),
            bounds.width.saturating_sub(4),
        );
        self.settings.explorer_left_pane_width = self.workspace_explorer.left_pane_width;
    }

    fn resize_agents(&mut self, row: u16) {
        let Some(bounds) = self.regions.agents_bounds else {
            return;
        };
        let top = row.clamp(bounds.y, bounds.bottom().saturating_sub(5));
        self.settings.agents_height = bounds.bottom().saturating_sub(top).max(5);
    }

    fn resize_graph_column(&mut self, drag: GraphColumnDrag, column: u16) {
        let requested = i32::from(column) - i32::from(drag.origin_x);
        let minimum = i32::from(drag.left.minimum_width()) - i32::from(drag.left_width);
        let maximum = i32::from(drag.right_width) - i32::from(drag.right.minimum_width());
        let delta = requested.clamp(minimum, maximum);
        let left_width = (i32::from(drag.left_width) + delta) as u16;
        let right_width = (i32::from(drag.right_width) - delta) as u16;
        self.settings.set_graph_column_width(drag.left, left_width);
        self.settings
            .set_graph_column_width(drag.right, right_width);
    }
}
