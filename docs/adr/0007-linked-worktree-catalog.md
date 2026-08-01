# ADR 0007: Centralize Linked Worktree Authority

- Status: Accepted
- Date: 2026-08-01

## Context

Linked-worktree behavior was spread across `WorktreeManager`, `WorkspacePanel`, `App`, header pickers, and the Git adapter. Those callers independently combined known repository paths, Git inventory, Herdr ownership, active-workspace state, and removal safety. The removal dialog also cached whether removal should use Git or Herdr before the user confirmed it.

This made the Worktrees interaction a shallow module: removing it would leave linked-worktree discovery, naming, ownership reconciliation, and safety policy distributed across the application. It also tied background inventory freshness to whether the Worktrees overlay happened to be open.

## Decision

An application-owned `LinkedWorktreeCatalog` is the single authority for linked-worktree topology and removal planning.

- Git inventory is authoritative for existence, checkout state, primary status, locks, and prunability.
- Herdr is authoritative only for ownership and Herdr-mediated removal of a Git-inventoried worktree.
- Known repositories are discovery memory and never authoritative topology.
- The active workspace is application state used by the catalog's removal policy.
- Herdr-only paths do not become catalog entries.

When Herdr is disabled, removable worktrees use native Git removal. When Herdr ownership is verified, owned worktrees use Herdr and unowned worktrees use native Git. While enabled ownership is unverified, browsing and opening remain available, but destructive removal waits for verification.

`WorktreeManager` remains an Interaction. It owns filtering, selection, dialogs, and create/remove progress, consumes a catalog snapshot, and emits the semantic effect `Remove(path)`. `App` asks the current catalog for a removal plan only when applying that effect, so a dialog cannot execute stale routing information.

The catalog refreshes independently of overlay visibility, retains its previous snapshot while loading, and rejects stale generations. Failed discovery is retained as unavailable state unless repository absence is definitive.

## Consequences

- Linked-worktree authority and safety policy have one owner and a small snapshot/planning interface.
- Header pickers, labels, and the Worktrees interaction consume the same inventory.
- Herdr availability does not block non-destructive worktree use.
- Removal routing is recomputed at confirmation and cannot use stale ownership.
- Git parsing/removal and Herdr execution remain concrete adapters rather than being hidden behind a generic runtime interface.
- `WorkspacePanel` emits one owned observation containing candidates and ownership instead of three parallel values that callers must reconcile.

## Rejected Alternatives

- Keeping inventory in `WorktreeManager` would continue coupling application topology to one overlay's lifecycle.
- Treating Herdr snapshots as topology would display paths that Git does not recognize as linked worktrees.
- Allowing native removal while enabled Herdr ownership is unverified risks bypassing Herdr ownership during transient unavailability.
- Caching a native-or-Herdr route in the confirmation dialog allows authority to change before destructive execution.
- A generic worktree-provider or runtime interface would add mechanism without another topology implementation.
