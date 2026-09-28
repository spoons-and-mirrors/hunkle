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
- Open a Norm tab for the active workspace and follow active Norm-tab workspace
  changes.
- Use the same repository and preview state across narrow and wide layouts.

## Run

A recent Rust toolchain and Git are required.

```sh
cargo run -p hunkle
cargo run -p hunkle -- /path/to/repository
cargo run -p hunkle -- /path/to/file.json
```

Hunkle opens exactly the requested path. A Git repository root enables status,
history, graph, and repository actions. Any other directory opens as a local
workspace with file browsing, search, editing, and previews; Hunkle does not
climb into an enclosing repository. Passing a file opens its parent workspace
and selects the file.

Install a development build for the current checkout with:

```sh
cargo hunkle-install-local
```

The command uses this worktree's ignored `target` directory and keeps its build
independent from other Hunkle worktrees. It is offline by default. Use
`cargo hunkle-install-local-online` once when dependencies are not cached. Use
`cargo hunkle-install-global` only for an explicitly requested global release
install.

## Norm

Click the green **AGENT** header card or press `Ctrl+Space` to ask a running Norm
TUI to open a tab at Hunkle's active workspace. Hunkle passes the workspace path
to `norm open`; it does not create or manage the tab's session, process, model,
conversation, transcript, or terminal layout.

Hunkle passively watches the same-user Norm presence socket at
`$XDG_RUNTIME_DIR/norm/daemon.sock`, falling back to
`/tmp/norm-<euid>/norm/daemon.sock`. The first snapshot establishes a baseline.
When an existing Norm instance changes its active tab within the same daemon
epoch, Hunkle opens that tab's workspace. A daemon replacement establishes a new
baseline instead of moving Hunkle. Hunkle does not start, attach to, or control a
Norm TUI.

These are Hunkle's only Norm responsibilities.

## OpenCode V2 tabs

To follow OpenCode V2 CLI tabs, add the local CLI-only plugin to your V2
`cli.json` (for the `oc2` launcher, `~/.config/opencode-v2/opencode/cli.json`;
keep any existing settings):

```json
"plugins": [{ "package": "/absolute/path/to/hunkle/plugins/opencode-hunkle" }]
```

With OpenCode tabs enabled, switching to a session tab publishes its workspace
to `$XDG_RUNTIME_DIR/hunkle/opencode-active.json` (or
`/tmp/hunkle-<euid>/hunkle/opencode-active.json`). Hunkle follows later changes
through its usual workspace-open queue. An existing snapshot at Hunkle startup
is only a baseline. This is a one-way local handoff: Hunkle does not inspect or
manage OpenCode sessions, transcripts, models, or terminal layouts.

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
| `Ctrl+Space` | Open a Norm tab for the active workspace |
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
| `app::norm_presence` | Norm tab opening and active-workspace following |
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
