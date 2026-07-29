# ADR 0006: Open Workspaces Progressively

- Status: Accepted
- Date: 2026-07-29

## Context

Repository open loaded Git status, the complete file inventory, branch history, the all-refs Graph, and refs concurrently, but published nothing until every facet finished. Large ignored or generated trees therefore kept the application on "Opening workspace…" even though the Files tree only needs a root directory listing.

## Decision

Split workspace opening into two stages:

1. Bootstrap the workspace by canonicalizing its root, classifying it as Git or local, resolving the common Git directory when applicable, and preparing the lazy root Files tree.
2. Publish that bootstrap snapshot immediately, then use the existing full scoped refresh to load status, inventory, history, Graph, and refs in the background.

Bootstrap snapshots explicitly record that repository details are not ready. Empty facet values are not interpreted as authoritative until the full refresh succeeds. Files browsing and previews remain available during hydration; Git actions and inventory-backed search wait for readiness. A new workspace open may supersede background hydration, with the existing load generation rejecting its stale completion.

This supersedes ADR 0004's decision that repository open loads every facet before publication. Scoped refresh policy after initial hydration is unchanged.

## Consequences

- Workspace identity and root files appear without waiting for recursive inventory or history.
- The existing aggregate `RepositoryData`, refresh workers, selection restoration, and generation checks remain in use.
- Initial facets still become ready atomically, so one failed facet leaves all repository details unavailable until manual retry.
- The implementation does not add filesystem watchers, persistent indexes, worker cancellation, or independently streamed facet results.

## Rejected Alternatives

- Publishing five facet completions independently would reduce tail latency and isolate failures, but requires per-facet lifecycle, scheduling, and UI states that are not yet justified.
- Optimizing inventory alone would not remove history, Graph, and refs from the opening barrier.
