# ADR 0011: Restore Active Norm Views Through Herdr

- Status: Accepted
- Date: 2026-08-22

## Context

Norm presence gives Hunkle stable agent identity and workspace metadata. Opening
only that workspace loses the live Norm TUI that the card represents. Inferring
a terminal from cwd, title, or process name is ambiguous, and attaching another
Norm client would take its controller lease.

Norm can publish the launch-time `HERDR_PANE_ID` for each TUI instance. Herdr
keeps that pane ID as a resolvable alias when the terminal moves. One Norm TUI
can contain multiple agent tabs, but presence identifies only its active tab;
there is no remote tab-activation command.

## Decision

Treat the optional Norm pane ID as an exact view-restoration hint, not as agent
topology. Associate it with a card only when that card's exact agent ID and
generation are the instance's active tab.

On card activation inside embedded Herdr:

1. Resolve the launch-time alias with the official `pane.current` API, always
   supplying `caller_pane_id`.
2. Submit the resolved pane, workspace, and tab to `HerdrSession`'s existing
   serialized layout-exchange operation.
3. Key saved layouts by the stable external pane alias and retain that ownership
   across Herdr agent snapshots.
4. Reopen the Norm agent's reported workspace only after layout exchange
   succeeds.

Do not move a pane for an inactive Norm tab, because that would display a
different agent. Open its workspace with a notice instead. Do the same when an
older running Norm daemon omits pane metadata, with a notice to restart Norm.
Standalone and background-attached Hunkle keep the workspace-only behavior.

Hunkle remains a passive Norm-presence consumer. It does not start Norm, attach
to the daemon as a controller, or shell out to Herdr outside `HerdrSession`.

## Consequences

- Clicking an active hosted Norm card restores the existing terminal, process,
  scrollback, and compact/full Norm UI beside Hunkle.
- Restoration uses released Herdr APIs and the established layout owner,
  including focus preservation, recovery, persistence, and pruning.
- Inactive tabs have an explicit workspace fallback until Norm offers a bounded,
  identity-checked tab-activation command.
- Norm must be restarted once after upgrading from a daemon that does not relay
  pane metadata.

## Rejected Alternatives

- Match terminals by cwd, title, or process name: ambiguous for multiple agents
  in one workspace.
- Launch or attach a new Norm client: changes controller ownership and does not
  restore the original TUI.
- Add separate layout workers or persistence for Norm: duplicates
  `HerdrSession` and risks conflicting operations.
- Modify Herdr: the released `pane.current` and layout APIs already provide the
  required behavior.
