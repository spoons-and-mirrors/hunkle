# ADR 0001: Introduce Semantic Hit Targets Incrementally

- Status: Accepted
- Date: 2026-07-20

## Context

Hunkle historically stored rendered rectangles as individual fields on the
global `Regions` structure. Application mouse handlers then reconstructed
meaning from those rectangles, list offsets, and row heights. Variable-height
and filtered lists exposed this cost clearly: application code needed to know
presentation details to identify the item under the pointer.

## Decision

Rendering may register semantic hit targets alongside their rectangles. Input
routing asks which target occupies a point and does not infer item identity from
presentation geometry.

Renderers register targets for overlays, lists, controls, and exact visible
items. Global application code applies the semantic action and does not know row
heights or list geometry.

Other interactions will migrate only when changed or when the pattern removes
meaningful geometry coupling. This is not a requirement to rewrite all rectangle
handling immediately.

## Consequences

- Variable-height rows no longer leak into application input code.
- Global `Regions` loses interaction-specific rectangle fields in favor of one
  semantic collection.
- Rendering still owns terminal geometry, while application routing still owns
  cross-domain effects.
- During incremental migration, semantic targets and legacy rectangle fields
  coexist.

## Rejected Alternatives

- Splitting every modal into a new file without changing its interface would
  create shallow modules and preserve the coupling.
- Migrating every interaction in one change would increase regression risk
  without validating the seam first.
- Keeping row-height arithmetic in `App` would continue coupling application
  behavior to presentation details.
