# Hunkle Domain Context

This glossary records product concepts whose names should stay stable across modules and UI text.

## Workspace

The directory Hunkle has opened. A workspace is either a Git repository root or a local workspace.

## Repository

A Git-backed workspace. Repository data includes the worktree, file inventory, history, graph, refs, and host capabilities.

## Local workspace

A directory opened without Git behavior. It supports file browsing, search, previews, and file operations.

## Worktree

The tracked and untracked changes shown in the left CHANGES pane. Staging actions operate on this view even while Graph is visible.

## Linked worktree

A Git checkout registered through `git worktree`. The Worktrees interaction lists linked worktrees from repositories Hunkle knows about and can open or safely remove them. This is distinct from the CHANGES-pane Worktree.

Git inventory is authoritative for whether a linked worktree exists and for its checkout state. Herdr may own an existing linked worktree and therefore determine how Hunkle removes it, but Herdr observations do not create catalog entries. Known repositories are discovery memory rather than authoritative topology.

## Linked worktree catalog

The application-owned catalog that reconciles Git inventory, known repository discovery, Herdr ownership, and the active workspace. Interactions consume its snapshot and ask it to plan destructive removal; they do not independently reconstruct ownership or safety policy.

## Files

The complete filesystem tree inside the workspace, including Git-ignored content but excluding Git's own metadata directory.

## Explorer

The `o` interaction for finding and opening another workspace. Explorer is not the repository browser. Confirming a file path opens the file's parent directory as a workspace and selects the file.

## Repository browser

The `b` interaction for branches, pull requests, and issues belonging to the active repository.

## Interaction

A focused user flow that owns its transient state and interprets input. An interaction may emit an application effect, such as opening a branch tip in Graph.

## Hit target

A semantic action or location attached to a rendered rectangle. Rendering owns the geometry; application input routing consumes the semantic target rather than reconstructing item identity from coordinates.
