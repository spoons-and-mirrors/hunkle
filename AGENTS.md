# Agent Instructions

## Dependency Boundaries

- Hunkle must work with official released Herdr binaries and their documented APIs.
- Treat the Herdr repository, Herdr worktrees, installed Herdr binary, and running Herdr server as read-only unless the user explicitly authorizes changing Herdr for the current task.
- Do not build, install, replace, patch, or live-handoff a custom Herdr binary to make Hunkle behavior work.
- Fix compatibility and behavior inside Hunkle. If the official Herdr API cannot support a requirement, explain the limitation and ask before proposing a cross-repository change.

## Responsive Workspace

- Read `docs/adr/0009-responsive-workspace-composition.md` before changing responsive layout or navigation.
- `src/ui/workspace.rs` exclusively owns structural composition. Add single, column, or future row arrangements there instead of branching inside feature renderers.
- Compute `LayoutProfile` once per frame and pass presentation choices to renderers explicitly. Do not add device detection, `is_mobile`, or feature-owned viewport queries.
- Keep navigation and Back behavior in `WorkspaceNavigation`. Worktree and Files remain hierarchical subnavigation owned by `ChangesState`.
- Share application, repository, selection, loading, and preview state across compositions. Do not create parallel mobile feature state or UI trees.
- Renderers own geometry and register semantic hit and scroll targets. Input routing must consume those targets instead of reconstructing row or pane meaning from coordinates.
- Keep component-local visual adaptations local. Changes to which surfaces exist or how they are arranged belong to the workspace shell.

## Installation

- Do not install Hunkle or restart running Hunkle processes after changes unless the user explicitly requests it for the current task.
- When installation is explicitly requested, install the current checkout with `cargo install --path . --force --locked`.
- After an explicitly requested installation from inside Herdr, restart any open Hunkle process in place so it loads the new binary. Preserve its pane and tab layout, and do not restart unrelated panes or processes.
- Restart Hunkle in one shell invocation after identifying its pane: `herdr pane send-text PANE_ID $'\x03' && sleep 0.1 && herdr pane run PANE_ID hunkle` (replace `PANE_ID` with the actual pane ID). Do not split a successful restart into separate stop, verification, start, and verification tool calls. `q && hunkle` is not valid because Hunkle, rather than the shell, receives `q`.
