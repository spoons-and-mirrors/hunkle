use super::super::*;

impl App {
    pub(crate) fn open_workspace_panel(&mut self) {
        if !self.workspace_panel_enabled() {
            return;
        }
        self.workspace_panel.refresh();
        self.mode = Mode::WorkspacePanel;
    }

    pub(crate) fn handle_workspace_panel(&mut self, key: KeyEvent) {
        let captures_input = self.workspace_panel.group_editing
            || self.workspace_panel.snapshot_editing
            || self.workspace_panel.create_menu_open
            || self.workspace_panel.snapshot_menu_open
            || self.workspace_panel.rename_dialog.is_some()
            || self.workspace_panel.delete_dialog.is_some()
            || self.workspace_panel.snapshot_load_dialog.is_some();
        if self
            .settings
            .shortcuts
            .matches(ShortcutAction::OpenPresets, key)
            && !captures_input
        {
            self.open_workspace_presets();
            return;
        }
        let panel_key = if captures_input {
            key
        } else {
            self.settings.shortcuts.remap_workspace(key)
        };
        let effect = self.workspace_panel.handle_key(panel_key);
        if effect == WorkspacePanelEffect::Unhandled {
            if self.settings.shortcuts.matches_main(key) {
                self.mode = Mode::Normal;
                self.handle_key(key);
            }
        } else {
            self.apply_workspace_panel_effect(effect);
        }
    }

    pub(crate) fn open_workspace_presets(&mut self) {
        if !self.workspace_panel_enabled() {
            return;
        }
        self.workspace_panel.open_workspace_presets();
        self.mode = Mode::WorkspacePresets;
    }

    pub(crate) fn handle_workspace_presets(&mut self, key: KeyEvent) {
        let key = if self.workspace_panel.snapshot_editing
            || self.workspace_panel.snapshot_load_dialog.is_some()
        {
            key
        } else {
            self.settings.shortcuts.remap_presets(key)
        };
        let effect = self.workspace_panel.handle_workspace_presets(key);
        self.apply_workspace_panel_effect(effect);
    }
}
