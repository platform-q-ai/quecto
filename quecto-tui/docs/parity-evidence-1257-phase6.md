# #1257 Phase 6 parity evidence

| Surface | Claim | Evidence | Verdict |
|---|---|---|---|
| Public crate shape | `lib.rs` exposes only feature modules; no `interface` | architecture `tui_lib_rs_exposes_only_architecture_layers` | PASS |
| File placement | All production `.rs` under approved top-level modules | architecture + BDD placement scenarios | PASS |
| Ratchets | Raw-JSON and wire-DTO inventories unchanged by move | architecture ratchet tests exact totals 55/121/122 | PASS |
| Unit behaviour | App/event/render paths unchanged | `cargo test -p quecto-tui --lib` 1668 passed | PASS |
| Visual/BDD | TUI headless BDD unchanged | 175 scenarios passed under `QUECTO_TAG=tui` | PASS |
| Architecture BDD | Final module set + purity rules executable | `QUECTO_TAG=tui` harness architecture feature | PASS |
