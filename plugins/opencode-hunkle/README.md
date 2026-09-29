# Hunkle sync for OpenCode V2

This directory in the Hunkle repository is the source of truth;
`~/code/ocplugins/hunkle-sync` is a symlink here for OpenCode's plugin loading.

CLI-only plugin that publishes open tabs and the active session's workspace path to
`$XDG_RUNTIME_DIR/hunkle/opencode-active.json` (or
`/tmp/hunkle-<euid>/hunkle/opencode-active.json`). Hunkle follows changes via
its existing workspace-open queue and mirrors the tab strip as clickable cards.
Cards show repository and branch inside, the OpenCode session title beneath, and active and busy/attention/unread status.
Clicking a card focuses that already-open tab through the CLI's public tab API.
Hover + F2 in Hunkle renames the OpenCode session through its session API.
OpenCode remains responsible for sessions and the server.

Add to your V2 `~/.config/opencode/cli.json`:

```json
"plugins": [{ "package": "/home/spoon/code/ocplugins/hunkle-sync" }]
```

An existing snapshot fills Hunkle's strip at startup but does not move its workspace;
switch OpenCode session tabs to follow a project. Status updates never trigger a
workspace open. Snapshots are atomic and deduplicated, using the CLI's reactive cache.
No server plugin or polling of session APIs is required.

The latest CLI to switch tabs owns the handoff; another CLI's background activity
does not replace it. The plugin removes its publication on unload, and Hunkle clears
the strip if the publishing CLI exits unexpectedly. Scroll the strip to browse overflow;
the active card is revealed on tab changes and terminal resizes.

Hunkle sends a small focus or rename request to the publishing plugin's local Unix socket,
addressed to its instance and session ID. Requests are handled immediately without
a polling loop; duplicate requests and requests for closed tabs or old
plugin instances are ignored. The resulting OpenCode selection is published normally,
so workspace opening keeps Hunkle's existing edit/operation guards.
