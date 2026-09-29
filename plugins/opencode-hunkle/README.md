# Hunkle sync for OpenCode V2

This directory in the Hunkle repository is the source of truth;
`~/code/ocplugins/hunkle-sync` is a symlink here for OpenCode's plugin loading.

CLI-only plugin that publishes the active session tab's workspace path to
`$XDG_RUNTIME_DIR/hunkle/opencode-active.json` (or
`/tmp/hunkle-<euid>/hunkle/opencode-active.json`). Hunkle follows changes via
its existing workspace-open queue. It does not control sessions or the server.

Add to your V2 `cli.json` (for `oc2`,
`~/.config/opencode-v2/opencode/cli.json`):

```json
"plugins": [{ "package": "/home/spoon/code/ocplugins/hunkle-sync" }]
```

The first snapshot already present when Hunkle starts is a baseline; switch
OpenCode session tabs to follow a workspace. No server plugin is required.
