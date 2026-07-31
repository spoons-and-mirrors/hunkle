# ADR 0005: Separate Workspace Focus from Manager Selection

- Status: Accepted
- Date: 2026-07-21

## Context

The Workspace Manager displays both a navigation cursor and the active Herdr workspace. They look related but have different owners: the cursor is local interaction state, while active focus comes from the containing Hunkle process, Herdr snapshots, and asynchronous focus requests.

Representing those sources independently in `WorkspacePanel` allowed stale snapshots, request completions, and a hidden process's cursor to disagree during workspace transitions.

## Decision

`WorkspacePanel` retains row data and the local cursor. A dedicated `WorkspaceFocusState` owns the containing workspace ID, latest observed Herdr focus, pending focus request, and monotonic request sequence.

The displayed active workspace is derived in this order: pending request target, latest observed Herdr focus, then containing workspace. Only the current request completion may resolve or fail a transition. Snapshots update observed focus without overriding a pending target, and snapshots remove requests whose target no longer exists.

After a successful outbound focus command, the hidden Hunkle process resets its cursor to its containing workspace so its buffered UI is ready when that workspace is shown again. Rendering consumes a unified entry state containing both `active` and `selected` rather than reconstructing either value.

A workspace-row click changes the local cursor and immediately opens that repository in the current Hunkle. Hunkle records the repository shown before the first click as speculative restore state. `Enter` or a matching second click requests Herdr focus; after that request succeeds, the now-hidden source process restores its recorded repository. A failed focus request leaves the single-click navigation in place.

## Consequences

- Cursor navigation remains independent from Herdr focus.
- Stale snapshots and out-of-order completions cannot cause transition flicker.
- Each Hunkle process can pre-render its containing workspace correctly while hidden.
- Switching Herdr workspaces cannot replace a hidden Hunkle process's repository.
- Focus precedence and failure rollback have one owner and transition-focused tests.
- Speculative single-click repository opening and rollback, grouping, dragging, and agent focus remain outside workspace-focus state.

## Rejected Alternatives

- Treating the selected row as active would make navigation switch workspaces before confirmation.
- Trusting snapshot focus alone retains the polling delay during transitions.
- Updating styles independently in the renderer allows marker and background state to drift again.
- A generic application state-machine framework would add mechanism beyond this bounded interaction.
