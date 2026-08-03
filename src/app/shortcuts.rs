use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ShortcutAction {
    ToggleFullscreen,
    ShowChanges,
    ShowFiles,
    ShowAgents,
    ToggleGraph,
    Quit,
    OpenHerdr,
    FindFile,
    Refresh,
    OpenExplorer,
    OpenSettings,
    OpenActions,
    OpenGitCommand,
    StartAgent,
    OpenHelp,
    ToggleWrap,
    ToggleMarkdown,
    RenameFile,
    DeleteFile,
    EditFile,
    ConfigureEditor,
    FocusCommit,
    ToggleAgents,
    UnstageAll,
    StageSelection,
    DiscardChanges,
    SaveOrFormat,
    SubmitCommit,
    ExplorerFavorite,
    AuthorEnableAll,
    AuthorDisableAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyChord {
    pub(crate) code: KeyCode,
    pub(crate) modifiers: KeyModifiers,
}

impl KeyChord {
    pub(crate) fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        normalize(code, modifiers)
    }

    pub(crate) fn from_event(event: KeyEvent) -> Self {
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('j' | 'm'))
        {
            return Self::new(KeyCode::Enter, KeyModifiers::CONTROL);
        }
        Self::new(event.code, event.modifiers)
    }

    pub(crate) fn label(self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            parts.push("Super".to_owned());
        }
        let shifted_letter = matches!(self.code, KeyCode::Char(c) if c.is_ascii_lowercase())
            && self.modifiers == KeyModifiers::SHIFT;
        if self.modifiers.contains(KeyModifiers::SHIFT) && !shifted_letter {
            parts.push("Shift".to_owned());
        }
        let key = match self.code {
            KeyCode::Char(c) if shifted_letter => c.to_ascii_uppercase().to_string(),
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char('+') => "Plus".to_owned(),
            KeyCode::Char('-') => "Minus".to_owned(),
            KeyCode::Char('=') => "Equals".to_owned(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::F(number) => format!("F{number}"),
            KeyCode::BackTab => "BackTab".to_owned(),
            other => format!("{other:?}"),
        };
        parts.push(key);
        parts.join("+")
    }

    fn parse(value: &str) -> Option<Self> {
        let mut modifiers = KeyModifiers::NONE;
        let mut key = None;
        for part in value.trim().split('+') {
            let part = part.trim();
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
                "alt" => modifiers.insert(KeyModifiers::ALT),
                "shift" => modifiers.insert(KeyModifiers::SHIFT),
                "super" | "meta" => modifiers.insert(KeyModifiers::SUPER),
                _ if key.is_none() => key = parse_key_code(part),
                _ => return None,
            }
        }
        Some(Self::new(key?, modifiers))
    }
}

fn parse_key_code(name: &str) -> Option<KeyCode> {
    if name.chars().count() == 1 {
        return Some(KeyCode::Char(name.chars().next()?));
    }
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "delete" | "del" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "backspace" => KeyCode::Backspace,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "plus" => KeyCode::Char('+'),
        "minus" => KeyCode::Char('-'),
        "equals" => KeyCode::Char('='),
        name if name.starts_with('f') => KeyCode::F(name[1..].parse().ok()?),
        _ => return None,
    })
}

fn normalize(code: KeyCode, modifiers: KeyModifiers) -> KeyChord {
    let mut modifiers = modifiers
        & (KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    let code = match code {
        KeyCode::Char(character) if character.is_ascii_uppercase() => {
            modifiers.insert(KeyModifiers::SHIFT);
            KeyCode::Char(character.to_ascii_lowercase())
        }
        KeyCode::Char(character) if !character.is_ascii_alphabetic() => {
            modifiers.remove(KeyModifiers::SHIFT);
            KeyCode::Char(character)
        }
        other => other,
    };
    KeyChord { code, modifiers }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ShortcutDefinition {
    pub(crate) action: ShortcutAction,
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) section: &'static str,
    scope: u16,
    default: KeyChord,
}

const MAIN: u16 = 1 << 0;
const COMMIT: u16 = 1 << 1;
const FILE_EDIT: u16 = 1 << 2;
const EXPLORER: u16 = 1 << 3;
const AUTHOR_FILTER: u16 = 1 << 4;

const fn chord(code: KeyCode, modifiers: KeyModifiers) -> KeyChord {
    KeyChord { code, modifiers }
}

macro_rules! shortcut {
    ($action:ident, $id:literal, $label:literal, $section:literal, $scope:expr, $code:expr) => {
        ShortcutDefinition {
            action: ShortcutAction::$action,
            id: $id,
            label: $label,
            section: $section,
            scope: $scope,
            default: chord($code, KeyModifiers::NONE),
        }
    };
    ($action:ident, $id:literal, $label:literal, $section:literal, $scope:expr, $code:expr, $mods:expr) => {
        ShortcutDefinition {
            action: ShortcutAction::$action,
            id: $id,
            label: $label,
            section: $section,
            scope: $scope,
            default: chord($code, $mods),
        }
    };
}

pub(crate) static SHORTCUTS: &[ShortcutDefinition] = &[
    shortcut!(
        ToggleFullscreen,
        "toggle-fullscreen",
        "Toggle fullscreen",
        "Navigation",
        MAIN | COMMIT,
        KeyCode::Tab
    ),
    shortcut!(
        ShowChanges,
        "show-changes",
        "Show Changes",
        "Navigation",
        MAIN | COMMIT,
        KeyCode::F(1)
    ),
    shortcut!(
        ShowFiles,
        "show-files",
        "Show Files",
        "Navigation",
        MAIN | COMMIT,
        KeyCode::F(2)
    ),
    shortcut!(
        ShowAgents,
        "show-agents",
        "Show Agents",
        "Navigation",
        MAIN | COMMIT,
        KeyCode::F(3)
    ),
    shortcut!(
        ToggleGraph,
        "toggle-graph",
        "Show / hide Git graph",
        "Navigation",
        MAIN | COMMIT,
        KeyCode::Char('g')
    ),
    shortcut!(
        Refresh,
        "refresh",
        "Refresh repository",
        "Navigation",
        MAIN,
        KeyCode::Char('r')
    ),
    shortcut!(
        OpenExplorer,
        "open-explorer",
        "Explorer",
        "Navigation",
        MAIN,
        KeyCode::Char('o')
    ),
    shortcut!(
        FindFile,
        "find-file",
        "Search repository",
        "Navigation",
        MAIN,
        KeyCode::Char('/')
    ),
    shortcut!(
        OpenSettings,
        "open-settings",
        "Settings",
        "Navigation",
        MAIN,
        KeyCode::Char('s')
    ),
    shortcut!(
        OpenActions,
        "open-actions",
        "Git actions",
        "Navigation",
        MAIN,
        KeyCode::Char('x')
    ),
    shortcut!(
        OpenGitCommand,
        "open-git-command",
        "Git command",
        "Navigation",
        MAIN,
        KeyCode::Char('g'),
        KeyModifiers::SHIFT
    ),
    shortcut!(
        OpenHerdr,
        "open-herdr",
        "Send to Herdr pane",
        "Navigation",
        MAIN,
        KeyCode::F(1),
        KeyModifiers::SHIFT
    ),
    shortcut!(
        StartAgent,
        "start-agent",
        "Start agent",
        "Navigation",
        MAIN,
        KeyCode::Char(' '),
        KeyModifiers::CONTROL
    ),
    shortcut!(
        OpenHelp,
        "open-help",
        "Keyboard help",
        "Navigation",
        MAIN,
        KeyCode::Char('?')
    ),
    shortcut!(Quit, "quit", "Quit", "Navigation", MAIN, KeyCode::Char('q')),
    shortcut!(
        ToggleWrap,
        "toggle-wrap",
        "Preview wrapping",
        "Changes / files",
        MAIN,
        KeyCode::Char('z')
    ),
    shortcut!(
        ToggleMarkdown,
        "toggle-markdown",
        "Markdown preview",
        "Changes / files",
        MAIN,
        KeyCode::Char('m')
    ),
    shortcut!(
        RenameFile,
        "rename-file",
        "Rename file / folder",
        "Changes / files",
        MAIN,
        KeyCode::F(2),
        KeyModifiers::SHIFT
    ),
    shortcut!(
        DeleteFile,
        "delete-file",
        "Delete from Files",
        "Changes / files",
        MAIN,
        KeyCode::Delete,
        KeyModifiers::CONTROL
    ),
    shortcut!(
        EditFile,
        "edit-file",
        "Edit selected file",
        "Changes / files",
        MAIN,
        KeyCode::Char('e')
    ),
    shortcut!(
        ConfigureEditor,
        "configure-editor",
        "Configure editor",
        "Changes / files",
        MAIN,
        KeyCode::Char('e'),
        KeyModifiers::SHIFT
    ),
    shortcut!(
        FocusCommit,
        "focus-commit",
        "Commit editor",
        "Changes / files",
        MAIN,
        KeyCode::Char('c')
    ),
    shortcut!(
        ToggleAgents,
        "toggle-agents",
        "Cycle agents / stash / off",
        "Changes / files",
        MAIN,
        KeyCode::Char('a')
    ),
    shortcut!(
        UnstageAll,
        "unstage-all",
        "Unstage all",
        "Changes / files",
        MAIN,
        KeyCode::Char('u')
    ),
    shortcut!(
        StageSelection,
        "stage-selection",
        "Stage selection / hunk",
        "Changes / files",
        MAIN,
        KeyCode::Char(' ')
    ),
    shortcut!(
        DiscardChanges,
        "discard-changes",
        "Discard unstaged changes",
        "Changes / files",
        MAIN,
        KeyCode::Delete
    ),
    shortcut!(
        SaveOrFormat,
        "save-or-format",
        "Save editor / format file",
        "Changes / files",
        MAIN | FILE_EDIT,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL
    ),
    shortcut!(
        SubmitCommit,
        "submit-commit",
        "Create commit",
        "Commit",
        COMMIT,
        KeyCode::Enter,
        KeyModifiers::CONTROL
    ),
    shortcut!(
        ExplorerFavorite,
        "explorer-favorite",
        "Add / remove favorite",
        "Explorer",
        EXPLORER,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL
    ),
    shortcut!(
        AuthorEnableAll,
        "author-enable-all",
        "Enable all authors",
        "Author filter",
        AUTHOR_FILTER,
        KeyCode::Char('a')
    ),
    shortcut!(
        AuthorDisableAll,
        "author-disable-all",
        "Disable all authors",
        "Author filter",
        AUTHOR_FILTER,
        KeyCode::Char('n')
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shortcuts {
    overrides: BTreeMap<ShortcutAction, KeyChord>,
}

impl Shortcuts {
    pub(crate) fn definitions() -> &'static [ShortcutDefinition] {
        SHORTCUTS
    }

    pub(crate) fn binding(&self, action: ShortcutAction) -> KeyChord {
        self.overrides
            .get(&action)
            .copied()
            .unwrap_or_else(|| definition(action).default)
    }

    pub(crate) fn label(&self, action: ShortcutAction) -> String {
        self.binding(action).label()
    }

    pub(crate) fn matches(&self, action: ShortcutAction, event: KeyEvent) -> bool {
        self.binding(action) == KeyChord::from_event(event)
    }

    pub(crate) fn main_action(&self, event: KeyEvent) -> Option<ShortcutAction> {
        let chord = KeyChord::from_event(event);
        SHORTCUTS.iter().find_map(|definition| {
            (definition.scope & MAIN != 0 && self.binding(definition.action) == chord)
                .then_some(definition.action)
        })
    }

    pub(crate) fn remap_explorer(&self, event: KeyEvent) -> KeyEvent {
        self.remap(event, EXPLORER)
    }

    pub(crate) fn remap_author_filter(&self, event: KeyEvent) -> KeyEvent {
        self.remap(event, AUTHOR_FILTER)
    }

    fn remap(&self, event: KeyEvent, scope: u16) -> KeyEvent {
        let incoming = KeyChord::from_event(event);
        if let Some(definition) = SHORTCUTS.iter().find(|definition| {
            definition.scope & scope != 0 && self.binding(definition.action) == incoming
        }) {
            let mut code = definition.default.code;
            if definition.default.modifiers.contains(KeyModifiers::SHIFT)
                && let KeyCode::Char(character) = code
                && character.is_ascii_lowercase()
            {
                code = KeyCode::Char(character.to_ascii_uppercase());
            }
            return KeyEvent::new(code, definition.default.modifiers);
        }
        if SHORTCUTS.iter().any(|definition| {
            definition.scope & scope != 0
                && definition.default == incoming
                && self.binding(definition.action) != incoming
        }) {
            return KeyEvent::new(KeyCode::Null, KeyModifiers::NONE);
        }
        event
    }

    pub(crate) fn set(&mut self, action: ShortcutAction, chord: KeyChord) -> Result<(), String> {
        if chord.code == KeyCode::Esc
            || (chord.code == KeyCode::Char('c') && chord.modifiers.contains(KeyModifiers::CONTROL))
        {
            return Err("Esc and Ctrl+C are reserved recovery keys".to_owned());
        }
        let current = definition(action);
        if KeyChord::parse(&chord.label()) != Some(chord) {
            return Err(format!("{} cannot be saved as a shortcut", chord.label()));
        }
        if conflicts_with_fixed_input(current.scope, chord) {
            return Err(format!(
                "{} is reserved for navigation or text editing in this context",
                chord.label()
            ));
        }
        if let Some(conflict) = SHORTCUTS.iter().find(|candidate| {
            candidate.action != action
                && candidate.scope & current.scope != 0
                && self.binding(candidate.action) == chord
        }) {
            return Err(format!(
                "{} is already used by {}",
                chord.label(),
                conflict.label
            ));
        }
        if chord == current.default {
            self.overrides.remove(&action);
        } else {
            self.overrides.insert(action, chord);
        }
        Ok(())
    }

    pub(crate) fn reset(&mut self, action: ShortcutAction) -> bool {
        self.overrides.remove(&action).is_some()
    }

    pub(crate) fn is_overridden(&self, action: ShortcutAction) -> bool {
        self.overrides.contains_key(&action)
    }

    pub(crate) fn serialized(&self) -> impl Iterator<Item = (&'static str, String)> + '_ {
        SHORTCUTS.iter().filter_map(|definition| {
            self.overrides
                .get(&definition.action)
                .map(|chord| (definition.id, chord.label()))
        })
    }

    pub(crate) fn load_override(&mut self, id: &str, value: &str) {
        let Some(definition) = SHORTCUTS.iter().find(|definition| {
            definition.id == id
                || (id == "toggle-pane" && definition.action == ShortcutAction::ToggleFullscreen)
        }) else {
            return;
        };
        let Some(chord) = KeyChord::parse(value) else {
            return;
        };
        let _ = self.set(definition.action, chord);
    }
}

fn conflicts_with_fixed_input(scope: u16, chord: KeyChord) -> bool {
    let main_navigation = matches!(
        chord.code,
        KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Enter
            | KeyCode::Char('h' | 'j' | 'k' | 'l')
    );
    let editor_input = chord.modifiers.is_empty()
        && matches!(
            chord.code,
            KeyCode::Char(_)
                | KeyCode::Enter
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
        );
    scope & MAIN != 0 && main_navigation || scope & FILE_EDIT != 0 && editor_input
}

fn definition(action: ShortcutAction) -> &'static ShortcutDefinition {
    SHORTCUTS
        .iter()
        .find(|definition| definition.action == action)
        .expect("every shortcut action has a definition")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_parses_and_normalizes_key_chords() {
        for value in [
            "Tab",
            "g",
            "G",
            "Ctrl+s",
            "Ctrl+Delete",
            "F3",
            "Insert",
            "Space",
        ] {
            let chord = KeyChord::parse(value).unwrap();
            assert_eq!(KeyChord::parse(&chord.label()), Some(chord), "{value}");
        }
        assert_eq!(
            KeyChord::from_event(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            KeyChord::parse("G").unwrap()
        );
    }

    #[test]
    fn control_space_starts_an_agent_by_default() {
        let shortcuts = Shortcuts::default();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL);

        assert_eq!(shortcuts.main_action(key), Some(ShortcutAction::StartAgent));
        assert_eq!(shortcuts.label(ShortcutAction::StartAgent), "Ctrl+Space");
    }

    #[test]
    fn tab_toggles_fullscreen_and_loads_the_previous_override_id() {
        let mut shortcuts = Shortcuts::default();

        assert_eq!(
            shortcuts.main_action(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(ShortcutAction::ToggleFullscreen)
        );
        shortcuts.load_override("toggle-pane", "Alt+f");
        assert_eq!(shortcuts.label(ShortcutAction::ToggleFullscreen), "Alt+f");
    }

    #[test]
    fn function_keys_select_sidebar_panes_by_default() {
        let shortcuts = Shortcuts::default();

        assert_eq!(
            shortcuts.main_action(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
            Some(ShortcutAction::ShowChanges)
        );
        assert_eq!(
            shortcuts.main_action(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            Some(ShortcutAction::ShowFiles)
        );
        assert_eq!(
            shortcuts.main_action(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)),
            Some(ShortcutAction::ShowAgents)
        );
    }

    #[test]
    fn rejects_overlapping_bindings_but_allows_separate_contexts() {
        let mut shortcuts = Shortcuts::default();
        let graph = shortcuts.binding(ShortcutAction::ToggleGraph);
        assert!(shortcuts.set(ShortcutAction::OpenExplorer, graph).is_err());
        assert!(
            shortcuts
                .set(
                    ShortcutAction::ExplorerFavorite,
                    KeyChord::new(KeyCode::Char('g'), KeyModifiers::NONE),
                )
                .is_ok()
        );
    }

    #[test]
    fn registry_ids_and_defaults_are_unambiguous() {
        for (index, left) in SHORTCUTS.iter().enumerate() {
            for right in &SHORTCUTS[index + 1..] {
                assert_ne!(left.id, right.id, "duplicate shortcut id {}", left.id);
                if left.scope & right.scope != 0 {
                    assert_ne!(
                        left.default, right.default,
                        "{} and {} share a default in the same context",
                        left.id, right.id
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_fixed_navigation_and_editor_input() {
        let mut shortcuts = Shortcuts::default();
        assert!(
            shortcuts
                .set(
                    ShortcutAction::Quit,
                    KeyChord::new(KeyCode::Char('j'), KeyModifiers::NONE),
                )
                .is_err()
        );
        assert!(
            shortcuts
                .set(
                    ShortcutAction::SaveOrFormat,
                    KeyChord::new(KeyCode::Char('x'), KeyModifiers::NONE),
                )
                .is_err()
        );
    }

    #[test]
    fn resetting_restores_the_default() {
        let mut shortcuts = Shortcuts::default();
        shortcuts
            .set(
                ShortcutAction::OpenExplorer,
                KeyChord::new(KeyCode::Char('v'), KeyModifiers::ALT),
            )
            .unwrap();
        assert!(shortcuts.is_overridden(ShortcutAction::OpenExplorer));
        assert!(shortcuts.reset(ShortcutAction::OpenExplorer));
        assert_eq!(shortcuts.label(ShortcutAction::OpenExplorer), "o");
    }
}
