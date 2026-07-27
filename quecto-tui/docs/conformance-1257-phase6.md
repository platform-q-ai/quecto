# #1257 Phase 6 conformance

## Scope

Final App composition + interface retirement:

- Move remaining `interface/app*.rs`, stdin buffer, and harness support into `shell/`
- Move presentation primitives (`ansi`, `theme`, `utils`, `kitty`, overlays) into `components/`
- Delete `interface/`
- Lockstep architecture/BDD/docs to the final feature-oriented module set
- Preserve #1220 ratchet inventories under the relocated scan roots

## Zero behaviour change

No command ordering, protocol handling, render semantics, or user-visible behaviour changes. Mechanical path/module ownership only.

## Guardrails updated

- `TUI_LIB_RS_MODULES` / `TUI_TOP_LEVEL_MODULES`: final nine feature modules (no `interface`)
- Feature/view ratchet roots: `components`, `shell`, `conversation`, `agents`, `sessions`, `workflow`, `inference`, `workspace`
- Exact totals preserved: feature/view raw-JSON `0`, protocol raw-JSON `86`, explicit wire-DTO references `16`, combined ≤ `178`
- `interface/` deletion asserted; shell owns `app`/`stdin_buffer`; components own `ansi`/`theme`
- Components must not import `protocol::client`; protocol must not import shell app/terminal widgets

## Parity evidence pointers

- `cargo test -p quecto-agentic-harness --test architecture` — 43 passed
- `cargo test -p quecto-tui --lib` — 1668 passed
- `QUECTO_TAG=tui cargo test -p quecto-agentic-harness --features test-support --test bdd` — architecture feature green
- `cd quecto-tui && QUECTO_TAG=tui cargo test --features test-harness --test bdd` — 28 features / 175 scenarios passed

## Final issue reconciliation

All #1257 acceptance criteria are represented by executable architecture/BDD guards.
Feature/view raw JSON is eliminated outside the issue-linked response dispatch seam;
explicit wire DTO references are confined to documented shell/runtime integration seams.
Controller extraction was optional in the approved Phase 6 plan; feature-owned App
extensions remain physically owned by their capability modules and shell only composes them.
