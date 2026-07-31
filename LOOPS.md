# Hunkle Loops

## hunkle-improve

- Purpose: Repeatedly improve Hunkle's existing terminal UI and user experience through live, evidence-backed dogfooding.
- Trigger: Invoke manually after creating and switching to the branch that should receive the work.
- Saved: 2026-07-31

### Prompt

```text
Use the current Hunkle branch as the sandbox. Preserve pre-existing changes; do not create or switch branches, reset or clean, commit, push, merge, read tickets/, modify Herdr, or touch other repositories. Read AGENTS.md, CONTEXT.md, README.md, UI and interaction code, tests, and git state. Discover the Hunkle pane through Herdr; stop if it is unavailable or ambiguous. Use visible text and ANSI pane reads plus send-keys to exercise real flows. Repeat: choose one observed UI/UX friction, make the smallest Hunkle-only fix, relaunch if needed, and verify the same flow plus relevant surface tests. At the end run cargo fmt --check, cargo test --all-targets --all-features, cargo clippy --all-targets --all-features -- -D warnings, and cargo install --path . --force --locked. Stop on no evidence-backed improvement, a product or upstream decision, or failed verification. Leave a receipt and keep changes uncommitted.
```
