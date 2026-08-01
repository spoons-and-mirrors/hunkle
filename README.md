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
- A filterable repository browser for local and remote branches plus open GitHub
  pull requests and issues.
- Source-aware diffs with changed-file and line-count summaries, line numbers,
  syntax color, and tinted additions, deletions, and hunk headers.
- Nonblocking worktree refresh when files, the index, branches, or HEAD change
  outside hunkle.
- Automatic OpenCode theme matching, with Catppuccin Macchiato as the fallback.

## Run

A recent Rust toolchain is required. Git is required for repository status,
history, staging, and repository actions. GitHub CLI (`gh`) is optional and
supplies pull requests and issues in the repository browser when installed and
authenticated. GitHub data is prefetched and cached in memory for 15 minutes.

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
| `W`                               | Manage linked Git worktrees from known repositories                                                                                                                                  |
| `F1`, `F2`, `F3` in Explorer      | Switch between Explorer, Worktrees, and Branches tabs                                                                                                                                |
| `N` in Worktrees                  | Create a linked worktree from the selected checkout                                                                                                                                  |
| `b`                               | Open the Explorer modal's Branches tab for branches, pull requests, and issues                                                                                                       |
| `Delete` in Branches              | Delete a local branch, optionally including its tracked remote branch or forcing deletion of unmerged work; checked-out, default, `main`, `master`, and `dev` branches are protected |
| `w`                               | Open or close the Herdr Workspace Manager                                                                                                                                            |
| `F2` in Workspaces                | Rename the selected workspace                                                                                                                                                        |
| `Delete` in Workspaces            | Confirm closing a workspace and its panes, or safely removing a linked worktree                                                                                                      |
| `p`                               | Open workspace presets; create, update, load, or delete saved setups                                                                                                                 |
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

Explorer is the shared modal for workspace-level tools. Click its top tabs or
press `F1`, `F2`, and `F3` to switch between Explorer, Worktrees, and Branches.
In the Explorer tab, **Around Here** shows the ancestor branch, current
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

Worktrees lists the linked Git checkouts belonging to repositories hunkle has
opened or discovered through the active Herdr session. Press `W`, then type to
filter by repository, branch, path, or commit. Press `N` to create a linked
worktree from the selected checkout; enter a new or existing local branch and
its destination path, and Hunkle opens the result after Git creates it. Press
`Enter` or double-click to open a checkout. `Delete` safely removes a selected
linked worktree after confirmation; primary, current, locked, missing, and dirty
worktrees are protected, and Herdr-owned worktrees are removed through Herdr.

When hunkle runs inside Herdr, press `w` to open the Workspace Manager, a
responsive modal backed by Herdr's session snapshot. It presents workspace
hierarchy and agent activity without reducing the width of the main repository
view. Single-click a workspace to open its repository immediately in the current
hunkle without switching Herdr workspaces. Press `F2` to rename the selected
workspace. Press `Enter` or double-click to switch the active Herdr workspace;
after a successful switch, the hidden hunkle restores the repository it showed
before the first click. Use `j`/`k` to navigate or `w`/`Esc` to return to
hunkle. Inventory refresh continues in the background, including while the
manager is closed.

Click `+ New` in the manager to create a Herdr workspace at Hunkle's current
path or a worktree based on the selected workspace, without leaving the current
workspace. Press `p`, or click `Presets`, to open Workspace Presets. Use `n` to
capture the current setup as a new preset, `u` to update the selected preset,
`Enter` to load it, and `Delete` to remove it. Presets preserve workspace paths,
labels, linked-worktree entries, the focused workspace, and Hunkle groups
including empty and folded groups; they are stored in `workspace-snapshots.json`
beside Hunkle's config. Before recall, Hunkle shows how many workspaces and
panes will open or close and requires confirmation. Recall opens missing
workspaces before focusing its saved workspace and closing workspaces outside
the preset, then reconnects groups to the resulting Herdr workspace IDs. Legacy
presets without group metadata preserve currently known group memberships by
matching workspace paths instead of clearing them. Linked worktrees stay
indented beneath their parent workspace and move with that parent rather than
between groups independently. Inside the manager, press `g` to create a group.
Click groups to fold or expand them, and drag parent workspaces onto a group or
back into ungrouped space. A single workspace click opens it in Hunkle; press
`Enter` or double-click to switch to its Herdr workspace. Agents are ordered by
recent Herdr activity, and clicking one focuses its terminal pane directly.
Agent timers accumulate across every session used by the same agent in a
terminal, are shared between Hunkle processes, and persist across restarts in
`agent-timings.json` beside Hunkle's config. Press `Delete` to confirm closing a
selected workspace and all its panes, or safely removing a selected linked
worktree from disk.

## Mouse

- Click header controls to switch views, refresh, open Explorer, or open help.
- Drag the divider between Changes and Diff to resize either panel.
- Click a workspace or agent in the Workspace Manager to select it; click
  outside the modal to close it.
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
  selection.
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
During a slowdown, run `tail -f ~/.local/state/hunkle/hunkle.log`; a
`stalled phase=...` line identifies the main-loop phase that has remained
blocked for at least two seconds.

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
| `app::repository_browser`       | Branch, pull-request, and issue interaction plus cached remote data                                        |
| `app::settings`                 | Settings discovery, legacy fallback, validation, and persistence                                           |
| `app::shortcuts`                | Named command registry, contextual conflict checks, key normalization, overrides, and labels               |
| `app::worktree_manager`         | Known-repository inventory and linked-worktree interaction, creation, filtering, opening, and safe removal |
| `app::workspace_panel`          | Workspace Panel interaction, focus transitions, groups, presets, and background refresh                    |
| `app::workspace_panel::herdr`   | Typed Herdr environment, command, restore, and session-snapshot adapter                                    |
| `app::workspace_panel::presets` | Preset and group persistence, migration, matching, and recall planning                                     |
| `repository_session`            | Active workspace lifecycle, background operations, and completion invalidation policy                      |
| `git`                           | Installed-Git facade, refresh orchestration, worktree operations, and history loading                      |
| `git::graph`                    | Commit capping and deterministic graph-lane projection                                                     |
| `git::inventory`                | Git and local workspace file inventory, ignore, sparse-checkout, and submodule policy                      |
| `ui::preview`                   | Stateful preview styling, wrapping, viewport windows, and hunk geometry                                    |
| `selection`                     | Screen-cell selection, text extraction, and clipboard fallback                                             |
| `tree`                          | Pure worktree and file-tree projection                                                                     |
| `ui`                            | Rendering shell, header, and view dispatch                                                                 |
| `ui::changes`                   | Changes, Files, Diff, and commit workspace                                                                 |
| `ui::history`                   | Current-branch history and all-refs graph                                                                  |
| `ui::overlays`                  | Explorer, worktree manager, repository browser, settings, and help overlays                                |
| `ui::workspace_panel`           | Responsive Herdr Workspace Manager modal                                                                   |
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
