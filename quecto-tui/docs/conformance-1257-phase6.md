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
- Exact totals at Phase 6 head: feature/view raw-JSON `0` (only `shell/app_response.rs` allowlisted as the #1220 response dispatch seam), protocol raw-JSON `127` (seed `127`; `protocol/client.rs` allowlisted as the wire seam), wire-DTO usage `98` (seed `98`; no whole-file exemptions — seam files stay measured), combined raw-JSON `127` ≤ historical ceiling `178`
- `interface/` deletion asserted; shell owns `app`/`stdin_buffer`; components own `ansi`/`theme`
- Components must not import `protocol::client`; protocol must not import shell app/terminal widgets

## Parity evidence pointers

- `cargo test -p quecto-agentic-harness --test architecture` — TUI architecture suite green (20 `tui_*` tests)
- `cargo test -p quecto-tui --lib` — 1668 passed
- `QUECTO_TAG=tui cargo test -p quecto-agentic-harness --features test-support --test bdd` — architecture feature green
- `cd quecto-tui && QUECTO_TAG=tui cargo test --features test-harness --test bdd` — 28 features / 175 scenarios passed

## Final issue reconciliation

All #1257 acceptance criteria are represented by executable architecture/BDD guards.

- Feature/view raw JSON is eliminated outside the issue-linked `shell/app_response.rs` response dispatch seam.
- Wire DTO usage is decrease-only at `98` and concentrated in documented shell/runtime and agents direct-feed integration paths (`shell/app*.rs`, `shell/cli.rs`, `shell/tui_harness*.rs`, `agents/{view,runtime,controller_*}.rs`); the empty wire-DTO allowlist keeps every seam file measured so growth inside a seam still fails the ratchet.
- Feature policy lives under feature-owned `controller_*.rs` modules; `shell::app` remains the composition root and path-composes those controllers as `App` extensions (full standalone controller types were optional in the approved Phase 6 plan and are not required by AC).
