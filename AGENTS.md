# Agent Instructions

## Dependency Boundaries

- Hunkle must work with official released Herdr binaries and their documented APIs.
- Treat the Herdr repository, Herdr worktrees, installed Herdr binary, and running Herdr server as read-only unless the user explicitly authorizes changing Herdr for the current task.
- Do not build, install, replace, patch, or live-handoff a custom Herdr binary to make Hunkle behavior work.
- Fix compatibility and behavior inside Hunkle. If the official Herdr API cannot support a requirement, explain the limitation and ask before proposing a cross-repository change.

## Established Systems

- Read the applicable accepted ADR under `docs/adr/` before changing repository lifecycle, operations, refresh behavior, previews, linked worktrees, or input routing. Extend the established owner instead of creating a parallel mechanism.
- `RepositorySession` owns workspace opening and hydration, repository operation compatibility, background workers, refresh scopes and queueing, and stale-result rejection. Submit operation or refresh intents to it; do not add parallel busy flags, worker channels, refresh queues, or unconditional full reloads.
- `LinkedWorktreeCatalog` is the authority for linked-worktree topology and destination metadata. Git inventory defines existence and checkout state; Herdr observations and known repositories are discovery inputs, not alternate topology.
- `PreviewPresentation` owns preview styling, wrapping, large-file windows, rendered-row and hunk mappings, scroll limits, and cache identity. Do not recompute or cache those mappings in feature state or renderers.
- Use `RepoPath` for workspace-relative paths and the guarded operations in `filesystem` for workspace reads and mutations. Use `atomic_write` or `atomic_write_if_unchanged` for persisted state and edits; do not assume paths are UTF-8, join unchecked paths to the workspace root, or overwrite files directly.
- Keep Herdr CLI and API interaction in `src/app/herdr_session/client.rs` and expose behavior through `HerdrSession`; UI and general application code must not shell out to Herdr independently.
- Use `WorkspaceState` for restoring and persisting the active workspace per Herdr pane. Do not create another active-workspace persistence path.
- Use `TextInput` for editable single-line controls so cursor movement, selection, paste, Unicode boundaries, and blink behavior remain consistent.

## Responsive Workspace

- Read `docs/adr/0009-responsive-workspace-composition.md` before changing responsive layout or navigation.
- Treat the responsive workspace architecture as the established foundation, not as a reason for another broad refactor. Implement concrete mobile behaviors within its ownership boundaries and add focused interaction coverage for them.
- `src/ui/workspace.rs` exclusively owns structural composition. Add single, column, or future row arrangements there instead of branching inside feature renderers.
- Compute `LayoutProfile` once per frame and pass presentation choices to renderers explicitly. Do not add device detection, `is_mobile`, or feature-owned viewport queries.
- Keep navigation and Back behavior in `WorkspaceNavigation`. Worktree and Files remain hierarchical subnavigation owned by `ChangesState`.
- Share application, repository, selection, loading, and preview state across compositions. Do not create parallel mobile feature state or UI trees.
- Renderers own geometry and register semantic hit and scroll targets. Input routing must consume those targets instead of reconstructing row or pane meaning from coordinates.
- Keep component-local visual adaptations local. Changes to which surfaces exist or how they are arranged belong to the workspace shell.

## Installation

- At the end of every task that changes Hunkle, run `cargo hunkle-install-local` from the current worktree root unless the user explicitly requests a global installation. When explicitly requested, run `cargo hunkle-install-global` instead.
- The project-defined command installs to `target/hunkle-install` and builds in `target`, both inside the current worktree. Do not replace it with a direct `cargo install`; user-level Cargo configuration may otherwise share stale artifacts or replace another worktree's binary.
- Do not run `cargo hunkle-install-global` or otherwise replace the globally installed Hunkle binary unless the user explicitly requests it for the current task.
- Do not restart an open Hunkle process after installing. Hunkle automatically detects and loads the worktree's local build.
