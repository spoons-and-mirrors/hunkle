# Agent Instructions

## Dependency Boundaries

- Hunkle must work with official released Herdr binaries and their documented APIs.
- Treat the Herdr repository, Herdr worktrees, installed Herdr binary, and running Herdr server as read-only unless the user explicitly authorizes changing Herdr for the current task.
- Do not build, install, replace, patch, or live-handoff a custom Herdr binary to make Hunkle behavior work.
- Fix compatibility and behavior inside Hunkle. If the official Herdr API cannot support a requirement, explain the limitation and ask before proposing a cross-repository change.

## Installation

- At the end of every task that changes Hunkle, install the current checkout with `cargo install --path . --force --locked`.
- After installing from inside Herdr, restart any open Hunkle process in place so it loads the new binary. Preserve its pane and tab layout, and do not restart unrelated panes or processes.
- Restart Hunkle in one shell invocation after identifying its pane: `herdr pane send-text PANE_ID $'\x03' && sleep 0.1 && herdr pane run PANE_ID hunkle` (replace `PANE_ID` with the actual pane ID). Do not split a successful restart into separate stop, verification, start, and verification tool calls. `q && hunkle` is not valid because Hunkle, rather than the shell, receives `q`.
