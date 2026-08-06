# ADR 0010: Separate Local and Project Scheduled Task Sources

- Status: Accepted
- Date: 2026-08-06

## Context

Scheduled task definitions were stored as global Markdown files and mirrored into
SQLite. Repository-local task files were moved into that global directory, so file
ownership, database ownership, and run state could drift. It also prevented a
repository from safely declaring a reviewable task without surrendering its source
file to Hunkle.

Repository task prompts can change when a branch is updated. Automatically running
new source content would allow a checkout change to modify unattended behavior
without local approval. Linked worktrees can also expose the same repository task
from multiple paths and must not create duplicate schedules.

## Decision

`SchedulerService` owns scheduled task definitions, activation, discovery, approval,
execution state, and run history.

- Tasks created in Hunkle are local tasks whose definitions live in the scheduler
  database.
- Repositories may declare project tasks as direct `.md` files under
  `.agents/scheduled/`. Those files remain repository-owned and are never rewritten
  or moved by Hunkle.
- Project files own their ID, title, description, prompt, model, and suggested
  frequency. The database owns enabled state, approved source content, execution
  destination, next run, Discord webhook selection, and history.
- A discovered project task starts disabled. Enabling it records the exact approved
  source bytes. Changed or missing source disables the task and requires explicit
  approval before another run.
- Execution rereads project source and compares it with the approved bytes, so a
  task cannot run stale or newly changed content between repository refreshes.
- Project identity is the repository common directory plus project task ID. The
  first discovered worktree remains its execution destination; another linked
  worktree does not duplicate or silently retarget it.
- `App` requests discovery for the active repository after workspace open, relevant
  repository refreshes, and scheduler opening. It does not scan unrelated filesystem
  roots or create another repository lifecycle.
- Existing global and repository-local legacy task files are imported once as local
  database tasks without deleting their source files.

## Consequences

- Hunkle provides a complete editor for local tasks while project tasks remain
  reviewable and portable through Git.
- Pulling or switching to changed project-task content cannot silently alter an
  enabled schedule.
- Task history and local delivery configuration survive project source removal.
- Discovery is bounded to the active workspace and uses guarded workspace reads.

## Rejected Alternatives

- Mirroring every definition between Markdown and SQLite creates two writable
  authorities and ambiguous conflict resolution.
- Storing all task definitions only in repository files prevents private local tasks
  and makes UI-created tasks awkward to own.
- Automatically enabling repository tasks or accepting changed prompts without
  approval lets checked-in content schedule unattended work.
- Scanning every known repository continuously duplicates repository lifecycle and
  linked-worktree discovery responsibilities.
