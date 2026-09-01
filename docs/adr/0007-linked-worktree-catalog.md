# ADR 0007: Centralize Linked Worktree Authority

- Status: Accepted
- Date: 2026-08-01

## Context

Linked-worktree behavior was spread across `App`, header pickers, and the Git adapter. Those callers independently combined known repository paths and Git inventory.

This tied background inventory freshness to whichever interaction happened to request it and duplicated naming and destination metadata.

## Decision

An application-owned `LinkedWorktreeCatalog` is the single authority for linked-worktree topology and discovery metadata.

- Git inventory is authoritative for existence, checkout state, primary status, locks, and prunability.
- Known repositories are discovery memory and never authoritative topology.

The catalog refreshes independently of overlay visibility, retains its previous snapshot while loading, and rejects stale generations. Failed discovery is retained as unavailable state unless repository absence is definitive.

## Consequences

- Linked-worktree topology has one owner and a small snapshot interface.
- Header pickers and linked-worktree labels consume the same inventory.
- Git parsing remains a concrete adapter rather than being hidden behind a generic runtime interface.

## Rejected Alternatives

- Treating remembered repository paths as topology would display paths that Git does not recognize as linked worktrees.
- A generic worktree-provider or runtime interface would add mechanism without another topology implementation.
