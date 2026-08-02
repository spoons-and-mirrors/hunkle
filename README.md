# hunkle

- A collapsible worktree tree with per-file added/deleted line counts for
  inspecting, staging, unstaging, and committing changes.
- A switchable repository file tree that includes tracked, untracked, and
  Git-ignored content, with syntax-colored inline editing and rendered Markdown
  previews.
- Local workspaces for browsing, searching, and previewing directories that are
  not Git repositories.
- A resizable current-branch history shelf with HEAD, branch, remote, and tag
  decorations; selecting a commit shows its patch.
- A repository Actions menu for committing, pushing, fetching, pulling with
  rebase, and running non-interactive Git commands with captured output.
- An all-refs commit graph showing branches, remotes, tags, authors, dates,
  hashes, lazy-loaded line-change totals, and interactive author filtering. Drag
  any vertical header separator to resize the adjacent columns.
- Herdr-aware agent launching from any known repository or linked worktree into
  a selected pane in the active tab.
- Source-aware diffs with changed-file and line-count summaries, line numbers,
  syntax color, and tinted additions, deletions, and hunk headers.
- Nonblocking worktree refresh when files, the index, branches, or HEAD change
  outside hunkle.
- Automatic OpenCode theme matching, with Catppuccin Macchiato as the fallback.

## Run

A recent Rust toolchain is required. Git is required for repository status,
history, staging, and repository actions.

```sh
cargo run -p hunkle
cargo run -p hunkle -- /path/to/repository
```

hunkle opens exactly the current or requested directory. When that directory is
a Git repository root, Git status and history are available. The Changes pane
uses the Git graph as its detail surface while the working tree is clean, then
returns to the diff as soon as changes appear. Any other directory opens as a
local file workspace with recursive file browsing, fuzzy search, and previews;
it never climbs into an enclosing repository.

## Keys

These are the default bindings. Open Settings and select **Shortcuts** to
reassign named commands; structural editing/navigation keys and `Esc` remain
fixed. `Ctrl+C` copies a native selection in the inline editor and remains the
emergency quit command elsewhere.

| Key                               | Action                                                                                                                                                                               |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Tab`                             | Switch between Changes and Files, closing Graph if necessary                                                                                                                         |
| `g`                               | Show or hide the Git graph without changing the selected Changes/Files pane                                                                                                          |
| `j`, `k`                          | Move selection; scroll oversized hunks by 10 rows                                                                                                                                    |
| `Home`, `End`                     | First or last row                                                                                                                                                                    |
| `PageUp`, `PageDown`              | Scroll the selected file's diff                                                                                                                                                      |
| `z`                               | Toggle wrapping in the Diff or File preview (on by default)                                                                                                                          |
| `e`, `E`                          | Open the selected file in your editor, or configure the editor                                                                                                                       |
| `m`                               | Toggle rendered Markdown and source for Markdown files in Files                                                                                                                      |
| `F1`                              | Send a command or prompt to the Herdr pane directly below Hunkle, creating it when needed                                                                                            |
| `F2`                              | Rename the selected file or folder in Files                                                                                                                                          |
| `F3`                              | Fuzzy-search repository files from the main view                                                                                                                                     |
| `Ctrl+Delete`                     | Permanently delete the selected file or folder from Files after confirmation                                                                                                         |
| `Ctrl+S`                          | Save and stay in the inline editor, optionally formatting; otherwise format the selected Files entry                                                                                |
| `Ctrl+Enter` in the inline editor | Save and close the inline editor                                                                                                                                                    |
| `Ctrl+A`, `Ctrl+C`, `Ctrl+X`      | Select all, copy, or cut text in the inline editor                                                                                                                                  |
| `Ctrl+Z`, `Ctrl+Shift+Z`, `Ctrl+Y` | Undo or redo the active file's inline edits                                                                                                                                          |
| `Tab`, `Shift+Tab` in the inline editor | Indent or outdent the selected lines as one undoable edit                                                                                                                        |
| `Esc` in the inline editor        | Collapse a selection, close the editor, or press twice to discard unsaved edits                                                                                                     |
| `h`, `l`, `Left`, `Right`         | Navigate the tree; Right enters/stages in hunk mode and Left exits it                                                                                                                |
| `Enter`                           | Toggle the selected directory                                                                                                                                                        |
| `Space`                           | Stage or unstage the selected entry, or stage the selected hunk                                                                                                                      |
| `Delete` in Changes               | Discard the selected file's unstaged changes after confirmation; staged changes are preserved                                                                                        |
| `Right`, `l` in hunk mode         | Stage the selected hunk                                                                                                                                                              |
| `a`                               | Show or hide the Agents section                                                                                                                                                      |
| `u`                               | Unstage all changes                                                                                                                                                                  |
| `c`                               | Focus the commit message editor                                                                                                                                                      |
| `Enter`, `Ctrl+Enter`             | New commit-message line, create commit                                                                                                                                               |
| `Left`, `Right`, `Home`, `End`    | Move within the commit message                                                                                                                                                       |
| `Ctrl+A`                          | Select the complete commit message                                                                                                                                                   |
| `Ctrl+Backspace`, `Alt+Backspace` | Delete the previous commit-message word                                                                                                                                              |
| `r`                               | Refresh                                                                                                                                                                              |
| `o`                               | Open Explorer                                                                                                                                                                        |
| `s`                               | Open settings                                                                                                                                                                        |
| `x`                               | Open repository Actions                                                                                                                                                              |
| `G`                               | Open Git command                                                                                                                                                                     |
| `?`                               | Help                                                                                                                                                                                 |
| `q`                               | Quit                                                                                                                                                                                 |

Files uses terminal-safe one-cell glyphs and theme colors to distinguish source,
configuration, documentation, media, archive, and other common file types.
Folder names use the primary text tier, ordinary files use a softer intermediate
tier, and tree connectors stay faint; filename colors remain reserved for Git
status, so type and repository state stay independently visible.

Explorer's **Around Here** pane shows the ancestor branch, current
directory, and its child directories while **Contents** lists the directories
and files inside the current location; `Tab` switches panes. Start typing and
Explorer immediately captures the text as a new PATH query instead of applying
normal application or letter-based navigation shortcuts. Search accepts fuzzy
directory names, relative paths, absolute paths, and `~/...`; path matches
include hidden directories and files alongside a live child preview. The path
field supports cursor editing and `Ctrl+Backspace` or `Alt+Backspace` removes
the previous path segment. `Tab` accepts the best completion, adding a trailing
`/` for directories, and `Enter` opens a repository, navigates into a directory,
or opens a file's parent workspace with that file selected. Press `Ctrl+F` to
name and save the current directory as a persistent header favorite; click its
card to return there, or press `Ctrl+F` again while that directory is active to
remove it. Hidden directories are browseable, `.config` participates in
background search, and only Git metadata and expensive generated trees are
omitted from indexing.

The **WORKTREE** header card lists linked Git checkouts for the active
repository. Type to filter by branch, path, or commit, and press `Enter` or
double-click to open a checkout. Press `Ctrl+N` in the popover to create a
linked worktree directly with Git; enter its name and starting branch, and
Hunkle opens the result after creation. Hunkle-created worktrees are stored in
`$XDG_DATA_HOME/hunkle/worktrees`, or `~/.local/share/hunkle/worktrees` when
`XDG_DATA_HOME` is unset.

When Hunkle runs inside Herdr, click the green **AGENT** header card or press
`Ctrl+Space` to start an OpenCode agent at the repository, worktree, and branch
shown in the header. Choose which non-Hunkle pane in the active Herdr tab to
replace. The displaced pane is parked in its own tab, named after its starting
directory, so it cannot be merged into another agent's saved layout.

The Agents section acts as a live per-agent layout switcher. Clicking an agent
restores its complete pane layout around the fixed Hunkle pane, keeps keyboard
focus in Hunkle, and opens the selected agent's working directory. The currently
visible layout is parked in the selected layout's former tab. Existing terminals
move between tabs without restarting their processes or losing scrollback. Every
pane other than Hunkle belongs to the displayed agent, including panes beside or
below Hunkle. Hunkle remembers each agent's geometry across restarts. By default,
the section lists and operates on agents in Hunkle's Herdr workspace only; enable
**Cross-workspace agents** in Settings to include agents from other Herdr
workspaces. Click **⛶** at the top-right of Hunkle to temporarily fill the Herdr
tab; click it again to restore the exact previous pane layout. Zoomed tabs must be
restored before agent layouts can be exchanged.

Hover a live agent card and choose **STASH** to save its repository, worktree,
branch, harness, and session before closing its Herdr pane and process. The
**STASH** control in the Agents section header replaces the live cards with saved
agents. Click a saved card to choose a pane and resume that exact OpenCode
session; the saved entry is removed only after the agent starts successfully.

Hunkle remembers the last repository opened in each Herdr pane. Relaunching it
from the same shell directory restores that repository; changing the shell
directory or passing an explicit path starts from the requested location instead.

Agent timers accumulate across every session used by the same agent in a
terminal, are shared between Hunkle processes, and persist across restarts in
`agent-timings.json` beside Hunkle's config.

## Mouse

- Click header controls to switch views, refresh, open Explorer, or open help.
- Drag the divider between Changes and Diff to resize either panel.
- Hover an agent to preview its latest user message. Scroll over the card or
  preview to cycle through the last five messages, or hover the history squares
  in the preview header to inspect one directly; click the card to display its
  Herdr tab layout beside Hunkle.
- Drag the History section header vertically to resize the current-branch commit
  shelf.
- Click `x ACTIONS` above History to push, fetch, pull with rebase, or run a
  custom Git command.
- Click or scroll History to inspect a commit's patch; click a Changes file to
  return to its current diff, or double-click it to open its current content in
  Files.
- Click a directory to expand or collapse it. Click a file's right-aligned
  checkbox or right-click its row to stage or unstage it.
- Click `CHANGES` or `FILES` in the left header to switch modes; clicking a
   repository file previews its contents. Click plain source, or an added/context
   line in an unstaged diff, to edit the working file inline.
- In the inline editor, drag to select source text, double-click a word, and use
   `Shift` with navigation keys to extend a selection. Typing, paste, and delete
   replace the selected text; editor selection is separate from review-pane copy
   selection. `Tab` and `Shift+Tab` indent or outdent selected lines, and the
   mouse wheel scrolls the editor viewport without moving the cursor.
- Inline editor gutters show added, removed, and modified lines from the current
  diff, plus unsaved lines changed during the current editing session.
- Markdown files in Files show a top-right `Preview` button for switching
  between rendered Markdown and source.
- Click `+` in the Files header to create a file or folder. Drag a Files entry
  onto a folder or the Files header to move it.
- The wheel pans Changes and Files as viewports without changing the selected
  file; click a visible row to select it.
- Right-click interactions are delivered to hunkle while terminal mouse capture
  is enabled; Herdr does not consume them first.
- Use the wheel over Diff or Graph to scroll that surface.
- Click the Graph `AUTHOR` header to include or exclude commits by author.
- Drag the one-column Diff scrollbar or click its track to move quickly through
  large patches.
- Click the Changes `Stage all` checkbox to stage everything; click it again
  when checked to unstage everything.
- Click the commit editor inside Changes, type a message, use the mouse wheel to
  scroll longer messages, and press `Ctrl+Enter` to commit.
- When `opencode` is installed, click `✦` below the commit editor to generate a
  message from the staged diff, or from the unstaged diff when nothing is
  staged. Hunkle streams the complete diff directly to OpenCode without
  file-attachment or tool-output truncation, deletes the one-shot OpenCode
  session after generation, uses `openai/gpt-5.6-sol` with low reasoning, and
  never overwrites a message edited while generation is running.
- When `opencode` is installed, click `MC` below the commit editor to run Magic
  Commit in the background. Its one-shot OpenCode task organizes current
  changes into logical commits using selected index patches. Hunkle denies the
  task access to file-editing tools and unrelated shell commands, so it cannot
  edit worktree files or perform unrelated Git operations. The task remains
  scoped to its originating repository when you switch workspaces; click its
  spinning `MC` control again to cancel it.
- In Explorer, click the path field to type, a row to select it, or a preview to
  continue completing. Drag the divider between AROUND HERE and CONTENTS to
  resize the panes; its exact width persists across launches. AROUND HERE shows
  the ancestor branch, current folder, and its child folders so you can traverse
  up or down entirely in the left pane. Double-click a folder or press `Enter`
  to make it the current location, updating PATH, AROUND HERE, and CONTENTS
  together, including when that folder is a Git repository. Double-click the
  parent row to go back. Use the `Open current repository` or
  `Open current location` row to open that directory as a workspace; files open
  on double-click or `Enter`.
- Drag across visible text to select it and automatically copy it to the
  clipboard. In Files, hold `Shift` while dragging to select text instead of
  moving an entry. Selection stays within the panel where the drag starts.

## Settings

Settings are saved as `key=value` pairs in `$XDG_CONFIG_HOME/hunkle/config`, or
`~/.config/hunkle/config` when `XDG_CONFIG_HOME` is unset. On Windows, hunkle
uses `%APPDATA%\hunkle\config`. Existing settings are loaded from the old
`gitui` location when no hunkle config exists. The **OpenCode** page selects the
model and reasoning variant used for generated commit messages; model IDs are
the values reported by `opencode models`, and `Default` reasoning omits the
variant for models that do not support one. The **Shortcuts** page captures a
replacement key with `Enter` or a mouse click, resets an override with `Delete`,
rejects conflicts in overlapping contexts, and stores only overrides as
`shortcut.<command>=<key>`. The first `e` press asks for an editor command such
as `nvim`, `micro`, or `code --wait`; hunkle saves it, suspends the TUI, and
runs the editor interactively. Press `E` to change it later. Auto-fetch can
periodically run `git fetch --all --prune` for the active repository without
blocking the interface; its interval is configurable from 1 to 1440 minutes. The
last manually selected Changes width, Explorer left-pane width, and Agents
height are stored as exact terminal-cell counts.

## Theme

hunkle uses the active OpenCode TUI theme when OpenCode is installed. It follows
OpenCode's `tui.json`/`tui.jsonc` selection first, then
`~/.local/state/opencode/kv.json`, and supports all bundled OpenCode themes plus
user and project themes under `opencode/themes/*.json`. If no usable theme is
found, hunkle uses Catppuccin Macchiato.

## Diagnostics

hunkle writes lifecycle, workspace loading, file indexing, slow main-loop
phases, and watchdog stall reports to `$XDG_STATE_HOME/hunkle/hunkle.log`, or
`~/.local/state/hunkle/hunkle.log` when `XDG_STATE_HOME` is unset. Set
`HUNKLE_LOG` to use another file. The log rotates to `hunkle.log.old` at 4 MiB.
Every line includes the originating process ID. During a slowdown, run
`tail -f ~/.local/state/hunkle/hunkle.log`; a `stalled phase=...` line identifies
the main-loop phase that has remained blocked for at least two seconds. Each
stalled activity is reported once rather than once per second.

## Architecture

The binary stays deliberately direct, with modules split by the behavior they
own:

| Module                          | Responsibility                                                                                             |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `main`                          | Terminal setup, cleanup, and event loop                                                                    |
| `diagnostics`                   | Rotating performance log, slow-phase timing, and main-loop watchdog                                        |
| `app`                           | Global input routing, workspace state, Git mutations, and notices                                          |
| `app::actions`                  | Repository Actions, command input, and captured results                                                    |
| `app::author_filter`            | Repository-scoped Graph author filtering and selection                                                     |
| `app::changes`                  | Changes-screen selection, navigation, semantic targets, and displayed content                              |
| `app::changes::preview_loader`  | Coalesced asynchronous file, commit, and diff preview loading                                              |
| `app::commit_summary`           | Lazy, repository-scoped cache of commit file and line-change summaries                                     |
| `app::explorer`                 | Workspace discovery, navigation, fuzzy search, and semantic interaction targets                            |
| `app::file_editor`              | Bounded UTF-8 editor state, selections, undo/redo, indentation, and safe atomic persistence                 |
| `app::linked_worktrees`         | Git-authoritative linked-worktree catalog, discovery memory, and destination metadata                       |
| `app::settings`                 | Settings discovery, legacy fallback, validation, and persistence                                           |
| `app::shortcuts`                | Named command registry, contextual conflict checks, key normalization, overrides, and labels               |
| `app::herdr_session`            | Herdr session snapshots, agent activity, timing, pane layouts, and linked-worktree observations             |
| `app::herdr_session::client`    | Typed Herdr environment, command, pane-layout, agent replacement, and session-snapshot adapter              |
| `repository_session`            | Active workspace lifecycle, background operations, and completion invalidation policy                      |
| `git`                           | Installed-Git facade, refresh orchestration, worktree operations, and history loading                      |
| `git::graph`                    | Commit capping and deterministic graph-lane projection                                                     |
| `git::inventory`                | Git and local workspace file inventory, ignore, sparse-checkout, and submodule policy                      |
| `ui::preview`                   | Stateful preview styling, wrapping, viewport windows, and hunk geometry                                    |
| `selection`                     | Screen-cell selection, text extraction, and clipboard fallback                                             |
| `tree`                          | Pure worktree and file-tree projection                                                                     |
| `ui`                            | Rendering shell, header, and view dispatch                                                                 |
| `ui::changes`                   | Changes, Files, Diff, and commit workspace                                                                 |
| `ui::editor`                    | Inline-editor layout, cursor, selection, wrapping, and gutter presentation                                 |
| `ui::history`                   | Current-branch history and all-refs graph                                                                  |
| `ui::overlays`                  | Explorer, settings, help, actions, command, and file-operation overlays                                    |
| `ui::agents`                    | Agent activity, destination metadata, timing, and layout-switching controls                                 |
| `ui::text`                      | Deterministic source and diff presentation                                                                 |
| `theme`                         | Theme discovery, resolution, and palette data                                                              |

Keep Git command details in `git`, operation scheduling in `repository_session`,
interaction decisions in `app`, and visual formatting in `ui`. Add another
module only when it can own a cohesive behavior behind a smaller interface than
the implementation it hides.

Custom commands run as Git arguments from the active repository with prompts and
editors disabled. They do not invoke a shell, so pipes, redirects, and other
shell syntax are never interpreted.

## Scope

This first version stays deliberately small. It uses the installed Git
executable so ordering, configuration, worktrees, refs, and repository formats
behave like Git itself. The graph uses terminal-native Unicode rather than
terminal-specific image protocols.
