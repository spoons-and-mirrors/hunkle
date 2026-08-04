# Agent Instructions

## Dependency Boundaries

- Hunkle must work with official released Herdr binaries and their documented APIs.
- Treat the Herdr repository, Herdr worktrees, installed Herdr binary, and running Herdr server as read-only unless the user explicitly authorizes changing Herdr for the current task.
- Do not build, install, replace, patch, or live-handoff a custom Herdr binary to make Hunkle behavior work.
- Fix compatibility and behavior inside Hunkle. If the official Herdr API cannot support a requirement, explain the limitation and ask before proposing a cross-repository change.

## Installation

- At the end of every task that changes Hunkle, run `cargo hunkle-install-local` from the current worktree root.
- The project-defined command installs to `target/hunkle-install` and builds in `target`, both inside the current worktree. Do not replace it with a direct `cargo install`; user-level Cargo configuration may otherwise share stale artifacts or replace another worktree's binary.
- Never run `cargo hunkle-install-global` or otherwise replace the globally installed Hunkle binary as part of a development task.
- Do not restart an open Hunkle process after installing. The user chooses when to load a worktree's build with the `↻` control in Hunkle's top-right corner.
