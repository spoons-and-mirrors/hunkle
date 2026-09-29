# hunkle

Hunkle is a terminal workspace for inspecting and editing repositories and local
directories.

- Inspect, stage, unstage, discard, and commit worktree changes.
- Browse tracked, untracked, and ignored files without entering Git metadata.
- Preview source, diffs, Markdown, images, PDFs, archives, and SQLite databases.
- Edit text files inline with selection, undo/redo, indentation, and optional
  format-on-save.
- Browse current-branch history and an all-refs graph with decorations, change
  totals, and author filtering.
- Run push, fetch, pull-with-rebase, and non-interactive Git commands.
- Open and create linked Git worktrees from the header.
- Search or open repositories, directories, and files through Explorer.
- Follow active OpenCode-tab workspace changes and show existing tabs in an optional strip.
- Use the same repository and preview state across narrow and wide layouts.

## Run

A recent Rust toolchain and Git are required.

```sh
cargo run -p hunkle
cargo run -p hunkle -- /path/to/repository
cargo run -p hunkle -- /path/to/file.json
cargo run -p hunkle -- --tabs
cargo run -p hunkle -- --tabs-mono
```

Hunkle opens exactly the requested path. A Git repository root enables status,
history, graph, and repository actions. Any other directory opens as a local
workspace with file browsing, search, editing, and previews; Hunkle does not
climb into an enclosing repository. Passing a file opens its parent workspace
and selects the file.

`hunkle --tabs` opens a compact OpenCode tab-manager widget: group labels, a blank
row, cards, and session names—no repository header, workspace, or footer. Give
its terminal pane at least **20 columns × 5 rows**. Taller panes leave the
remaining space blank. The widget mirrors the same CLI and saved groups as full
Hunkle, including click-to-focus, drag/drop, group isolation, and hover + F2
rename. It waits for OpenCode if no tabs are open, never opens a repository or
starts an OpenCode server, and ignores workspace-follow events. Press `q` or
`Ctrl+C` to exit.

By default, `hunkle` (or `hunkle /path/to/repository`) opens the normal workspace
without the OpenCode strip. `hunkle --tabs-mono [path]` opens the workspace
**with** the strip above it. Both
workspace modes still follow OpenCode tab switches, so `hunkle --tabs` and a
plain `hunkle` can live in separate panes.

Install a development build for the current checkout with:

```sh
cargo hunkle-install-local
```

The command uses this worktree's ignored `target` directory and keeps its build
independent from other Hunkle worktrees. It is offline by default. Use
`cargo hunkle-install-local-online` once when dependencies are not cached. Use
`cargo hunkle-install-global` only for an explicitly requested global release
install.

## OpenCode V2 tabs

To follow OpenCode V2 CLI tabs, add the local CLI-only plugin to
`~/.config/opencode/cli.json` (keep any existing settings):

```json
"plugins": [{ "package": "/home/spoon/code/ocplugins/hunkle-sync" }]
```

`~/code/ocplugins/hunkle-sync` is a symlink to this repository's
`plugins/opencode-hunkle`, which is the source of truth for the plugin.

For Hunkle-only tabs, set `"tabs": { "mode": "off" }` in the same CLI config.
The plugin imports OpenCode's saved tabs once, then remembers visited sessions
in durable plugin storage and navigates through OpenCode's public router API.
This works with the official OpenCode build. The external list is global across
local CLI instances and survives restarts. With native tabs enabled, their list
continues to be mirrored instead.

With native tabs off, **Ctrl+Tab / Alt+Down** and **Ctrl+Shift+Tab / Alt+Up** cycle
the external list in OpenCode. Use **Close Hunkle tab** in its command palette
or `/hunkle-close` to remove a card without deleting or stopping the session.
Visiting that session again restores its card.

In `--tabs` or `--tabs-mono` mode, Hunkle shows two-row cards with repository and branch,
with the OpenCode session title beneath each card,
active highlight, and busy/attention/unread indicators. Cards follow group order;
within a group they initially follow OpenCode's tab order until rearranged locally.
Scroll over the strip (or swipe horizontally on narrow screens) to see more tabs;
switching tabs in OpenCode reveals the active card's group, brings the card into view,
and opens its project. When a group is isolated, isolation follows the active session's
group; otherwise, a hidden destination group is shown. Activity and title updates
leave your group visibility choices alone.
Click a card to focus that tab in OpenCode; Hunkle follows its confirmed selection.
Hover a card and press **F2** to rename the actual OpenCode session. The editor
starts with its current title; Enter/✓ sends the rename to OpenCode, Esc/× cancels,
and empty names are rejected.
Activity and title updates redraw without reopening
the workspace, and the strip disappears when its OpenCode CLI exits.

The strip has one flat layer of named groups, initially **General**:

- Click **[+]** to create a group; hover a group and press **F2** to rename it
  (right-click also works). Enter/✓ saves, Esc/× cancels.
- Drag a card onto another card in the same group to reorder it, or onto a group
  chip to move it. Both arrangements are saved only in Hunkle; OpenCode's native
  tab order stays unchanged. Releasing without dragging focuses the OpenCode tab.
  Drag over empty strip space or scroll while holding to reach off-screen cards.
- Drag a group label onto another to reorder groups. The order is saved, and
  cards follow it. Drag over the group-row arrows or scroll while holding to reach
  off-screen groups; Esc cancels a drag.
- Click a group to hide/show its cards. Double-click to isolate it; while isolated,
  click another group to switch isolation, or double-click any group to restore
  your previous hide/show choices.
- Scroll over the group row, or use its arrows, to reach more groups.

Group names, card assignments, card order, and visibility are saved in Hunkle's config directory
as `opencode-groups.json`. They survive workspace switches and OpenCode restarts.
Grouping only organizes Hunkle's strip; the OpenCode plugin supplies the open-tab list.

Herdr normally reserves right-click for its pane menu. To forward Ctrl+right-click
to Hunkle, set `right_click_passthrough_modifier = "ctrl"` under `[ui]` in
`~/.config/herdr/config.toml`. Hover + F2 needs no right-click passthrough.

The plugin publishes only changed tab snapshots to
`$XDG_RUNTIME_DIR/hunkle/opencode-active.json` (or
`/tmp/hunkle-<euid>/hunkle/opencode-active.json`). Existing cards appear at Hunkle
startup without moving its workspace; later tab switches use the usual workspace-open
queue. With multiple OpenCode CLIs, the latest tab switch selects which CLI is mirrored;
background activity from another CLI does not claim the handoff. Hunkle does not own
OpenCode sessions, transcripts, models, or terminal layouts.

Clicks use the publishing plugin's local Unix socket beside the snapshot. The
plugin handles requests without polling and focuses only an already-open tab
in the addressed CLI. Stale clicks cannot reopen a closed tab or target another CLI.

## Keys

Defaults can be changed under **Settings > Shortcuts**. Structural text-editing
keys and `Esc` remain fixed. `Ctrl+C` copies an inline-editor selection and is
the emergency quit command elsewhere.

| Key | Action |
| --- | --- |
| `F1`, `F2` | Show Changes or Files |
| `g` | Show or hide the Git graph |
| `/` | Search the active workspace |
| `j`, `k`, `Home`, `End` | Move the active selection |
| `PageUp`, `PageDown` | Scroll the active preview |
| `h`, `l`, `Left`, `Right` | Navigate trees and hunk mode |
| `Enter` | Open or toggle the selected item |
| `Space` | Stage or unstage an entry, or stage a hunk |
| `Delete` | Discard selected unstaged changes after confirmation |
| `Ctrl+Delete` | Delete the selected Files entry after confirmation |
| `u` | Unstage all changes |
| `c` | Focus the commit editor |
| `Ctrl+Enter` | Commit, or save and close the inline editor |
| `z` | Toggle preview wrapping |
| `m` | Toggle rendered Markdown and source |
| `e`, `E` | Open the selected file or configure the editor |
| `Shift+F2` | Rename the selected Files entry |
| `Ctrl+S` | Save/format an inline edit, or format the selected file |
| `r` | Refresh |
| `o` | Open Explorer |
| `s` | Open Settings |
| `x` | Open repository Actions |
| `G` | Open Git command input |
| `?` | Open Help |
| `q` | Quit |

## Files And Previews

Files uses terminal-safe one-cell glyphs and theme colors to distinguish common
file types. Git status colors remain independent from file-type styling.

Click a directory to expand it and a file to preview it. Click source, or an
added/context line in an unstaged diff, to edit the working file inline. The
editor supports mouse and keyboard selection, word selection, undo/redo,
indent/outdent, clipboard operations, and gutters for repository and unsaved
changes. Markdown files can switch between source and rendered views.

On Sixel terminals, images refine progressively: an immediate truecolor
halfblock preview is followed by a detailed 64-color Sixel frame and then the
final 256-color Sixel frame. The two Sixel encoders run concurrently after one
shared resize, and a completed final frame is never replaced by a late preview.
`Fast` uses the 256-color final palette without diffusion; `Quality` adds
diffusion for smoother tonal transitions. While switching images, the current
frame stays painted until the replacement halfblocks are ready, and the final
Sixel overwrites that bridge without an intermediate clear.

Use `+` in the Files header to create a file or folder. Drag a Files entry onto a
folder or the Files header to move it. Right-click a repository file or click its
checkbox to stage or unstage it.

## Explorer And Worktrees

Explorer's **Around Here** pane shows the ancestor path, current directory, and
child directories. **Contents** shows entries in the current directory. Type to
search by fuzzy name, relative path, absolute path, or `~/...`; `Tab` accepts the
best completion. `Enter` opens a directory or opens a file's parent workspace
with that file selected. `Ctrl+F` saves or removes the current directory as a
header favorite.

The **WORKTREE** header card lists Git-linked checkouts for the active repository.
Filter by branch, path, or commit. Press `Ctrl+N` in the picker to create a
linked worktree. Hunkle-created worktrees live under
`$XDG_DATA_HOME/hunkle/worktrees`, or `~/.local/share/hunkle/worktrees` when the
environment variable is unset.

## Settings

Settings are stored as `key=value` pairs in
`$XDG_CONFIG_HOME/hunkle/config`, normally `~/.config/hunkle/config`. On Windows,
Hunkle uses `%APPDATA%\hunkle\config`.

General settings cover auto-fetch, fetch interval, format-on-save, media preview
protocol, Sixel quality, and the external editor command. The Shortcuts page
captures a replacement with `Enter` or a click, resets an override with
`Delete`, rejects conflicting bindings, and stores only overrides. Pane widths
are persisted in terminal cells.

## Theme

Hunkle uses its built-in Catppuccin Macchiato palette for deterministic colors.

## Diagnostics

Hunkle writes lifecycle, workspace-loading, file-indexing, slow-phase, and
watchdog reports to `$XDG_STATE_HOME/hunkle/hunkle.log`, normally
`~/.local/state/hunkle/hunkle.log`. Set `HUNKLE_LOG` to choose another path. The
log rotates at 4 MiB.

## Architecture

| Module | Responsibility |
| --- | --- |
| `main` | Terminal setup, cleanup, and event loop |
| `diagnostics` | Rotating performance log and watchdog |
| `app` | Input routing, workspace state, mutations, and notices |
| `app::changes` | Changes/Files selection, navigation, and targets |
| `app::explorer` | Workspace discovery, fuzzy search, and navigation |
| `app::file_editor` | Inline editing and safe atomic persistence |
| `app::linked_worktrees` | Git-authoritative linked-worktree catalog |
| `app::opencode_presence` | Existing OpenCode tabs and active-workspace following |
| `app::settings` | Settings validation and persistence |
| `app::shortcuts` | Named commands, conflicts, and overrides |
| `repository_session` | Workspace lifecycle, background work, and stale-result rejection |
| `git` | Installed-Git facade, refreshes, history, and worktrees |
| `ui::preview` | Preview styling, wrapping, windows, and hunk geometry |
| `ui::workspace` | Responsive structural composition |
| `ui::changes` | Changes, Files, Diff, and commit workspace rendering |
| `ui::editor` | Inline-editor layout and presentation |
| `ui::history` | Current-branch history and all-refs graph |
| `ui::overlays` | Explorer, Settings, Help, Actions, and file dialogs |
| `filesystem` | Guarded workspace reads and mutations |
| `selection` | Screen-cell selection and clipboard fallback |
| `theme` | Built-in palette |

Keep Git command details in `git`, operation coordination in
`repository_session`, interaction decisions in `app`, and visual formatting in
`ui`.
