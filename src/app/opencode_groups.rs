//! Hunkle-local organization of externally owned OpenCode tabs.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    time::Instant,
};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;
use serde::{Deserialize, Serialize};

use super::opencode_tabs::OpenCodeTab;
use super::{App, DOUBLE_CLICK_INTERVAL, HitTarget, ScrollTarget, TextInput};
use crate::filesystem::atomic_write;

#[cfg(test)]
mod tests;

#[derive(Serialize, Deserialize)]
pub(crate) struct Group {
    pub id: u64,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct GroupCatalog {
    pub groups: Vec<Group>,
    pub assignments: BTreeMap<String, u64>,
    // Preserve older local labels in existing catalogs; OpenCode now owns session titles.
    pub session_names: BTreeMap<String, String>,
    pub card_order: BTreeMap<u64, Vec<String>>,
    pub hidden: BTreeSet<u64>,
    pub solo: Option<u64>,
}

impl Default for GroupCatalog {
    fn default() -> Self {
        Self {
            groups: vec![Group {
                id: 1,
                name: "General".into(),
            }],
            assignments: BTreeMap::new(),
            session_names: BTreeMap::new(),
            card_order: BTreeMap::new(),
            hidden: BTreeSet::new(),
            solo: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NameTarget {
    Group(Option<u64>),
    Session {
        instance_id: String,
        session_id: String,
    },
}

pub(crate) struct GroupEdit {
    pub target: NameTarget,
    pub input: TextInput,
}

enum DragSource {
    Card {
        instance_id: String,
        session_id: String,
    },
    Group(u64),
}

pub(crate) struct StripDrag {
    source: DragSource,
    start: Position,
    previous: Position,
    pub moved: bool,
    pub target: Option<u64>,
    pub card_target: Option<String>,
}

impl StripDrag {
    pub(crate) fn is_group(&self) -> bool {
        matches!(self.source, DragSource::Group(_))
    }
}

pub(crate) struct OpenCodeGroups {
    pub catalog: GroupCatalog,
    path: Option<PathBuf>,
    pub edit: Option<GroupEdit>,
    pub drag: Option<StripDrag>,
    pub scroll: usize,
    last_click: Option<(u64, Instant)>,
}

impl OpenCodeGroups {
    pub(crate) fn load(path: Option<PathBuf>) -> Self {
        let catalog = path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<GroupCatalog>(&bytes).ok())
            .filter(|catalog| !catalog.groups.is_empty())
            .unwrap_or_default();
        Self {
            catalog,
            path,
            edit: None,
            drag: None,
            scroll: 0,
            last_click: None,
        }
    }

    pub(crate) fn group_for(&self, session_id: &str) -> u64 {
        self.catalog
            .assignments
            .get(session_id)
            .copied()
            .filter(|id| self.catalog.groups.iter().any(|group| group.id == *id))
            // The oldest group remains the default even after reordering.
            .unwrap_or_else(|| {
                self.catalog
                    .groups
                    .iter()
                    .map(|group| group.id)
                    .min()
                    .unwrap()
            })
    }

    pub(crate) fn visible(&self, group: u64) -> bool {
        self.catalog.solo.map_or_else(
            || !self.catalog.hidden.contains(&group),
            |solo| solo == group,
        )
    }

    /// Order only the local presentation. Unknown tabs retain OpenCode's order.
    pub(crate) fn ordered_tabs(&self, tabs: &[OpenCodeTab]) -> Vec<usize> {
        let mut indices = (0..tabs.len()).collect::<Vec<_>>();
        indices.sort_by_key(|&index| {
            let group = self.group_for(&tabs[index].session_id);
            let position = self
                .catalog
                .groups
                .iter()
                .position(|item| item.id == group)
                .unwrap();
            let rank = self
                .catalog
                .card_order
                .get(&group)
                .and_then(|order| order.iter().position(|id| id == &tabs[index].session_id));
            (position, rank.unwrap_or(usize::MAX))
        });
        indices
    }

    fn reorder_card(&mut self, tabs: &[OpenCodeTab], session: &str, target: &str) -> bool {
        let group = self.group_for(session);
        if session == target || group != self.group_for(target) {
            return false;
        }
        let mut order = self
            .ordered_tabs(tabs)
            .into_iter()
            .filter(|&index| self.group_for(&tabs[index].session_id) == group)
            .map(|index| tabs[index].session_id.clone())
            .collect::<Vec<_>>();
        let Some(from) = order.iter().position(|id| id == session) else {
            return false;
        };
        let Some(to) = order.iter().position(|id| id == target) else {
            return false;
        };
        let moved = order.remove(from);
        order.insert(to, moved);
        if from == to {
            return false;
        }
        // Keep positions for tabs temporarily closed in OpenCode.
        if let Some(previous) = self.catalog.card_order.get(&group) {
            for id in previous {
                if !order.contains(id) {
                    order.push(id.clone());
                }
            }
        }
        self.catalog.card_order.insert(group, order);
        true
    }

    fn toggle_hidden(&mut self, group: u64) {
        if !self.catalog.hidden.remove(&group) {
            self.catalog.hidden.insert(group);
        }
    }

    fn click(&mut self, group: u64) {
        let double = self.last_click.take().is_some_and(|(previous, at)| {
            previous == group && at.elapsed() < DOUBLE_CLICK_INTERVAL
        });
        if double {
            if self.catalog.solo.is_some() {
                self.catalog.solo = None;
            } else {
                // Undo the first click, preserving the pre-isolation visibility choices.
                self.toggle_hidden(group);
                self.catalog.solo = Some(group);
            }
        } else {
            self.last_click = Some((group, Instant::now()));
            if self.catalog.solo.is_some() {
                self.catalog.solo = Some(group);
            } else {
                self.toggle_hidden(group);
            }
        }
    }

    fn begin_edit(&mut self, id: Option<u64>) {
        let mut input = TextInput::default();
        input.set(
            id.and_then(|id| self.catalog.groups.iter().find(|group| group.id == id))
                .map(|group| group.name.clone())
                .unwrap_or_else(|| format!("Group {}", self.catalog.groups.len() + 1)),
        );
        input.select_all();
        self.edit = Some(GroupEdit {
            target: NameTarget::Group(id),
            input,
        });
        self.last_click = None;
    }

    fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(path, &serde_json::to_vec_pretty(&self.catalog)?)
    }
}

impl App {
    fn persist_opencode_groups(&mut self) {
        if let Err(error) = self.opencode_groups.save() {
            self.notice = Some(format!("Could not save tab groups: {error}"));
        }
    }

    pub(super) fn handle_opencode_group_key(&mut self, key: KeyEvent) -> bool {
        if self.opencode_groups.drag.take().is_some() && key.code == KeyCode::Esc {
            return true;
        }
        if self.opencode_groups.edit.is_none()
            && key.code == KeyCode::F(2)
            && key.modifiers.is_empty()
            && let Some(target) = self.hovered_hit_target.clone()
            && self.regions.hit_target_rect(target.clone()).is_some()
        {
            match target {
                HitTarget::OpenCodeGroup(id) => {
                    self.opencode_groups.begin_edit(Some(id));
                    return true;
                }
                HitTarget::OpenCodeTab {
                    instance_id,
                    session_id,
                } => {
                    let Some(tab) = self
                        .opencode_presence
                        .tabs
                        .items
                        .iter()
                        .find(|tab| tab.session_id == session_id)
                    else {
                        return false;
                    };
                    let mut input = TextInput::default();
                    input.set(tab.title.clone());
                    input.select_all();
                    self.opencode_groups.edit = Some(GroupEdit {
                        target: NameTarget::Session {
                            instance_id,
                            session_id,
                        },
                        input,
                    });
                    return true;
                }
                _ => {}
            }
        }
        let Some(edit) = &mut self.opencode_groups.edit else {
            return false;
        };
        match key.code {
            KeyCode::Esc => self.opencode_groups.edit = None,
            KeyCode::Enter => self.finish_opencode_group_edit(),
            _ => {
                edit.input.handle_edit_key(key);
            }
        }
        true
    }

    fn finish_opencode_group_edit(&mut self) {
        let Some(edit) = &self.opencode_groups.edit else {
            return;
        };
        let name = edit
            .input
            .text()
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>();
        let name = name.trim();
        if name.is_empty() {
            self.notice = Some("Give the tab or group a name, or press Esc to cancel".into());
            return;
        }
        if let NameTarget::Session {
            instance_id,
            session_id,
        } = &edit.target
        {
            match self
                .opencode_presence
                .rename_tab(instance_id, session_id, name)
            {
                Ok(()) => self.opencode_groups.edit = None,
                Err(error) => self.notice = Some(format!("Could not rename OpenCode tab: {error}")),
            }
            return;
        }
        let catalog = &mut self.opencode_groups.catalog;
        match &edit.target {
            NameTarget::Session { .. } => unreachable!(),
            NameTarget::Group(Some(id)) => {
                if let Some(group) = catalog.groups.iter_mut().find(|group| group.id == *id) {
                    group.name = name.to_owned();
                }
            }
            NameTarget::Group(None) => {
                let id = catalog
                    .groups
                    .iter()
                    .map(|group| group.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                catalog.groups.push(Group {
                    id,
                    name: name.to_owned(),
                });
                // Reveal the newly created chip, including in narrow layouts.
                self.opencode_groups.scroll = usize::MAX;
            }
        }
        self.opencode_groups.edit = None;
        self.persist_opencode_groups();
    }

    pub(super) fn handle_opencode_strip_mouse(&mut self, mouse: MouseEvent) -> bool {
        let point = Position::new(mouse.column, mouse.row);
        let hit = self.regions.hit_target_at(point);
        if self.opencode_groups.edit.is_some() {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                match hit {
                    Some(HitTarget::OpenCodeGroupSave) => self.finish_opencode_group_edit(),
                    Some(HitTarget::OpenCodeGroupName) => {
                        self.opencode_groups.edit.as_mut().unwrap().input.focus();
                    }
                    _ => self.opencode_groups.edit = None,
                }
            }
            return true;
        }

        // Strip scrolling also remains available while dragging a card or editing a file.
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ) && let Some(target @ (ScrollTarget::OpenCodeTabs | ScrollTarget::OpenCodeGroups)) =
            self.regions.scroll_target_at(point)
        {
            let delta = if matches!(
                mouse.kind,
                MouseEventKind::ScrollLeft | MouseEventKind::ScrollUp
            ) {
                -1
            } else {
                1
            };
            self.scroll_target(target, delta, true);
            return true;
        }

        if let Some(mut drag) = self.opencode_groups.drag.take() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left)
                | MouseEventKind::Moved
                | MouseEventKind::Up(MouseButton::Left) => {
                    drag.moved |= drag.start.x.abs_diff(point.x) >= 2 || drag.start.y != point.y;
                    drag.target = match &hit {
                        Some(HitTarget::OpenCodeGroup(id)) => Some(*id),
                        _ => None,
                    };
                    drag.card_target = match (&drag.source, &hit) {
                        (
                            DragSource::Card { instance_id, .. },
                            Some(HitTarget::OpenCodeTab {
                                instance_id: target_instance,
                                session_id,
                            }),
                        ) if instance_id == target_instance => Some(session_id.clone()),
                        _ => None,
                    };
                    if drag.moved {
                        self.opencode_groups.last_click = None;
                    }
                    if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                        match drag.source {
                            DragSource::Group(id) => {
                                if drag.moved {
                                    if let Some(target) = drag.target {
                                        let groups = &mut self.opencode_groups.catalog.groups;
                                        let from =
                                            groups.iter().position(|group| group.id == id).unwrap();
                                        let to = groups
                                            .iter()
                                            .position(|group| group.id == target)
                                            .unwrap();
                                        if from != to {
                                            let group = groups.remove(from);
                                            groups.insert(to, group);
                                            self.opencode_presence.tabs.reveal_active = true;
                                            self.persist_opencode_groups();
                                        }
                                    }
                                } else if drag.target == Some(id) {
                                    self.opencode_groups.click(id);
                                    self.opencode_presence.tabs.scroll = 0;
                                    self.opencode_presence.tabs.reveal_active = false;
                                    self.persist_opencode_groups();
                                }
                            }
                            DragSource::Card {
                                instance_id,
                                session_id,
                            } => {
                                if drag.moved {
                                    if self.opencode_presence.instance_id()
                                        == Some(instance_id.as_str())
                                        && self
                                            .opencode_presence
                                            .tabs
                                            .items
                                            .iter()
                                            .any(|tab| tab.session_id == session_id)
                                    {
                                        if let Some(group) = drag.target {
                                            let previous =
                                                self.opencode_groups.group_for(&session_id);
                                            if previous != group {
                                                self.opencode_groups
                                                    .catalog
                                                    .assignments
                                                    .insert(session_id.clone(), group);
                                                for order in self
                                                    .opencode_groups
                                                    .catalog
                                                    .card_order
                                                    .values_mut()
                                                {
                                                    order.retain(|id| id != &session_id);
                                                }
                                                if let Some(order) = self
                                                    .opencode_groups
                                                    .catalog
                                                    .card_order
                                                    .get_mut(&group)
                                                {
                                                    order.push(session_id);
                                                }
                                                self.opencode_presence.tabs.reveal_active = false;
                                                self.persist_opencode_groups();
                                            }
                                        } else if let Some(target) = drag.card_target
                                            && self.opencode_groups.reorder_card(
                                                &self.opencode_presence.tabs.items,
                                                &session_id,
                                                &target,
                                            )
                                        {
                                            self.opencode_presence.tabs.reveal_active = false;
                                            self.persist_opencode_groups();
                                        }
                                    }
                                } else if let Err(error) =
                                    self.opencode_presence.focus_tab(&instance_id, &session_id)
                                {
                                    self.notice =
                                        Some(format!("Could not focus OpenCode tab: {error}"));
                                }
                            }
                        }
                    } else {
                        if drag.moved
                            && !drag.is_group()
                            && drag.card_target.is_none()
                            && self.regions.scroll_target_at(point)
                                == Some(ScrollTarget::OpenCodeTabs)
                        {
                            self.scroll_target(
                                ScrollTarget::OpenCodeTabs,
                                drag.previous.x as isize - point.x as isize,
                                false,
                            );
                        }
                        if drag.moved
                            && drag.is_group()
                            && let Some(HitTarget::OpenCodeGroupScroll(delta)) = hit
                        {
                            self.scroll_target(ScrollTarget::OpenCodeGroups, delta, false);
                        }
                        drag.previous = point;
                        self.opencode_groups.drag = Some(drag);
                    }
                    return true;
                }
                MouseEventKind::Down(_) => {}
                _ => {
                    self.opencode_groups.drag = Some(drag);
                    return true;
                }
            }
        }

        match (mouse.kind, hit) {
            (
                MouseEventKind::Down(MouseButton::Left),
                Some(HitTarget::OpenCodeTab {
                    instance_id,
                    session_id,
                }),
            ) => {
                self.mobile_scroll_drag = None;
                self.selection.clear();
                self.opencode_groups.last_click = None;
                self.opencode_groups.drag = Some(StripDrag {
                    source: DragSource::Card {
                        instance_id,
                        session_id,
                    },
                    start: point,
                    previous: point,
                    moved: false,
                    target: None,
                    card_target: None,
                });
            }
            (MouseEventKind::Down(MouseButton::Left), Some(HitTarget::OpenCodeGroup(id))) => {
                self.mobile_scroll_drag = None;
                self.selection.clear();
                self.opencode_groups.drag = Some(StripDrag {
                    source: DragSource::Group(id),
                    start: point,
                    previous: point,
                    moved: false,
                    target: None,
                    card_target: None,
                });
            }
            (MouseEventKind::Down(MouseButton::Right), Some(HitTarget::OpenCodeGroup(id))) => {
                self.opencode_groups.begin_edit(Some(id))
            }
            (MouseEventKind::Down(MouseButton::Left), Some(HitTarget::OpenCodeGroupAdd)) => {
                self.opencode_groups.begin_edit(None)
            }
            (
                MouseEventKind::Down(MouseButton::Left),
                Some(HitTarget::OpenCodeGroupScroll(delta)),
            ) => self.scroll_target(ScrollTarget::OpenCodeGroups, delta, false),
            _ => return false,
        }
        true
    }
}
