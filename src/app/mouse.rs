use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::time::Instant;

use crate::{repo_path::RepoPath, selection::SelectionOutcome};

use super::{
    ACTION_ITEMS, AgentKey, AgentPreview, App, CloneField, DOUBLE_CLICK_INTERVAL,
    ExplorerHitTarget, FileSearchHitTarget, GraphColumnDrag, GraphHitTarget, HeaderPickerKind,
    HitTarget, LeftPane, MobileDragAxis, MobileScrollDrag, Mode, PreviewOrigin, ScrollTarget,
    SettingsHitTarget, View, changes::ChangesEffect, file_editor::FileEditor, scroll_table,
};

const AGENT_PREVIEW_SWIPE_THRESHOLD: u16 = 4;
const HEADER_SCROLL_THRESHOLD: u16 = 2;

impl App {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.mode == Mode::AgentPreview {
            let point = Position::new(mouse.column, mouse.row);
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.handle_agent_preview_modal_click(point)
                }
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    if let Some(
                        target @ (ScrollTarget::AgentTimeline(_)
                        | ScrollTarget::AgentTranscript(_)
                        | ScrollTarget::AgentScheduledTranscript(_)),
                    ) = self.regions.scroll_target_at(point)
                    {
                        let delta = if mouse.kind == MouseEventKind::ScrollUp {
                            -1
                        } else {
                            1
                        };
                        self.scroll_target(target, delta, true);
                    }
                }
                MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                    if let Some(target) = self.regions.scroll_target_at(point)
                        && matches!(
                            target,
                            ScrollTarget::AgentTimeline(_)
                                | ScrollTarget::AgentTranscript(_)
                                | ScrollTarget::AgentScheduledTranscript(_)
                        )
                    {
                        let effect = self.agent_preview.handle_horizontal_scroll(
                            &target,
                            mouse.kind == MouseEventKind::ScrollRight,
                        );
                        self.apply_agent_preview_effect(effect);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.mode == Mode::Scheduler {
            let point = Position::new(mouse.column, mouse.row);
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => self.handle_left_click(point),
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    if let Some(
                        target @ (ScrollTarget::SchedulerTasks
                        | ScrollTarget::SchedulerRuns
                        | ScrollTarget::SchedulerPrompt
                        | ScrollTarget::SchedulerDestinations),
                    ) = self.regions.scroll_target_at(point)
                    {
                        let delta = if mouse.kind == MouseEventKind::ScrollUp {
                            -1
                        } else {
                            1
                        };
                        self.scroll_target(target, delta, true);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.handle_mobile_scroll_gesture(mouse) {
            return;
        }
        self.handle_mouse_inner(mouse);
    }

    fn handle_mouse_inner(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(HitTarget::AgentPaneId(pane_id)) = self.regions.hit_target_at(point)
        {
            self.selection.clear();
            self.copy_request = Some(format!("herdr_pane_id {pane_id}"));
            return;
        }
        if self.agent_preview.picker_open
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && !matches!(
                self.regions.hit_target_at(point),
                Some(HitTarget::AgentPreviewPicker(_) | HitTarget::AgentPreviewPickerItem(_))
            )
        {
            self.agent_preview.close_picker();
        }
        if mouse.kind == MouseEventKind::Moved {
            self.hovered_hit_target = self.regions.hit_target_at(point);
            if let Some(target) = self.hovered_hit_target.as_ref() {
                let agent = match target {
                    HitTarget::Agent(key) | HitTarget::AgentStash(key) => Some(key),
                    HitTarget::AgentPreviewPicker(agent)
                    | HitTarget::AgentPreviewPickerItem(agent)
                    | HitTarget::AgentPreviewMessageTimeline(agent)
                    | HitTarget::AgentPreviewPrompt(agent)
                    | HitTarget::AgentPreviewPromptDelivery(agent)
                    | HitTarget::AgentPreviewRequest { agent, .. }
                    | HitTarget::AgentTooltip { agent, .. }
                    | HitTarget::AgentMessage { agent, .. } => Some(agent),
                    _ => None,
                };
                if let Some(index) = agent.and_then(|key| self.herdr.agent_index(key)) {
                    self.herdr.request_agent_latest_user_message(index);
                }
            }
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
                Some(HitTarget::HeaderIssue) => {
                    self.toggle_header_picker(HeaderPickerKind::Issues);
                    return;
                }
                Some(HitTarget::HeaderAgent) => {
                    self.start_header_agent();
                    return;
                }
                Some(HitTarget::HeaderSchedule) => {
                    self.open_scheduler();
                    return;
                }
                Some(HitTarget::HeaderFullscreen) => {
                    self.toggle_fullscreen();
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
                        Some(
                            HitTarget::HeaderPickerOverlay
                                | HitTarget::HeaderPickerItem(_)
                                | HitTarget::HeaderPickerDeleteBranch(_)
                                | HitTarget::HeaderPickerDeleteWorktree(_),
                        )
                    ) =>
                {
                    self.hovered_hit_target = None;
                    self.header_picker.scroll_by(3);
                }
                MouseEventKind::ScrollUp
                    if matches!(
                        self.regions.hit_target_at(point),
                        Some(
                            HitTarget::HeaderPickerOverlay
                                | HitTarget::HeaderPickerItem(_)
                                | HitTarget::HeaderPickerDeleteBranch(_)
                                | HitTarget::HeaderPickerDeleteWorktree(_),
                        )
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
                        Some(HitTarget::HeaderPickerIssueScope) => self.toggle_header_issue_scope(),
                        Some(HitTarget::HeaderPickerNewBranch) => {
                            self.begin_header_branch_creation()
                        }
                        Some(HitTarget::HeaderPickerOpenExplorer) => {
                            self.header_picker.close();
                            self.open_explorer();
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
                        Some(HitTarget::HeaderPickerDeleteWorktree(index)) => {
                            self.begin_header_worktree_deletion(index)
                        }
                        Some(HitTarget::HeaderPickerDeleteBranch(index)) => {
                            self.begin_header_branch_deletion(index)
                        }
                        Some(HitTarget::HeaderPickerConfirmDeleteBranch) => {
                            self.confirm_header_branch_deletion()
                        }
                        Some(HitTarget::HeaderPickerCancelDeleteBranch) => {
                            self.open_header_branches()
                        }
                        Some(HitTarget::HeaderPickerConfirmDeleteWorktree) => {
                            self.confirm_header_worktree_deletion()
                        }
                        Some(HitTarget::HeaderPickerCancelDeleteWorktree) => {
                            self.open_header_worktrees()
                        }
                        Some(HitTarget::HeaderPickerWorktreeName) => {
                            self.header_picker.worktree_name.focus()
                        }
                        Some(HitTarget::HeaderPickerOverlay) => {}
                        _ => self.header_picker.close(),
                    }
                }
                _ => {}
            }
            return;
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
        if self.mode == Mode::FileEdit {
            self.handle_file_editor_mouse(mouse, point);
            return;
        }

        if matches!(
            mouse.kind,
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
        ) {
            if self.regions.scroll_target_at(point) == Some(ScrollTarget::Header) {
                let delta = if mouse.kind == MouseEventKind::ScrollRight {
                    1
                } else {
                    -1
                };
                self.scroll_target(ScrollTarget::Header, delta, true);
                return;
            }
            if let Some(target) = self.regions.scroll_target_at(point)
                && matches!(
                    target,
                    ScrollTarget::AgentTimeline(_)
                        | ScrollTarget::AgentTranscript(_)
                        | ScrollTarget::AgentScheduledTranscript(_)
                )
            {
                let effect = self
                    .agent_preview
                    .handle_horizontal_scroll(&target, mouse.kind == MouseEventKind::ScrollRight);
                self.apply_agent_preview_effect(effect);
                return;
            }
        }

        if matches!(
            mouse.kind,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
        ) {
            let delta = if mouse.kind == MouseEventKind::ScrollDown {
                1
            } else {
                -1
            };
            if let Some(target) = self.regions.scroll_target_at(point) {
                self.scroll_target(target, delta, true);
            }
            return;
        }

        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            match self.regions.hit_target_at(point) {
                Some(HitTarget::Agent(key)) => {
                    let Some(index) = self.herdr.agent_index(&key) else {
                        return;
                    };
                    self.selection.clear();
                    self.handle_agent_card_action(
                        key,
                        index,
                        mouse.modifiers.contains(KeyModifiers::CONTROL),
                    );
                    return;
                }
                Some(HitTarget::AgentListModeToggle) => {
                    self.herdr.cycle_agent_list_mode();
                    return;
                }
                Some(HitTarget::AgentScheduledRun(run_id)) => {
                    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                        self.promote_scheduled_run(run_id);
                    } else {
                        self.open_scheduled_run_preview(run_id);
                    }
                    return;
                }
                Some(HitTarget::AgentStash(key)) => {
                    let Some(index) = self.herdr.agent_index(&key) else {
                        return;
                    };
                    self.stash_agent(index);
                    return;
                }
                Some(HitTarget::StashedAgent(index)) => {
                    self.restore_stashed_agent(index);
                    return;
                }
                Some(target) if AgentPreview::owns_target(&target) => {
                    let effect = self.agent_preview.activate_target(&target);
                    self.apply_agent_preview_effect(effect);
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
        if self.mode == Mode::Explorer {
            self.handle_explorer_mouse(mouse);
            return;
        }
        if self.mode == Mode::Command {
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
        if self.mode == Mode::Normal && self.view() == View::RepositorySearch {
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

        if let MouseEventKind::Down(MouseButton::Right) = mouse.kind {
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
    }

    fn handle_mobile_scroll_gesture(&mut self, mouse: MouseEvent) -> bool {
        if self.mode == Mode::FileEdit {
            self.mobile_scroll_drag = None;
            return false;
        }
        if self.dragging_splitter
            || self.dragging_agents
            || self.dragging_diff_scrollbar
            || self.dragging_graph_column.is_some()
            || self.workspace_explorer.dragging_splitter
            || self.file_drag.is_some()
        {
            self.mobile_scroll_drag = None;
            return false;
        }
        let point = Position::new(mouse.column, mouse.row);
        if let Some(mut drag) = self.mobile_scroll_drag.clone() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left)
                | MouseEventKind::Moved
                | MouseEventKind::Up(MouseButton::Left) => {
                    if point != drag.previous {
                        let horizontal = drag.start.x.abs_diff(point.x);
                        let vertical = drag.start.y.abs_diff(point.y);
                        let header = drag.scroll_target == Some(ScrollTarget::Header);
                        let vertical_threshold = if header { HEADER_SCROLL_THRESHOLD } else { 1 };
                        if drag.axis.is_none() {
                            drag.axis = if (drag.agent_preview.is_some()
                                && horizontal >= AGENT_PREVIEW_SWIPE_THRESHOLD
                                && horizontal > vertical)
                                || (header
                                    && horizontal >= HEADER_SCROLL_THRESHOLD
                                    && horizontal > vertical)
                            {
                                Some(MobileDragAxis::Horizontal)
                            } else if vertical >= vertical_threshold
                                && (drag.agent_preview.is_none() || vertical >= horizontal)
                            {
                                Some(MobileDragAxis::Vertical)
                            } else {
                                None
                            };
                        }
                        if drag.axis.is_none() && header {
                            if mouse.kind != MouseEventKind::Up(MouseButton::Left) {
                                self.mobile_scroll_drag = Some(drag);
                                return true;
                            }
                        } else {
                            drag.moved = true;
                            if drag.axis == Some(MobileDragAxis::Vertical) {
                                let delta = drag.previous.y as isize - point.y as isize;
                                if delta != 0 {
                                    if let Some(target) = drag
                                        .scroll_target
                                        .clone()
                                        .filter(|target| *target != ScrollTarget::Header)
                                    {
                                        self.scroll_target(target, delta, false);
                                    }
                                }
                            } else if drag.axis == Some(MobileDragAxis::Horizontal)
                                && drag.scroll_target == Some(ScrollTarget::Header)
                            {
                                let delta = drag.previous.x as isize - point.x as isize;
                                if delta != 0 {
                                    self.scroll_target(ScrollTarget::Header, delta, false);
                                }
                            }
                            drag.previous = point;
                        }
                    }
                    let released = mouse.kind == MouseEventKind::Up(MouseButton::Left);
                    if released {
                        self.mobile_scroll_drag = None;
                        if drag.axis == Some(MobileDragAxis::Horizontal) {
                            let horizontal = drag.start.x.abs_diff(point.x);
                            let vertical = drag.start.y.abs_diff(point.y);
                            if horizontal >= AGENT_PREVIEW_SWIPE_THRESHOLD
                                && horizontal > vertical
                                && let Some(agent) = drag.agent_preview.as_ref()
                            {
                                let target = ScrollTarget::AgentTimeline(agent.clone());
                                let effect = self
                                    .agent_preview
                                    .handle_horizontal_scroll(&target, point.x < drag.start.x);
                                self.apply_agent_preview_effect(effect);
                            }
                        } else if !drag.moved {
                            self.handle_mouse_inner(MouseEvent {
                                kind: MouseEventKind::Down(MouseButton::Left),
                                column: drag.start.x,
                                row: drag.start.y,
                                modifiers: drag.modifiers,
                            });
                            self.handle_mouse_inner(MouseEvent {
                                kind: MouseEventKind::Up(MouseButton::Left),
                                column: drag.start.x,
                                row: drag.start.y,
                                modifiers: drag.modifiers,
                            });
                        }
                    } else {
                        self.mobile_scroll_drag = Some(drag);
                    }
                    return true;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.mobile_scroll_drag = None;
                }
                _ => return true,
            }
        }
        if !self.layout_profile().is_single()
            || mouse.kind != MouseEventKind::Down(MouseButton::Left)
        {
            return false;
        }
        self.selection.clear();
        let scroll_target = self.regions.scroll_target_at(point);
        if !self.regions.has_hard_scroll_capture() && self.begin_mouse_control(point) {
            return true;
        }
        let agent_preview = (!self.regions.has_scroll_capture())
            .then(|| self.agent_preview_at(point))
            .flatten();
        self.mobile_scroll_drag = Some(MobileScrollDrag {
            start: point,
            previous: point,
            moved: false,
            axis: None,
            agent_preview,
            scroll_target,
            modifiers: mouse.modifiers,
        });
        true
    }

    fn agent_preview_at(&self, point: Position) -> Option<AgentKey> {
        if !self.agents_pane_visible() || !self.single_panel_detail_visible() {
            return None;
        }
        match self.regions.hit_target_at(point) {
            Some(
                HitTarget::AgentTooltip { agent, .. }
                | HitTarget::AgentMessage { agent, .. }
                | HitTarget::AgentExpandedMessage { agent, .. }
                | HitTarget::AgentPreviewMessageTimeline(agent)
                | HitTarget::AgentPreviewRequest { agent, .. },
            ) => Some(agent),
            _ => None,
        }
    }

    fn scroll_target(&mut self, target: ScrollTarget, delta: isize, wheel: bool) {
        let wheel_amount = |amount: isize| amount.saturating_mul(if wheel { 3 } else { 1 });
        match target {
            ScrollTarget::Header => {
                self.header_scroll = self
                    .header_scroll
                    .saturating_add_signed(wheel_amount(delta))
                    .min(self.regions.header_scroll_max);
            }
            ScrollTarget::HeaderPicker => {
                self.hovered_hit_target = None;
                self.header_picker.scroll_by(wheel_amount(delta));
            }
            ScrollTarget::ActionMenu => self.actions.move_selection(delta),
            ScrollTarget::AuthorFilter => self.author_filter.move_selection(delta),
            ScrollTarget::WorkspaceExplorer => {
                if self.workspace_explorer.editing_path {
                    self.workspace_explorer.move_match_selection(delta);
                } else {
                    self.workspace_explorer.move_selection(delta);
                }
            }
            ScrollTarget::WorkspaceExplorerSurroundings => {
                self.workspace_explorer.move_surrounding_selection(delta);
            }
            ScrollTarget::CommandOutput => self.actions.scroll_by(wheel_amount(delta)),
            ScrollTarget::SettingsShortcuts => {
                let key = if delta < 0 {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                for _ in 0..wheel_amount(delta).unsigned_abs() {
                    self.handle_shortcut_settings(KeyEvent::new(key, KeyModifiers::NONE));
                }
            }
            ScrollTarget::SchedulerTasks
            | ScrollTarget::SchedulerRuns
            | ScrollTarget::SchedulerPrompt
            | ScrollTarget::SchedulerDestinations => self.scroll_scheduler(target, delta),
            ScrollTarget::Commit => self.scroll_commit(delta, wheel),
            ScrollTarget::Worktree => self.scroll_worktree(wheel_amount(delta)),
            ScrollTarget::Explorer => self.scroll_explorer(wheel_amount(delta)),
            ScrollTarget::Agents => self.herdr.scroll_agents(delta),
            ScrollTarget::Preview => self.scroll_diff_by(wheel_amount(delta)),
            ScrollTarget::SqliteObjects => {
                let viewport = self
                    .regions
                    .sqlite_objects
                    .map_or(0, |rect| usize::from(rect.height));
                self.changes
                    .scroll_sqlite_objects(viewport, wheel_amount(delta));
            }
            ScrollTarget::SqliteRows => {
                let viewport = self
                    .regions
                    .sqlite_rows
                    .map_or(0, |rect| usize::from(rect.height));
                self.changes
                    .scroll_sqlite_rows(viewport, wheel_amount(delta));
            }
            ScrollTarget::Graph => self.scroll_graph(wheel_amount(delta)),
            ScrollTarget::RepositorySearch => self.file_search.move_selection(delta),
            target @ (ScrollTarget::AgentTimeline(_)
            | ScrollTarget::AgentTranscript(_)
            | ScrollTarget::AgentScheduledTranscript(_)) => {
                let live = match &target {
                    ScrollTarget::AgentTimeline(key) | ScrollTarget::AgentTranscript(key) => {
                        self.agent_preview_live_context_for_key(key)
                    }
                    _ => None,
                };
                let maximum = self
                    .regions
                    .scroll_state(&target)
                    .map_or(0, |state| state.maximum);
                let effect =
                    self.agent_preview
                        .handle_scroll(&target, wheel_amount(delta), live, maximum);
                self.apply_agent_preview_effect(effect);
            }
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
            && !self.layout_profile().is_single()
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
            self.regions.file_search,
            self.regions.file_dialog_overlay,
            self.regions
                .hit_target_rect(HitTarget::Explorer(ExplorerHitTarget::Overlay)),
            self.regions
                .hit_target_rect(HitTarget::Settings(SettingsHitTarget::Overlay)),
            self.regions
                .hit_target_rect(HitTarget::AgentPreviewModalOverlay),
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
            Mode::Command => {}
            Mode::HerdrPrompt => {}
            Mode::FileEdit => self.place_file_editor_cursor(point, false),
            Mode::Explorer => self.handle_explorer_mouse(mouse),
            Mode::Settings => self.handle_settings_mouse(mouse),
            Mode::AuthorFilter => self.handle_author_filter_mouse(mouse),
            Mode::Help => self.mode = Mode::Normal,
            Mode::Editor => {}
            Mode::Files => self.handle_file_dialog_click(point),
            Mode::Scheduler => {
                if let Some(HitTarget::Scheduler(target)) = self.regions.hit_target_at(point) {
                    self.activate_scheduler_target(target);
                }
            }
            Mode::AgentPreview => self.handle_agent_preview_modal_click(point),
            Mode::Normal if self.view() == View::RepositorySearch => {
                let global_navigation = [
                    self.regions.graph,
                    self.regions.left_pane_toggle,
                    self.regions.explorer,
                    self.regions.settings,
                    self.regions.help,
                ]
                .into_iter()
                .flatten()
                .any(|rect| rect.contains(point));
                if global_navigation {
                    self.file_search.close();
                    self.navigation.close_search();
                    self.handle_primary_left_click(point);
                } else {
                    self.handle_file_search_mouse(mouse);
                }
            }
            Mode::Normal | Mode::Commit => self.handle_primary_left_click(point),
        }
    }

    pub(super) fn handle_primary_left_click(&mut self, point: Position) {
        let target = self.regions.hit_target_at(point);
        if target == Some(HitTarget::Graph(GraphHitTarget::Search)) {
            self.focus_graph_search();
            return;
        }
        self.graph_search_focused = false;
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
        match target {
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
            Some(HitTarget::Agent(key)) => {
                let Some(index) = self.herdr.agent_index(&key) else {
                    return;
                };
                self.handle_agent_card_action(key, index, false);
                return;
            }
            Some(HitTarget::AgentListModeToggle) => {
                self.herdr.cycle_agent_list_mode();
                return;
            }
            Some(HitTarget::AgentScheduledRun(run_id)) => {
                self.open_scheduled_run_preview(run_id);
                return;
            }
            Some(HitTarget::AgentStash(key)) => {
                let Some(index) = self.herdr.agent_index(&key) else {
                    return;
                };
                self.stash_agent(index);
                return;
            }
            Some(HitTarget::StashedAgent(index)) => {
                self.restore_stashed_agent(index);
                return;
            }
            Some(target) if AgentPreview::owns_target(&target) => {
                let effect = self.agent_preview.activate_target(&target);
                self.apply_agent_preview_effect(effect);
                return;
            }
            Some(
                HitTarget::AgentTooltip { .. }
                | HitTarget::AgentMessage { .. }
                | HitTarget::AgentExpandedMessage { .. }
                | HitTarget::AgentScheduledMessage { .. },
            ) => return,
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
            self.show_previous_panel();
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
                self.show_detail_panel();
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
        if self.regions.preview_generation != self.changes.preview.generation() {
            self.notice = Some("Preview changed; click again to edit".to_owned());
            return;
        }
        let rendered_row = self
            .regions
            .preview_scroll
            .saturating_add(usize::from(point.y.saturating_sub(body.y)));
        let width = usize::from(body.width);
        let rendered_column = usize::from(point.x.saturating_sub(body.x));

        if matches!(
            self.changes.preview.origin(),
            PreviewOrigin::ExplorerFile { .. }
        ) {
            let Some(path) = self.regions.preview_path.clone() else {
                return;
            };
            let Some(content) = self.changes.preview.text() else {
                return;
            };
            let gutter = usize::from(width >= 72) * 7;
            let Some((line, column)) = self
                .changes
                .preview_presentation
                .source_position_at_rendered_position(
                    content,
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
            let Some(content) = self.changes.preview.text() else {
                return;
            };
            let Some((display_line, column)) = self
                .changes
                .preview_presentation
                .source_position_at_rendered_position(content, rendered_row, rendered_column, 0)
            else {
                return;
            };
            let Some(source_line) = display_line.checked_sub(2) else {
                self.notice = Some("Click a source line to edit this file".to_owned());
                return;
            };
            let Some(content) = self.changes.preview.text() else {
                return;
            };
            let displayed = self
                .changes
                .preview_presentation
                .source_line(content, display_line.saturating_sub(1));
            let next = self
                .changes
                .preview_presentation
                .source_line(content, display_line);
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
        let Some(document) = self.changes.preview.document() else {
            return;
        };
        let position = self.regions.preview_path.clone().and_then(|path| {
            self.changes
                .preview_presentation
                .diff_position_at_rendered_position(document, rendered_row, rendered_column, gutter)
                .map(|(line, column)| (path, line, column))
        });
        let position = position.or_else(|| {
            self.changes
                .preview_presentation
                .diff_file_position_at_rendered_position(
                    document,
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
                self.navigation.close_changes_detail();
            }
            Some(ChangesEffect::PaneActivated) => {
                self.dismiss_agent_preview();
                self.last_worktree_file_click = None;
                self.mode = Mode::Normal;
                self.show_detail_panel();
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
                self.show_detail_panel();
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

    pub(super) fn show_agent(&mut self, index: usize) {
        if let Err(error) = self.herdr.show_agent(index) {
            self.notice = Some(error);
        }
    }

    pub(super) fn activate_agent_card(&mut self, key: AgentKey, index: usize) {
        if self.herdr.fullscreen() {
            let double_click = self
                .last_agent_click
                .as_ref()
                .is_some_and(|(previous, at)| {
                    previous == &key && at.elapsed() <= DOUBLE_CLICK_INTERVAL
                });
            self.last_agent_click = (!double_click).then(|| (key.clone(), Instant::now()));
            if double_click {
                match self.herdr.toggle_fullscreen() {
                    Ok(()) => self.pending_fullscreen_agent = Some(key),
                    Err(error) => {
                        self.pending_fullscreen_agent = None;
                        self.notice = Some(format!("Could not toggle fullscreen: {error}"));
                    }
                }
            } else if let Some(path) = self
                .herdr
                .agent_destination(index)
                .map(|path| path.to_path_buf())
            {
                self.queue_workspace_restore(path);
            } else {
                self.notice = Some("Agent has not reported its working directory".to_owned());
            }
            return;
        }

        self.last_agent_click = None;
        if self.layout_profile().is_single() && self.agents_pane_visible() {
            self.open_agent_detail(index);
        } else {
            self.show_agent(index);
        }
    }

    fn handle_agent_card_action(&mut self, key: AgentKey, index: usize, control: bool) {
        if self.settings.agent_card_click_action.opens_preview(control) {
            self.open_agent_preview_modal(index);
        } else {
            self.activate_agent_card(key, index);
        }
    }

    pub(super) fn select_agent_preview(&mut self, index: usize) {
        let Some(key) = self.herdr.agent_key(index) else {
            return;
        };
        self.agent_preview.select_agent(key.clone());
        self.hovered_hit_target = self
            .herdr
            .agent_user_messages(index)
            .filter(|messages| !messages.is_empty())
            .map_or(Some(HitTarget::Agent(key.clone())), |messages| {
                Some(HitTarget::AgentTooltip {
                    agent: key,
                    message: messages.len() - 1,
                })
            });
        self.herdr.request_agent_latest_user_message(index);
    }

    fn handle_agent_preview_modal_click(&mut self, point: Position) {
        let Some(target) = self.regions.hit_target_at(point) else {
            return;
        };
        if AgentPreview::owns_target(&target) {
            let effect = self.agent_preview.activate_target(&target);
            self.apply_agent_preview_effect(effect);
        }
    }

    fn handle_action_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
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
                        self.graph_search
                            .apply(self.author_filter.visible_indices());
                        self.select_current_graph_search_match();
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

    fn handle_explorer_mouse(&mut self, mouse: MouseEvent) {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
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
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(HitTarget::FileSearch(target)) = self.regions.hit_target_at(point) else {
                    self.last_file_search_click = None;
                    return;
                };
                match target {
                    FileSearchHitTarget::Result { generation, row } => {
                        if !self.file_search.select(generation, row) {
                            return;
                        }
                        let Some(destination) = self.file_search.selected_destination() else {
                            return;
                        };
                        let double_click =
                            self.last_file_search_click
                                .as_ref()
                                .is_some_and(|(previous, at)| {
                                    previous == &destination
                                        && at.elapsed() <= DOUBLE_CLICK_INTERVAL
                                });
                        self.last_file_search_click =
                            (!double_click).then(|| (destination, Instant::now()));
                        if double_click {
                            self.activate_file_search_result();
                        }
                    }
                    target => {
                        self.last_file_search_click = None;
                        let Some(repo) = self.session.data() else {
                            return;
                        };
                        match target {
                            FileSearchHitTarget::Scope(scope) => {
                                self.file_search.set_scope(scope, repo)
                            }
                            FileSearchHitTarget::CaseSensitive => {
                                self.file_search.toggle_case(repo)
                            }
                            FileSearchHitTarget::WholeWord => {
                                self.file_search.toggle_whole_word(repo)
                            }
                            FileSearchHitTarget::Regex => self.file_search.toggle_regex(repo),
                            FileSearchHitTarget::IncludeIgnored => {
                                self.file_search.toggle_ignored(repo)
                            }
                            FileSearchHitTarget::Result { .. } => unreachable!(),
                        }
                    }
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
        let Some(HitTarget::Settings(target)) = self.regions.hit_target_at(point) else {
            self.close_settings();
            return;
        };
        self.activate_settings_target(target);
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

    fn scroll_commit(&mut self, delta: isize, wheel: bool) {
        let amount = delta.saturating_mul(if wheel { 2 } else { 1 });
        let current = self.regions.commit_scroll;
        let next = if amount < 0 {
            current.saturating_sub(amount.unsigned_abs())
        } else {
            current.saturating_add(amount as usize)
        }
        .min(self.regions.commit_scroll_max);
        self.commit_scroll = Some(next);
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
