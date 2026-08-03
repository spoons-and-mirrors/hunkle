# Performance Plan

Improve measured latency and background efficiency without reducing refresh
correctness, rendering fidelity, history limits, syntax support, or Herdr
capabilities. Agent transcript presentation is intentionally out of scope while
its design is still changing.

## Steps

- [x] Move Herdr timing-index persistence off the UI thread. Keep timing state
  immediately current in memory, serialize writes through one coalescing worker,
  retain cross-process merge and clear-watermark behavior, and report failures
  asynchronously.
- [x] Adapt subprocess lifecycle polling to favor fast command completion and
  back off for long-running commands. Keep bounded concurrent stdout/stderr
  draining, stdin support, process-tree termination, and the post-termination
  deadline for inherited pipes.
- [x] Add an allocation-free word-wrap height path with exactly the same line
  breaking and Unicode/tab-width behavior as cursor-aware wrapping.
- [ ] Batch syntax-highlighted text by styled run rather than allocating one
  `String` per grapheme, while preserving tab expansion and Unicode widths.
- [ ] Reuse parsed worktree status from the adaptive status check in the
  resulting refresh, avoiding an immediate duplicate `git status` without
  weakening refresh scope selection or stale-result protection. Deduplicate
  repeated agent and linked-worktree line-count loads within each batch.

## Verification

Each step receives focused regression coverage and a dedicated commit. After
all steps, run formatting, the full test suite, installation from the locked
checkout, and restart any open Hunkle processes in place.
