# Legacy

These paths remain temporarily for removal after the Norm migration. Do not
add new behavior to them.

## Herdr agent placement

- `src/app/herdr_prompt.rs`: pending-agent state, pane layout polling, and pane selection/splitting.
- `src/ui/overlays/herdr.rs`: the retained Herdr agent placement overlay.
- `src/app/herdr_session/` agent-creation helpers: pane replacement, splitting, tab creation, and layout operations.
- Herdr agent-placement hit targets and routing in `src/app/mod.rs`, `src/app/mouse.rs`, and `src/ui/`.

Norm now owns new agent-tab creation. The retained placement code is not used by
`Ctrl+Space` or the current Norm creation path.

## Agent sidebar

- `a` / `ToggleAgents`, `App::agents_visible`, and `AgentListMode` cycling.
- The Herdr live-agent, Norm, scheduled-run, and stashed-agent cards in
  `src/ui/agents.rs` and `src/ui/changes/`.
- Agent-card activation, Herdr layout restoration, agent previews, fullscreen
  agent views, stash/resume flows, and their input routing.
- Agent-only settings and controls such as cross-workspace agents, harness
  display, card-click behavior, time display, preview split width, and timing
  cleanup in `src/app/settings.rs`, `src/app/keys/settings.rs`, and
  `src/ui/overlays/settings.rs`.

The sidebar is hidden by default. These surfaces remain only as compatibility
UI until the old agent-management model is removed.
