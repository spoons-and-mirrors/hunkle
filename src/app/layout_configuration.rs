#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneDirection {
    Up,
    Right,
    Down,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneRect {
    pub(crate) id: String,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneLayout {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) panes: Vec<PaneRect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutFocus {
    Pane(usize),
    Split(usize, PaneDirection),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LayoutConfiguration {
    layout: Option<PaneLayout>,
    protected_pane_id: Option<String>,
    focus: Option<LayoutFocus>,
}

impl LayoutConfiguration {
    pub(crate) fn set_layout(&mut self, layout: PaneLayout, protected_pane_id: Option<String>) {
        self.focus = initial_focus(&layout, protected_pane_id.as_deref());
        self.layout = Some(layout);
        self.protected_pane_id = protected_pane_id;
    }

    pub(crate) fn layout(&self) -> Option<&PaneLayout> {
        self.layout.as_ref()
    }

    pub(crate) fn protected_pane_id(&self) -> Option<&str> {
        self.protected_pane_id.as_deref()
    }

    pub(crate) fn focus(&self) -> Option<LayoutFocus> {
        self.focus
    }

    pub(crate) fn cycle_focus(&mut self, backwards: bool) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let focus_count = layout.panes.len() * 5;
        if focus_count == 0 {
            self.focus = None;
            return;
        }
        let current = self
            .focus
            .map(focus_ordinal)
            .unwrap_or(0)
            .min(focus_count - 1);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(focus_count - 1)
        } else {
            (current + 1) % focus_count
        };
        self.focus = Some(focus_from_ordinal(next));
    }

    pub(crate) fn move_focus(&mut self, direction: PaneDirection) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let Some(focus) = self.focus else {
            self.focus = initial_focus(layout, self.protected_pane_id());
            return;
        };
        let (index, edge) = match focus {
            LayoutFocus::Pane(index) => (index, None),
            LayoutFocus::Split(index, edge) => (index, Some(edge)),
        };
        if layout.panes.get(index).is_none() {
            self.focus = initial_focus(layout, self.protected_pane_id());
            return;
        }
        self.focus = match edge {
            None => Some(LayoutFocus::Split(index, direction)),
            Some(edge) if edge == direction => neighboring_pane(layout, index, direction)
                .map(LayoutFocus::Pane)
                .or(Some(focus)),
            Some(edge) if opposite_direction(edge) == direction => Some(LayoutFocus::Pane(index)),
            Some(_) => Some(LayoutFocus::Split(index, direction)),
        };
    }
}

fn initial_focus(layout: &PaneLayout, protected_pane_id: Option<&str>) -> Option<LayoutFocus> {
    layout
        .panes
        .iter()
        .position(|pane| Some(pane.id.as_str()) != protected_pane_id)
        .or_else(|| (!layout.panes.is_empty()).then_some(0))
        .map(LayoutFocus::Pane)
}

fn focus_ordinal(focus: LayoutFocus) -> usize {
    match focus {
        LayoutFocus::Pane(index) => index * 5,
        LayoutFocus::Split(index, PaneDirection::Up) => index * 5 + 1,
        LayoutFocus::Split(index, PaneDirection::Right) => index * 5 + 2,
        LayoutFocus::Split(index, PaneDirection::Down) => index * 5 + 3,
        LayoutFocus::Split(index, PaneDirection::Left) => index * 5 + 4,
    }
}

fn focus_from_ordinal(ordinal: usize) -> LayoutFocus {
    let index = ordinal / 5;
    match ordinal % 5 {
        0 => LayoutFocus::Pane(index),
        1 => LayoutFocus::Split(index, PaneDirection::Up),
        2 => LayoutFocus::Split(index, PaneDirection::Right),
        3 => LayoutFocus::Split(index, PaneDirection::Down),
        4 => LayoutFocus::Split(index, PaneDirection::Left),
        _ => unreachable!(),
    }
}

fn opposite_direction(direction: PaneDirection) -> PaneDirection {
    match direction {
        PaneDirection::Up => PaneDirection::Down,
        PaneDirection::Down => PaneDirection::Up,
        PaneDirection::Left => PaneDirection::Right,
        PaneDirection::Right => PaneDirection::Left,
    }
}

fn neighboring_pane(
    layout: &PaneLayout,
    current_index: usize,
    direction: PaneDirection,
) -> Option<usize> {
    let current = layout.panes.get(current_index)?;
    let current_x = i32::from(current.x) * 2 + i32::from(current.width);
    let current_y = i32::from(current.y) * 2 + i32::from(current.height);
    layout
        .panes
        .iter()
        .enumerate()
        .filter_map(|(index, pane)| {
            let x = i32::from(pane.x) * 2 + i32::from(pane.width);
            let y = i32::from(pane.y) * 2 + i32::from(pane.height);
            let (primary, secondary) = match direction {
                PaneDirection::Up if y < current_y => (current_y - y, (x - current_x).abs()),
                PaneDirection::Down if y > current_y => (y - current_y, (x - current_x).abs()),
                PaneDirection::Left if x < current_x => (current_x - x, (y - current_y).abs()),
                PaneDirection::Right if x > current_x => (x - current_x, (y - current_y).abs()),
                _ => return None,
            };
            Some(((secondary, primary, index), index))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, index)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> PaneLayout {
        PaneLayout {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
            panes: vec![
                PaneRect {
                    id: "left".to_owned(),
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
                PaneRect {
                    id: "right".to_owned(),
                    x: 100,
                    y: 0,
                    width: 100,
                    height: 100,
                },
            ],
        }
    }

    #[test]
    fn directional_focus_moves_between_centers_and_edges() {
        let mut configuration = LayoutConfiguration::default();
        configuration.set_layout(layout(), None);

        assert_eq!(configuration.focus(), Some(LayoutFocus::Pane(0)));
        configuration.move_focus(PaneDirection::Right);
        assert_eq!(
            configuration.focus(),
            Some(LayoutFocus::Split(0, PaneDirection::Right))
        );
        configuration.move_focus(PaneDirection::Right);
        assert_eq!(configuration.focus(), Some(LayoutFocus::Pane(1)));
        configuration.move_focus(PaneDirection::Left);
        configuration.move_focus(PaneDirection::Right);
        assert_eq!(configuration.focus(), Some(LayoutFocus::Pane(1)));
    }

    #[test]
    fn initial_focus_skips_the_protected_pane_and_cycles() {
        let mut configuration = LayoutConfiguration::default();
        configuration.set_layout(layout(), Some("left".to_owned()));

        assert_eq!(configuration.focus(), Some(LayoutFocus::Pane(1)));
        configuration.cycle_focus(false);
        assert_eq!(
            configuration.focus(),
            Some(LayoutFocus::Split(1, PaneDirection::Up))
        );
        configuration.cycle_focus(true);
        assert_eq!(configuration.focus(), Some(LayoutFocus::Pane(1)));
    }
}
