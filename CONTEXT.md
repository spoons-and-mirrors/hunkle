# Hunkle Domain Context

This glossary records product concepts whose names should stay stable across modules and UI text.

## Workspace

The directory Hunkle has opened. A workspace is either a Git repository root or a local workspace.

## Repository

A Git-backed workspace. Repository data includes the worktree, file inventory, history, graph, refs, and host capabilities.

## Repository session

The application-owned lifecycle for the active Repository or Local workspace. It publishes a bootstrap snapshot, hydrates repository details, schedules and unions scoped refreshes, retries interrupted hydration, and rejects stale background results. Application code submits refresh intents and reconciles completed snapshots with UI state; it does not maintain a parallel refresh queue.

## Local workspace

A directory opened without Git behavior. It supports file browsing, search, previews, and file operations.

## Worktree

The tracked and untracked changes shown in the left CHANGES pane. Staging actions operate on this view even while Graph is visible.

## Linked worktree

A Git checkout registered through `git worktree`. This is distinct from the CHANGES-pane Worktree.

Git inventory is authoritative for whether a linked worktree exists and for its checkout state. Herdr observations do not create catalog entries. Known repositories are discovery memory rather than authoritative topology.

## Linked worktree catalog

The application-owned catalog that reconciles Git inventory, known repository discovery, and Herdr observations. Header pickers and linked-worktree labels consume its snapshot rather than independently reconstructing topology.

## Files

The complete filesystem tree inside the workspace, including Git-ignored content but excluding Git's own metadata directory.

## Explorer

The `o` interaction for finding and opening another workspace. Confirming a file path opens the file's parent directory as a workspace and selects the file.

## Agent destination

The active Git Repository or Linked worktree where Hunkle launches an OpenCode agent. The Repository, Worktree, and Branch header cards define this filesystem destination; clicking Agent uses it directly. Herdr remains the adapter for choosing pane placement and starting the agent.

## Interaction

A focused user flow that owns its transient state and interprets input. An interaction may emit an application effect, such as opening a selected workspace.

## Hit target

A semantic action or location attached to a rendered rectangle. Rendering owns the geometry; application input routing consumes the semantic target rather than reconstructing item identity from coordinates.
