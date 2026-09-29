# Hunkle sync for OpenCode V2

This directory in the Hunkle repository is the source of truth;
`~/code/ocplugins/hunkle-sync` is a symlink here for OpenCode's plugin loading.

CLI-only plugin that publishes open tabs and the active session's workspace path to
`$XDG_RUNTIME_DIR/hunkle/opencode-active.json` (or
`/tmp/hunkle-<euid>/hunkle/opencode-active.json`). Hunkle follows changes via
its existing workspace-open queue and mirrors the tab strip as clickable cards.
Cards show repository and branch inside, the OpenCode session title beneath, and active and busy/attention/unread status.
Clicking a card selects that session through the CLI's public router API, even
when OpenCode's native tabs are disabled.
Hover + F2 in Hunkle renames the OpenCode session through its session API.
OpenCode remains responsible for sessions and the server.

Add to your V2 `~/.config/opencode/cli.json`:

```json
"plugins": [{ "package": "/home/spoon/code/ocplugins/hunkle-sync" }]
```

To make Hunkle the only visible tab interface, also set `"tabs": { "mode": "off" }`.
This works with the official OpenCode V2 CLI; no custom OpenCode build is needed.
When native tabs are on, the plugin continues mirroring their list. When off,
it imports the saved native tabs once and remembers subsequently visited sessions
in OpenCode's durable plugin storage (`open-tabs`). This external list is global,
shared across local CLI instances, and survives plugin reloads and CLI restarts.
Hunkle's grouping and card order remain separate presentation state.

With native tabs off, use **Ctrl+Tab / Alt+Down** and **Ctrl+Shift+Tab / Alt+Up**
to cycle the external list. **Close Hunkle tab** in OpenCode's command palette
(or `/hunkle-close`) removes the current card without deleting or stopping its
session. Opening that session again adds it back. Deleted sessions are removed;
temporary connection failures do not discard cards.

An existing snapshot fills Hunkle's strip at startup but does not move its workspace;
switch OpenCode session tabs to follow a project. Status updates never trigger a
workspace open. Snapshots are atomic and deduplicated, using the CLI's reactive cache.
In external mode the plugin hydrates lightweight session, location, and attention
data on opening/reconnection, including known child sessions; live events carry
subsequent updates. It does not prefetch transcripts or poll session APIs.

The latest CLI to switch tabs owns the handoff; another CLI's background activity
does not replace it. The plugin removes its publication on unload, and Hunkle clears
the strip if the publishing CLI exits unexpectedly. Scroll the strip to browse overflow;
the active card is revealed on tab changes and terminal resizes.

Hunkle sends a small focus or rename request to the publishing plugin's local Unix socket,
addressed to its instance and session ID. Requests are handled immediately without
a polling loop; duplicate requests and requests for closed tabs or old
plugin instances are ignored. The resulting OpenCode selection is published normally,
so workspace opening keeps Hunkle's existing edit/operation guards.
