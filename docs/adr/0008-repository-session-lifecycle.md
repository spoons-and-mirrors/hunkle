# ADR 0008: Let RepositorySession Own the Repository Lifecycle

- Status: Accepted
- Date: 2026-08-01

## Context

`RepositorySession` already owned repository data, operation compatibility, worker channels, generations, status baselines, and stale-result rejection. However, `App` still interpreted completion invalidations, queued and unioned refresh scopes, started hydration after a bootstrap open, retried interrupted hydration after a failed open, and drained refreshes after reload completion.

This split ownership made the lifecycle shallow at both seams. Correctness depended on `App::poll_worker` reproducing policy from ADRs 0002, 0004, and 0006 while also updating selections, panes, caches, and notices.

## Decision

`RepositorySession` owns repository lifecycle policy end to end:

- A successful bootstrap open automatically starts an `ALL` hydration refresh.
- A failed open resumes hydration when the retained snapshot is not ready.
- Refresh requests made during an active reload are queued and unioned without prematurely widening their scopes.
- Queued scopes drain after load completion and widen to `ALL` only when the current snapshot is not ready.
- Worker invalidations and external worktree changes request refreshes inside the session.
- Load generations, repository generations, status baselines, and operation state continue to reject stale results independently.

`App` submits refresh intents and consumes whether a refresh started or queued. It retains application concerns: preserving selections, updating panes and caches, displaying notices, and enforcing editor, draft, Herdr, and navigation guards.

Repository execution remains on concrete worker channels. `RepositoryData` remains an aggregate snapshot, and initial hydration remains atomic.

## Consequences

- Refresh scheduling has one owner and one queue.
- Progressive opening no longer depends on `App` noticing a bootstrap completion and starting hydration correctly.
- Failed opens cannot strand a retained bootstrap snapshot without hydration.
- Operation invalidation policy is applied before completion crosses the application seam.
- Session lifecycle tests can exercise queueing, recovery, and hydration deterministically without relying on transient rendered states.
- UI restoration remains outside the session, avoiding repository-policy dependencies on panes and selections.

## Rejected Alternatives

- Keeping a second refresh queue in `App` would preserve split ownership and allow the two schedulers to drift.
- A generic event bus, runtime, or worker interface would add mechanism without deepening repository policy.
- Moving UI selection and notice handling into `RepositorySession` would mix application presentation with repository lifecycle.
- Publishing hydration facets independently would change the atomic readiness contract established by ADR 0006.
