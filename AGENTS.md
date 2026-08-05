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

- At the end of every task that changes Hunkle, run `cargo hunkle-install-local` from the current worktree root.
- The project-defined command installs to `target/hunkle-install` and builds in `target`, both inside the current worktree. Do not replace it with a direct `cargo install`; user-level Cargo configuration may otherwise share stale artifacts or replace another worktree's binary.
- Never run `cargo hunkle-install-global` or otherwise replace the globally installed Hunkle binary as part of a development task.
- Do not restart an open Hunkle process after installing. The user chooses when to load a worktree's build with the `↻` control in Hunkle's top-right corner.
