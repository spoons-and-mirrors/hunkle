use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::time::{Duration, Instant};

use crate::{repo_path::RepoPath, selection::SelectionOutcome};

use super::{
    ACTION_ITEMS, App, ExplorerHitTarget, GraphHitTarget, HitTarget, LeftPane,
    MINIMUM_WORKSPACE_PANEL_WIDTH, Mode, RepositoryBrowserEffect, RepositoryBrowserHitTarget, View,
    WorkspaceDropTarget, WorkspacePanelHitTarget, WorkspacePanelPlacement, WorktreeManagerEffect,
    WorktreeManagerHitTarget, changes::ChangesEffect, scroll_table,
};

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

impl App {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        if mouse.kind == MouseEventKind::Moved {
            self.hovered_hit_target = self.regions.hit_target_at(point);
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
        if self.dragging_workspace_panel_splitter {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.resize_workspace_panel(mouse.column);
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resize_workspace_panel(mouse.column);
                    self.dragging_workspace_panel_splitter = false;
                    self.persist_settings();
                }
                _ => {}
            }
            return;
        }
        if self.dragging_history {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => self.resize_history(mouse.row),
                MouseEventKind::Up(MouseButton::Left) => {
                    self.resize_history(mouse.row);
                    self.dragging_history = false;
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
                }
                _ => {}
            }
            return;
        }

        if self.workspace_panel.rename_dialog.is_some()
            || self.workspace_panel.delete_dialog.is_some()
            || self.workspace_panel.snapshot_load_dialog.is_some()
        {
            return;
        }
        if self.mode == Mode::WorktreeManager && self.worktree_manager.dialog_open() {
            return;
        }
        if self.mode == Mode::WorkspacePresets {
            self.handle_workspace_presets_mouse(mouse);
            return;
        }

        if self.workspace_panel.is_dragging_workspace() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    let target = self.workspace_drop_target(point);
                    self.workspace_panel.update_workspace_drag(target);
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    let effect = self.workspace_panel.finish_workspace_drag();
                    self.apply_workspace_panel_effect(effect);
                }
                _ => {}
            }
            return;
        }
        if (self.workspace_panel.create_menu_open || self.workspace_panel.snapshot_menu_open)
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            self.selection.clear();
            self.handle_workspace_panel_click(point);
            return;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(HitTarget::WorkspacePanel(target)) = self.regions.hit_target_at(point)
        {
            match target {
                WorkspacePanelHitTarget::Workspace(index)
                    if self.workspace_panel.begin_workspace_drag(index) =>
                {
                    self.mode = Mode::WorkspacePanel;
                    return;
                }
                WorkspacePanelHitTarget::Agent(_) => {
                    self.selection.clear();
                    self.mode = Mode::WorkspacePanel;
                    self.activate_workspace_panel_target(target);
                    return;
                }
                _ => {}
            }
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
        if self.mode == Mode::RepositoryBrowser {
            self.handle_repository_browser_mouse(mouse);
            return;
        }
        if self.mode == Mode::WorktreeManager {
            self.handle_worktree_manager_mouse(mouse);
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
        if self.mode == Mode::Explorer {
            self.handle_explorer_mouse(mouse);
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
        if self.mode == Mode::WorkspacePanel && mouse.kind == MouseEventKind::Moved {
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
        if matches!(
            self.mode,
            Mode::Normal | Mode::Commit | Mode::WorkspacePanel
        ) && self
            .regions
            .workspace_panel_splitter
            .is_some_and(|rect| rect.contains(point))
        {
            self.dragging_workspace_panel_splitter = true;
            self.resize_workspace_panel(point.x);
            return true;
        }
        if !matches!(self.mode, Mode::Normal | Mode::Commit) {
            return false;
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
            .history_splitter
            .is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Normal;
            self.dragging_history = true;
            self.changes.history_focused = true;
            self.resize_history(point.y);
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
            self.regions.hit_target_rect(HitTarget::RepositoryBrowser(
                RepositoryBrowserHitTarget::Overlay,
            )),
            self.regions.hit_target_rect(HitTarget::WorktreeManager(
                WorktreeManagerHitTarget::Overlay,
            )),
            self.regions.diff,
            self.regions.workspace_panel,
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
            Mode::Explorer => self.handle_explorer_mouse(mouse),
            Mode::FileSearch => self.handle_file_search_mouse(mouse),
            Mode::Settings => self.handle_settings_mouse(mouse),
            Mode::RepositoryBrowser => self.handle_repository_browser_mouse(mouse),
            Mode::WorktreeManager => self.handle_worktree_manager_mouse(mouse),
            Mode::AuthorFilter => self.handle_author_filter_mouse(mouse),
            Mode::Help => self.mode = Mode::Normal,
            Mode::Editor => {}
            Mode::Files => self.handle_file_dialog_click(point),
            Mode::WorkspacePanel => self.handle_workspace_panel_click(point),
            Mode::WorkspacePresets => self.handle_workspace_presets_mouse(mouse),
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
            Some(HitTarget::WorkspacePanel(target)) => {
                self.activate_workspace_panel_target(target);
                return;
            }
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
            self.toggle_changes_files();
            return;
        }
        if self
            .regions
            .changes
            .is_some_and(|rect| rect.contains(point))
        {
            self.view = View::Changes;
            self.graph_commit_open = false;
            self.show_graph_if_diff_empty();
        } else if self.regions.graph.is_some_and(|rect| rect.contains(point)) {
            self.view = match self.view {
                View::Changes => View::Graph,
                View::Graph => View::Changes,
            };
            self.graph_commit_open = false;
            self.show_graph_if_diff_empty();
        } else if self
            .regions
            .explorer
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_explorer();
        } else if self
            .regions
            .worktree_manager
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_worktree_manager();
        } else if self
            .regions
            .repository_browser
            .is_some_and(|rect| rect.contains(point))
        {
            self.open_repository_browser();
        } else if self
            .regions
            .settings
            .is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Settings;
        } else if self.regions.help.is_some_and(|rect| rect.contains(point)) {
            self.mode = Mode::Help;
        } else if self.select_explorer_row(point) {
            if self.changes.selected_explorer_directory_path().is_some() {
                let repo = self.session.data();
                self.changes.toggle_selected_explorer_directory(repo);
            } else {
                self.view = View::Changes;
                self.graph_commit_open = false;
            }
        } else if self.select_history_row(point) {
        } else if self.select_graph_row(point) {
            self.open_selected_graph_commit();
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

    fn apply_changes_effect(&mut self, effect: Option<ChangesEffect>) {
        match effect {
            Some(ChangesEffect::PaneActivated) => {
                self.mode = Mode::Normal;
                self.show_graph_if_diff_empty();
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
            Some(ChangesEffect::WorktreeFileSelected { path, staged }) => {
                if self.register_worktree_file_click(&path, staged)
                    && self.open_worktree_file_in_files(&path)
                {
                    return;
                }
                self.view = View::Changes;
                self.graph_commit_open = false;
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
        self.changes.set_pane(LeftPane::Files, Some(repo));
        self.mode = Mode::Normal;
        self.view = View::Changes;
        self.graph_commit_open = false;
        true
    }

    fn handle_workspace_panel_click(&mut self, point: Position) {
        let target = self.regions.hit_target_at(point);
        if self.workspace_panel.create_menu_open
            && !matches!(
                target,
                Some(HitTarget::WorkspacePanel(
                    WorkspacePanelHitTarget::CreateMenu
                        | WorkspacePanelHitTarget::CreateWorkspace
                        | WorkspacePanelHitTarget::CreateWorktree
                ))
            )
        {
            self.workspace_panel.close_create_menu();
            return;
        }
        if self.workspace_panel.snapshot_menu_open
            && !matches!(
                target,
                Some(HitTarget::WorkspacePanel(
                    WorkspacePanelHitTarget::SnapshotMenu
                        | WorkspacePanelHitTarget::SaveSnapshot
                        | WorkspacePanelHitTarget::Snapshot(_)
                ))
            )
        {
            self.workspace_panel.close_snapshot_menu();
            return;
        }
        if let Some(HitTarget::WorkspacePanel(target)) = target {
            self.activate_workspace_panel_target(target);
        } else if !self
            .regions
            .workspace_panel
            .is_some_and(|rect| rect.contains(point))
        {
            self.mode = Mode::Normal;
            self.handle_primary_left_click(point);
        }
    }

    fn activate_workspace_panel_target(&mut self, target: WorkspacePanelHitTarget) {
        match target {
            WorkspacePanelHitTarget::Focus => self.open_workspace_panel(),
            WorkspacePanelHitTarget::Collapse => {
                self.workspace_panel.hide();
                self.mode = Mode::Normal;
            }
            WorkspacePanelHitTarget::CreateMenu => {
                self.mode = Mode::WorkspacePanel;
                self.workspace_panel.toggle_create_menu();
            }
            WorkspacePanelHitTarget::CreateWorkspace => {
                let effect = self.workspace_panel.activate_create_choice(0);
                self.apply_workspace_panel_effect(effect);
            }
            WorkspacePanelHitTarget::CreateWorktree => {
                let effect = self.workspace_panel.activate_create_choice(1);
                self.apply_workspace_panel_effect(effect);
            }
            WorkspacePanelHitTarget::SnapshotMenu => {
                self.open_workspace_presets();
            }
            WorkspacePanelHitTarget::PresetOverlay => {}
            WorkspacePanelHitTarget::SaveSnapshot => {
                let effect = self.workspace_panel.activate_snapshot_choice(0);
                self.apply_workspace_panel_effect(effect);
            }
            WorkspacePanelHitTarget::Snapshot(index) => {
                let effect = self.workspace_panel.activate_snapshot_choice(index + 1);
                self.apply_workspace_panel_effect(effect);
            }
            WorkspacePanelHitTarget::Group(index) => self.workspace_panel.toggle_group(index),
            WorkspacePanelHitTarget::Workspace(index) => {
                let effect = self.workspace_panel.click_workspace(index);
                self.apply_workspace_panel_effect(effect);
            }
            WorkspacePanelHitTarget::Agent(index) => {
                self.mode = Mode::WorkspacePanel;
                let effect = self.workspace_panel.click_agent(index);
                self.apply_workspace_panel_effect(effect);
            }
        }
    }

    fn handle_workspace_presets_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        let target = self.regions.hit_target_at(point);
        match mouse.kind {
            MouseEventKind::Moved => match target {
                Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::SaveSnapshot)) => {
                    self.workspace_panel.select_snapshot_choice(0);
                }
                Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Snapshot(index))) => {
                    self.workspace_panel.select_snapshot_choice(index + 1);
                }
                _ => {}
            },
            MouseEventKind::ScrollUp => {
                let effect = self
                    .workspace_panel
                    .handle_workspace_presets(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                self.apply_workspace_panel_effect(effect);
            }
            MouseEventKind::ScrollDown => {
                let effect = self
                    .workspace_panel
                    .handle_workspace_presets(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                self.apply_workspace_panel_effect(effect);
            }
            MouseEventKind::Down(MouseButton::Left) => match target {
                Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::SaveSnapshot)) => {
                    let effect = self.workspace_panel.activate_snapshot_choice(0);
                    self.apply_workspace_panel_effect(effect);
                }
                Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Snapshot(index))) => {
                    let effect = self.workspace_panel.activate_snapshot_choice(index + 1);
                    self.apply_workspace_panel_effect(effect);
                }
                Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::PresetOverlay)) => {}
                _ => self.mode = Mode::Normal,
            },
            _ => {}
        }
    }

    fn workspace_drop_target(&self, point: Position) -> Option<WorkspaceDropTarget> {
        match self.regions.hit_target_at(point) {
            Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Group(index))) => {
                Some(WorkspaceDropTarget::Group(index))
            }
            Some(HitTarget::WorkspacePanel(WorkspacePanelHitTarget::Workspace(index))) => self
                .workspace_panel
                .group_for_workspace(index)
                .map(WorkspaceDropTarget::Group)
                .or(Some(WorkspaceDropTarget::Ungrouped)),
            _ if self
                .regions
                .workspace_panel
                .is_some_and(|rect| rect.contains(point)) =>
            {
                Some(WorkspaceDropTarget::Ungrouped)
            }
            _ => None,
        }
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

    fn handle_repository_browser_mouse(&mut self, mouse: MouseEvent) {
        if self.repository_browser.branch_delete_open() {
            return;
        }
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.repository_browser.move_selection(1),
            MouseEventKind::ScrollUp => self.repository_browser.move_selection(-1),
            MouseEventKind::Moved => {
                if let Some(HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::Item(index))) =
                    self.regions.hit_target_at(point)
                {
                    self.repository_browser.select(index);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => match self.regions.hit_target_at(point) {
                None => self.apply_repository_browser_effect(RepositoryBrowserEffect::Close),
                Some(HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::Tab(tab))) => {
                    self.repository_browser.set_tab(tab);
                }
                Some(HitTarget::RepositoryBrowser(RepositoryBrowserHitTarget::Item(index))) => {
                    let effect = self.repository_browser.activate(index);
                    self.apply_repository_browser_effect_option(effect);
                }
                Some(HitTarget::RepositoryBrowser(
                    RepositoryBrowserHitTarget::Overlay | RepositoryBrowserHitTarget::List,
                )) => {}
                Some(HitTarget::WorktreeManager(_)) => {}
                Some(HitTarget::Graph(_)) => {}
                Some(HitTarget::Explorer(_)) => {}
                Some(HitTarget::WorkspacePanel(_)) => {}
                Some(HitTarget::Changes(_)) => {}
                Some(HitTarget::CommitMessageGenerate) => {}
                Some(HitTarget::MarkdownPreviewToggle) => {}
            },
            _ => {}
        }
    }

    fn handle_worktree_manager_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && !self
                .regions
                .hit_target_rect(HitTarget::WorktreeManager(
                    WorktreeManagerHitTarget::Overlay,
                ))
                .is_some_and(|area| area.contains(point))
        {
            self.apply_worktree_manager_effect(WorktreeManagerEffect::Close);
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.worktree_manager.move_selection(1),
            MouseEventKind::ScrollUp => self.worktree_manager.move_selection(-1),
            MouseEventKind::Moved => {
                if let Some(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                    generation,
                    row,
                })) = self.regions.hit_target_at(point)
                {
                    self.worktree_manager.select_row(generation, row);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => match self.regions.hit_target_at(point) {
                Some(HitTarget::WorktreeManager(WorktreeManagerHitTarget::Item {
                    generation,
                    row,
                })) => {
                    let effect = self.worktree_manager.click_row(generation, row);
                    self.apply_worktree_manager_effect_option(effect);
                }
                None => {}
                Some(HitTarget::WorktreeManager(
                    WorktreeManagerHitTarget::Overlay | WorktreeManagerHitTarget::List,
                )) => {}
                Some(HitTarget::RepositoryBrowser(_)) => {}
                Some(HitTarget::Graph(_)) => {}
                Some(HitTarget::Explorer(_)) => {}
                Some(HitTarget::WorkspacePanel(_)) => {}
                Some(HitTarget::Changes(_)) => {}
                Some(HitTarget::CommitMessageGenerate) => {}
                Some(HitTarget::MarkdownPreviewToggle) => {}
            },
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
        let point = Position::new(mouse.column, mouse.row);
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        if self
            .regions
            .settings_overlay
            .is_some_and(|rect| !rect.contains(point))
        {
            self.mode = Mode::Normal;
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
            .workspace_panel_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 2;
            self.toggle_workspace_panel_enabled();
        } else if self
            .regions
            .agent_harness_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 3;
            self.toggle_agent_harness();
        } else if self
            .regions
            .media_preview_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 4;
            self.toggle_media_preview_protocol();
        } else if self
            .regions
            .editor_setting
            .is_some_and(|rect| rect.contains(point))
        {
            self.settings_selection = 5;
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

    fn select_history_row(&mut self, point: Position) -> bool {
        let Some(rect) = self
            .regions
            .history_list
            .filter(|rect| rect.contains(point))
        else {
            return false;
        };
        let Some(repo) = self.session.data() else {
            return false;
        };
        let relative_row = usize::from(point.y - rect.y);
        let selected = self.changes.select_history_row(repo, relative_row);
        if selected && self.view == View::Graph {
            self.graph_commit_open = true;
        }
        selected
    }

    fn select_graph_row(&mut self, point: Position) -> bool {
        if self.view != View::Graph {
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

    fn scroll_at(&mut self, point: Position, delta: isize) {
        if self
            .regions
            .workspace_panel_workspaces
            .is_some_and(|rect| rect.contains(point))
        {
            self.workspace_panel.move_workspace_selection(delta);
        } else if self
            .regions
            .workspace_panel_agents
            .is_some_and(|rect| rect.contains(point))
        {
            self.workspace_panel.move_agent_selection(delta);
        } else if self
            .regions
            .workspace_panel
            .is_some_and(|rect| rect.contains(point))
        {
            self.workspace_panel.handle_key(KeyEvent::new(
                if delta > 0 {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                },
                KeyModifiers::NONE,
            ));
        } else if self.regions.commit.is_some_and(|rect| rect.contains(point)) {
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
            .history_list
            .is_some_and(|rect| rect.contains(point))
        {
            if let Some(repo) = self.session.data() {
                self.changes.move_history_selection(repo, delta);
            }
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
    }

    fn resize_workspace_panel(&mut self, column: u16) {
        const MINIMUM_MAIN_WIDTH: u16 = 60;
        let Some(bounds) = self.regions.workspace_panel_bounds else {
            return;
        };
        self.settings.workspace_panel_width = match self.workspace_panel.placement {
            WorkspacePanelPlacement::Left => {
                let minimum = bounds.x.saturating_add(MINIMUM_WORKSPACE_PANEL_WIDTH);
                let maximum = bounds
                    .right()
                    .saturating_sub(MINIMUM_MAIN_WIDTH)
                    .saturating_sub(1)
                    .max(minimum);
                column.clamp(minimum, maximum).saturating_sub(bounds.x)
            }
            WorkspacePanelPlacement::Right => {
                let minimum = bounds.x.saturating_add(MINIMUM_MAIN_WIDTH);
                let maximum = bounds
                    .right()
                    .saturating_sub(MINIMUM_WORKSPACE_PANEL_WIDTH)
                    .saturating_sub(1)
                    .max(minimum);
                bounds
                    .right()
                    .saturating_sub(column.clamp(minimum, maximum))
                    .saturating_sub(1)
            }
            WorkspacePanelPlacement::Off => return,
        };
    }

    fn resize_history(&mut self, row: u16) {
        let Some(bounds) = self.regions.history_bounds else {
            return;
        };
        let top = row.clamp(bounds.y, bounds.bottom().saturating_sub(3));
        self.settings.history_height = bounds.bottom().saturating_sub(top).max(3);
    }
}
