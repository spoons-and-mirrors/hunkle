# hunkle

- A collapsible worktree tree with per-file added/deleted line counts for inspecting, staging, unstaging, and committing changes.
- A switchable repository file tree that includes tracked, untracked, and Git-ignored content, with read-only, syntax-colored previews and rendered Markdown.
- Local workspaces for browsing, searching, and previewing directories that are not Git repositories.
- A resizable current-branch history shelf with HEAD, branch, remote, and tag decorations; selecting a commit shows its patch.
- A repository Actions menu for committing, pushing, fetching, pulling with rebase, and running non-interactive Git commands with captured output.
- An all-refs commit graph showing branches, remotes, tags, authors, dates, hashes, lazy-loaded line-change totals, and interactive author filtering.
- A filterable repository browser for local and remote branches plus open GitHub pull requests and issues.
- Source-aware diffs with changed-file and line-count summaries, line numbers, syntax color, and tinted additions, deletions, and hunk headers.
- Nonblocking worktree refresh when files, the index, branches, or HEAD change outside hunkle.
- Automatic OpenCode theme matching, with Catppuccin Macchiato as the fallback.

## Run

A recent Rust toolchain is required. Git is required for repository status, history, staging, and repository actions. GitHub CLI (`gh`) is optional and supplies pull requests and issues in the repository browser when installed and authenticated. GitHub data is prefetched and cached in memory for 15 minutes.

```sh
cargo run -p hunkle
cargo run -p hunkle -- /path/to/repository
```

hunkle opens exactly the current or requested directory. When that directory is a Git repository root, Git status and history are available. Any other directory opens as a local file workspace with recursive file browsing, fuzzy search, and previews; it never climbs into an enclosing repository.

## Install

Install or replace `hunkle` from the current checkout:

```sh
cargo install --path . --force --locked
```

## Keys

| Key | Action |
|---|---|
| `1`, `2`, `Tab` | Changes, Graph, or switch view |
| `j`, `k` | Move selection; scroll oversized hunks by 10 rows |
| `Home`, `G` | First or last row |
| `PageUp`, `PageDown` | Scroll the selected file's diff |
| `Alt+w` | Toggle wrapping in the Diff or File preview |
| `e`, `E` | Open the selected file in your editor, or configure the editor |
| `f` | Switch the left pane between Changes and Files |
| `m` | Toggle rendered Markdown and source for Markdown files in Files |
| `F1` | Send a command or prompt to the Herdr pane directly below Hunkle, creating it when needed |
| `F2` | Rename the selected file or folder in Files |
| `F3` | Fuzzy-search repository files from anywhere |
| `Ctrl+Delete` | Permanently delete the selected file or folder from Files after confirmation |
| `Ctrl+S` | Format the selected file using an available file-type formatter |
| `h`, `l`, `Left`, `Right` | Navigate the tree; Right enters/stages in hunk mode and Left exits it |
| `Enter` | Toggle the selected directory |
| `Space` | Stage or unstage the selected entry, or stage the selected hunk |
| `Delete` in Changes | Discard the selected file's unstaged changes after confirmation; staged changes are preserved |
| `Right`, `l` in hunk mode | Stage the selected hunk |
| `a`, `u` | Stage all or unstage all |
| `c` | Focus the commit message editor |
| `Enter`, `Ctrl+Enter` | New commit-message line, create commit |
| `Left`, `Right`, `Home`, `End` | Move within the commit message |
| `Ctrl+A` | Select the complete commit message |
| `Ctrl+Backspace`, `Alt+Backspace` | Delete the previous commit-message word |
| `r` | Refresh |
| `o` | Open Explorer |
| `W` | Manage linked Git worktrees from known repositories |
| `N` in Worktrees | Create a linked worktree from the selected checkout |
| `b` | Browse branches, pull requests, and issues |
| `Delete` in Branches | Delete a local branch, optionally including its tracked remote branch or forcing deletion of unmerged work; checked-out, default, `main`, `master`, and `dev` branches are protected |
| `w` | Cycle the Herdr Workspaces and Agents rail through left, right, and off |
| `F2` in Workspaces | Rename the selected workspace |
| `Delete` in Workspaces | Confirm closing a workspace and its panes, or safely removing a linked worktree |
| `p` | Open workspace presets; create, update, load, or delete saved setups |
| `s` | Open settings |
| `x` | Open repository Actions |
| `g` | Open Git command |
| `?` | Help |
| `q` | Quit |

Files uses terminal-safe one-cell glyphs and theme colors to distinguish source, configuration, documentation, media, archive, and other common file types. Folder names use the primary text tier, ordinary files use a softer intermediate tier, and tree connectors stay faint; filename colors remain reserved for Git status, so type and repository state stay independently visible.

In Explorer, **Around Here** shows ancestors and neighboring directories while **Contents** shows what is inside the current location; `Tab` switches panes and `~` jumps home. Start typing a folder name, press `p` to search from an empty field, or `/` to start an absolute path. Search accepts fuzzy directory names, relative paths, absolute paths, and `~/...`; path matches include a live child preview. The path field supports cursor editing and `Ctrl+Backspace` or `Alt+Backspace` removes the previous path segment. `Tab` accepts the best completion with a trailing `/`, and `Enter` opens a repository or navigates into a directory. Hidden directories are browseable, `.config` participates in background search, and only Git metadata and expensive generated trees are omitted from indexing.

Worktrees lists the linked Git checkouts belonging to repositories hunkle has opened or discovered through the active Herdr session. Press `W`, then type to filter by repository, branch, path, or commit. Press `N` to create a linked worktree from the selected checkout; enter a new or existing local branch and its destination path, and Hunkle opens the result after Git creates it. Press `Enter` or double-click to open a checkout. `Delete` safely removes a selected linked worktree after confirmation; primary, current, locked, missing, and dirty worktrees are protected, and Herdr-owned worktrees are removed through Herdr.

When hunkle runs inside Herdr, it can show a Workspaces and Agents rail backed by Herdr's session snapshot. Single-click a workspace to open its repository immediately in the current hunkle without switching Herdr workspaces. Press `F2` to rename the selected workspace. Press `Enter` or double-click to switch the active Herdr workspace; after a successful switch, the hidden hunkle restores the repository it showed before the first click. Use `j`/`k` to navigate or `Esc` to return to hunkle. The rail refreshes in the background and hides automatically on narrow terminals or outside Herdr.

The rail starts on the left. Press `w` to cycle it through the right side, off, and back to the left. Click `+` beside WORKSPACES to create a Herdr workspace at Hunkle's current path or a worktree based on the selected workspace, without leaving the current workspace. Press `p`, or click `Load`, to open Workspace Presets. Use `n` to capture the current setup as a new preset, `u` to update the selected preset, `Enter` to load it, and `Delete` to remove it. Presets preserve workspace paths, labels, linked-worktree entries, the focused workspace, and Hunkle groups including empty and folded groups; they are stored in `workspace-snapshots.json` beside Hunkle's config. Before recall, Hunkle shows how many workspaces and panes will open or close and requires confirmation. Recall opens missing workspaces before focusing its saved workspace and closing workspaces outside the preset, then reconnects groups to the resulting Herdr workspace IDs. Legacy presets without group metadata preserve currently known group memberships by matching workspace paths instead of clearing them. Linked worktrees stay indented beneath their parent workspace and move with that parent rather than between groups independently. Inside the rail, press `g` to create a group. Click groups to fold or expand them, and drag parent workspaces onto a group or back into ungrouped space. A single workspace or agent click only selects it; press `Enter` or double-click to switch. Press `Delete` to confirm closing a selected workspace and all its panes, or safely removing a selected linked worktree from disk.

## Mouse

- Click header controls to switch views, refresh, open Explorer, or open help.
- Drag the divider between Changes and Diff to resize either panel.
- Drag the Workspaces rail divider to resize it on either side of the window.
- Drag the History section header vertically to resize the current-branch commit shelf.
- Click `x ACTIONS` above History to push, fetch, pull with rebase, or run a custom Git command.
- Click or scroll History to inspect a commit's patch; click a Changes file to return to its current diff, or double-click it to open its current content in Files.
- Click a directory to expand or collapse it. Click a file's right-aligned checkbox or right-click its row to stage or unstage it.
- Click `CHANGES` or `FILES` in the left header to switch modes; clicking a repository file previews its contents.
- Markdown files in Files show a top-right `Preview` button for switching between rendered Markdown and source.
- Click `+` in the Files header to create a file or folder. Drag a Files entry onto a folder or the Files header to move it.
- The wheel pans Changes and Files as viewports without changing the selected file; click a visible row to select it.
- Right-click interactions are delivered to hunkle while terminal mouse capture is enabled; Herdr does not consume them first.
- Use the wheel over Diff or Graph to scroll that surface.
- Click the Graph `AUTHOR` header to include or exclude commits by author.
- Drag the one-column Diff scrollbar or click its track to move quickly through large patches.
- Click the Changes `Stage all` checkbox to stage everything; click it again when checked to unstage everything.
- Click the commit editor inside Changes, type a message, use the mouse wheel to scroll longer messages, and press `Ctrl+Enter` to commit.
- When `opencode` is installed, click `✦` below the commit editor to generate a message from the staged diff, or from the unstaged diff when nothing is staged. Hunkle streams the complete diff directly to OpenCode without file-attachment or tool-output truncation, deletes the one-shot OpenCode session after generation, uses `openai/gpt-5.6-sol` with low reasoning, and never overwrites a message edited while generation is running.
- Click the Explorer path field to type, a surrounding location to jump there, a preview to continue completing, or a directory/repository entry to navigate or open it.
- Drag across visible text to select it and automatically copy it to the clipboard. In Files, hold `Shift` while dragging to select text instead of moving an entry. Selection stays within the panel where the drag starts.

## Settings

Settings are saved to `$XDG_CONFIG_HOME/hunkle/config`, or `~/.config/hunkle/config` when `XDG_CONFIG_HOME` is unset. On Windows, hunkle uses `%APPDATA%\hunkle\config`. Existing settings are loaded from the old `gitui` location when no hunkle config exists. The first `e` press asks for an editor command such as `nvim`, `micro`, or `code --wait`; hunkle saves it, suspends the TUI, and runs the editor interactively. Press `E` to change it later. Auto-fetch can periodically run `git fetch --all --prune` for the active repository without blocking the interface; its interval is configurable from 1 to 1440 minutes. The last manually selected Changes width and History height are stored as exact terminal-cell counts.

## Theme

hunkle uses the active OpenCode TUI theme when OpenCode is installed. It follows OpenCode's `tui.json`/`tui.jsonc` selection first, then `~/.local/state/opencode/kv.json`, and supports all bundled OpenCode themes plus user and project themes under `opencode/themes/*.json`. If no usable theme is found, hunkle uses Catppuccin Macchiato.

## Diagnostics

hunkle writes lifecycle, workspace loading, file indexing, slow main-loop phases, and watchdog stall reports to `$XDG_STATE_HOME/hunkle/hunkle.log`, or `~/.local/state/hunkle/hunkle.log` when `XDG_STATE_HOME` is unset. Set `HUNKLE_LOG` to use another file. The log rotates to `hunkle.log.old` at 4 MiB. During a slowdown, run `tail -f ~/.local/state/hunkle/hunkle.log`; a `stalled phase=...` line identifies the main-loop phase that has remained blocked for at least two seconds.

## Architecture

The binary stays deliberately direct, with modules split by the behavior they own:

| Module | Responsibility |
|---|---|
| `main` | Terminal setup, cleanup, and event loop |
| `diagnostics` | Rotating performance log, slow-phase timing, and main-loop watchdog |
| `app` | Global input routing, workspace state, Git mutations, and notices |
| `app::actions` | Repository Actions, command input, and captured results |
| `app::author_filter` | Repository-scoped Graph author filtering and selection |
| `app::changes` | Changes-screen selection, navigation, semantic targets, and displayed content |
| `app::changes::preview_loader` | Coalesced asynchronous file, commit, and diff preview loading |
| `app::commit_summary` | Lazy, repository-scoped cache of commit file and line-change summaries |
| `app::explorer` | Workspace discovery, navigation, fuzzy search, and semantic interaction targets |
| `app::repository_browser` | Branch, pull-request, and issue interaction plus cached remote data |
| `app::settings` | Settings discovery, legacy fallback, validation, and persistence |
| `app::worktree_manager` | Known-repository inventory and linked-worktree interaction, creation, filtering, opening, and safe removal |
| `app::workspace_panel` | Workspace Panel interaction, focus transitions, groups, presets, and background refresh |
| `app::workspace_panel::herdr` | Typed Herdr environment, command, restore, and session-snapshot adapter |
| `app::workspace_panel::presets` | Preset and group persistence, migration, matching, and recall planning |
| `repository_session` | Active workspace lifecycle, background operations, and completion invalidation policy |
| `git` | Installed-Git facade, refresh orchestration, worktree operations, and history loading |
| `git::graph` | Commit capping and deterministic graph-lane projection |
| `git::inventory` | Git and local workspace file inventory, ignore, sparse-checkout, and submodule policy |
| `ui::preview` | Stateful preview styling, wrapping, viewport windows, and hunk geometry |
| `selection` | Screen-cell selection, text extraction, and clipboard fallback |
| `tree` | Pure worktree and file-tree projection |
| `ui` | Rendering shell, header, and view dispatch |
| `ui::changes` | Changes, Files, Diff, and commit workspace |
| `ui::history` | Current-branch history and all-refs graph |
| `ui::overlays` | Explorer, worktree manager, repository browser, settings, and help overlays |
| `ui::workspace_panel` | Herdr Workspaces and Agents rail |
| `ui::text` | Deterministic source and diff presentation |
| `theme` | Theme discovery, resolution, and palette data |

Keep Git command details in `git`, operation scheduling in `repository_session`, interaction decisions in `app`, and visual formatting in `ui`. Add another module only when it can own a cohesive behavior behind a smaller interface than the implementation it hides.

Custom commands run as Git arguments from the active repository with prompts and editors disabled. They do not invoke a shell, so pipes, redirects, and other shell syntax are never interpreted.

## Scope

This first version stays deliberately small. It uses the installed Git executable so ordering, configuration, worktrees, refs, and repository formats behave like Git itself. The graph uses terminal-native Unicode rather than terminal-specific image protocols.
