# Agent Instructions

## Dependency Boundaries

- Hunkle's only Norm integration is opening a tab for the active workspace and following active Norm-tab workspace changes.
- Keep Norm presence and CLI interaction in `src/app/norm_presence.rs`; UI and general application code must not query or manage Norm sessions independently.
- Hunkle does not own agent sessions, transcripts, models, scheduling, delivery, or terminal layouts. Do not add those responsibilities back to Hunkle.

## Established Systems

- Read the applicable accepted ADR under `docs/adr/` before changing repository lifecycle, operations, refresh behavior, previews, linked worktrees, or input routing. Extend the established owner instead of creating a parallel mechanism.
- `RepositorySession` owns workspace opening and hydration, repository operation compatibility, background workers, refresh scopes and queueing, and stale-result rejection. Submit operation or refresh intents to it; do not add parallel busy flags, worker channels, refresh queues, or unconditional full reloads.
- `LinkedWorktreeCatalog` is the authority for linked-worktree topology. Git inventory defines existence and checkout state; known repositories are discovery memory, not alternate topology.
- `PreviewPresentation` owns preview styling, wrapping, large-file windows, rendered-row and hunk mappings, scroll limits, and cache identity. Do not recompute or cache those mappings in feature state or renderers.
- Use `RepoPath` for workspace-relative paths and the guarded operations in `filesystem` for workspace reads and mutations. Use `atomic_write` or `atomic_write_if_unchanged` for persisted state and edits; do not assume paths are UTF-8, join unchecked paths to the workspace root, or overwrite files directly.
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

- Before handing work back to the user after a task that changes Hunkle, run one final `cargo hunkle-install-local` from the current worktree root after all edits and verification are complete. Do not install at intermediate checkpoints. If a later edit is required, perform the final install again before handoff. This remains required when the user also requests a global installation.
- The project-defined command installs to `target/hunkle-install` and builds in `target`, both inside the current worktree. Do not replace it with a direct `cargo install`; user-level Cargo configuration may otherwise share stale artifacts or replace another worktree's binary.
- Normal local installs are offline and reuse the incremental dev cache. If a fresh checkout or dependency change is unavailable locally, run `cargo hunkle-install-local-online` once, then continue using `cargo hunkle-install-local`.
- Run `cargo hunkle-install-global` in addition to the required local install only when the user explicitly requests a global installation for the current task. Global installs use the optimized release profile and can require a long cold build.
- Do not run `cargo hunkle-clean-local-cache` as routine maintenance or build optimization. It deliberately removes compiled dependencies and makes the next local install a full cold rebuild; use it only when the user asks to reclaim storage or storage is genuinely constrained.
- Do not restart an open Hunkle process after installing. Hunkle automatically detects and loads the worktree's local build.
