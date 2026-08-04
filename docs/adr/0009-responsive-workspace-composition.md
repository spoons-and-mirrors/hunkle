# ADR 0009: Separate Workspace Navigation from Responsive Composition

- Status: Accepted
- Date: 2026-08-04

## Context

Hunkle must provide useful narrow-screen workflows rather than only compressing its desktop columns. Narrow layouts already drill from lists into diffs and agent transcripts, but layout and navigation policy had accumulated as independent width checks and boolean detail flags across rendering, keyboard handling, and pointer handling.

The application can also resize while running. Desktop and mobile are therefore not separate platforms with separate feature state; they are different compositions of the same active workspace.

## Decision

Use one responsive workspace shell over shared application and feature state.

`LayoutProfile` is the authoritative classification of the rendered viewport. It initially selects either a single surface or columns. Additional compositions, such as rows, will be added only for a concrete workflow with usable panel dimensions.

`WorkspaceNavigation` owns the current content route, Search return route, and Agents selection independently of composition. Content and Agents retain separate master/detail surfaces because columns can display them together. A single composition chooses the primary surface, while a multi-surface composition may render both. Worktree and Files remain hierarchical navigation owned by Changes because that state also restores preview provenance and selection.

Feature renderers receive the pane or surface they are rendering explicitly. They must not temporarily mutate application navigation state to render another surface.

Purely visual adaptations may remain local to a component and respond to its allocated area. Navigation transitions, Back behavior, workspace composition, and gesture destinations belong to shared application or shell policy.

## Consequences

- Narrow and wide layouts share repository, selection, loading, preview, and interaction state.
- Resizing changes composition without creating a second application lifecycle.
- New responsive layouts have an explicit extension point instead of adding another global width predicate.
- Application actions can reason about the selected master/detail surface without consulting renderer geometry.
- Existing feature renderers can migrate toward explicit master and detail surfaces incrementally.
- Semantic hit and scroll targets are the boundary for pointer actions and gestures rather than a separate mobile input stack.

## Rejected Alternatives

- Separate desktop and mobile UI trees would duplicate feature behavior and make live resizing a state-transfer problem.
- Continuing to add unrelated narrow-layout booleans would permit invalid combinations and distribute Back behavior across features.
- A generic event bus or widget framework would add indirection without improving ownership of layout or navigation policy.
- Adding a row composition before defining a useful vertical-split workflow would create speculative layout machinery.
