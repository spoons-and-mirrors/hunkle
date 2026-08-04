# Agent Instructions

## Dependency Boundaries

- Hunkle must work with official released Herdr binaries and their documented APIs.
- Treat the Herdr repository, Herdr worktrees, installed Herdr binary, and running Herdr server as read-only unless the user explicitly authorizes changing Herdr for the current task.
- Do not build, install, replace, patch, or live-handoff a custom Herdr binary to make Hunkle behavior work.
- Fix compatibility and behavior inside Hunkle. If the official Herdr API cannot support a requirement, explain the limitation and ask before proposing a cross-repository change.

## Installation

- At the end of every task that changes Hunkle, install the current checkout into its worktree with `cargo install --path . --root target/hunkle-install --force --locked`.
- Never replace the globally installed Hunkle binary as part of a development task.
- Do not restart an open Hunkle process after installing. The user chooses when to load a worktree's build with the `↻` control in Hunkle's top-right corner.
