# Ticket 001: Branch Review Mode

**Status:** Ready for implementation

**Blocked by:** None - can start immediately.

## What to Build

Add a persistent Branch Review mode that presents the current branch as a local pull request. It compares committed branch work against the branch's merge base, rather than presenting the worktree, and brings the commit range, cumulative diff, changed files, repository check status, and a lightweight review checklist into one navigable surface.

The completed mode should let a user answer: what commits will this branch contribute, what is the complete resulting patch, have I inspected it, and did my chosen verification command pass?

## Product Decisions

- Review only committed branch history. Uncommitted and staged worktree changes are excluded from the comparison and surfaced through a clear warning with a route back to Changes.
- Default the base to the remote default branch when it can be resolved. Let the user choose another local or remote branch, and remember the choice for the current repository during the session.
- Define the review range from the selected base/head merge base through `HEAD`. The cumulative patch is the tree difference between that merge base and `HEAD`.
- Show an explicit unsupported or empty state for detached HEAD, unborn branches, missing objects, shallow history that cannot produce a merge base, and branches with no commits beyond the merge base.
- Treat repository checks as local executable-plus-arguments commands, run without a shell from the repository root. A review check may be absent; its state is Not configured, Running, Passed, Failed, or Stale.
- A check result belongs to the exact repository, merge-base OID, and head OID. Moving either revision immediately marks the previous result stale.
- Keep checklist state local to the current Hunkle session and exact review identity. It must not be committed into the repository.
- Preserve Hunkle's architecture boundaries: Git comparison and parsing belong to the Git layer, scheduling and stale-result rejection belong to the repository session, interaction policy owns review state, and rendering owns geometry and semantic hit targets.

## Required Experience

- Add an obvious way to enter and leave Branch Review without losing the user's prior Changes or Graph selection.
- Present the selected base, merge-base identity, current head, ahead commit count, aggregate file count, and added/deleted line totals.
- List every commit in the review range with its subject, author, date, and short OID. Selecting a commit shows that commit's patch while preserving access to the cumulative branch patch.
- List changed files with status and line totals. Handle additions, modifications, deletions, renames, and binary files without silently reporting misleading zero-line changes.
- Render the cumulative diff through the existing preview behavior: syntax coloring, wrapping, scrolling, large-patch windowing, and source-aware presentation.
- Allow configuring and running one review check command. Capture its output and exit status without blocking the TUI, and expose the captured output from the review surface.
- Include a checklist with at least Commits reviewed, Cumulative diff reviewed, Changed files reviewed, and Checks acceptable. The user can toggle items manually; Hunkle may visually assist based on inspected content but must not claim that merely selecting one row constitutes a completed review.
- Refresh or invalidate review data when HEAD, refs, the selected base, or relevant remote refs change. Background results from an old repository, generation, base, or head must never overwrite the current review.

## Safety and Scope

- Branch Review is read-only apart from running the configured check command and changing session-local checklist state.
- Do not include staged or unstaged work in the cumulative branch diff.
- Do not require GitHub or an open pull request. A future `gh` integration may enrich the mode later but is not the source of truth for this ticket.
- Do not overload the existing single-commit preview cache with range results keyed only by commit OID; review results need an identity that includes repository, base, merge base, and head.
- New background work must declare its compatibility with current repository operations and its refresh/invalidation policy, following the operation-state and scoped-refresh ADRs.

## Acceptance Criteria

- [ ] On a normal feature branch, Hunkle resolves or asks for a base and shows commits reachable from `HEAD` but not the merge base.
- [ ] The cumulative changed-file summary and patch match Git's merge-base-to-HEAD tree comparison.
- [ ] Commit inspection and cumulative inspection are both available without leaving Branch Review.
- [ ] Added, modified, deleted, renamed, and binary files have truthful status and summary presentation.
- [ ] Staged and unstaged worktree changes are excluded and produce a visible warning when present.
- [ ] A configured local check runs asynchronously, captures output, and reports Running, Passed, Failed, or Stale against the exact reviewed revisions.
- [ ] The review checklist can be completed manually and resets or switches state when the review identity changes.
- [ ] Detached, unborn, shallow, missing-base, no-change, loading, stale, and command-failure states have useful messages and do not crash.
- [ ] Keyboard and mouse interactions use semantic actions and remain usable on narrow terminals.
- [ ] Real-Git tests cover linear history, merge-base selection, merge commits in the range, renamed and binary files, a moving HEAD, check success/failure/staleness, and repositories with no valid comparison.
- [ ] Ratatui surface tests cover entry/exit, base selection, commit and cumulative patch navigation, changed files, checklist interaction, check output, and important empty/error states.

## Not in This Ticket

- Publishing or updating pull requests.
- Remote CI-provider integrations.
- Reviewing uncommitted work as part of the branch comparison.
- Comments, annotations, or persisted team review data.
