# #1257 Phase 6 deletion ledger

| Deleted/replaced | Invariant | New owner |
|---|---|---|
| `interface/` directory | No production TUI code outside feature modules | deleted after moves |
| `interface/app*.rs` | App composition, event loop, routing | `shell/app*.rs` |
| `interface/stdin_buffer.rs` | Stdin buffering policy | `shell/stdin_buffer.rs` |
| `interface/tui_harness*.rs` | Headless harness | `shell/tui_harness*.rs` |
| `interface/{ansi,theme,utils,kitty,overlay,select_overlay}.rs` | Presentation primitives | `components/*` |
| `interface/mod.rs` | Interim compatibility export | removed; `lib.rs` exports feature modules only |
| External `quecto_tui::interface::*` paths | Test/harness imports | `quecto_tui::shell::*` / `quecto_tui::components::*` |
| Interim compatibility map in architecture doc | Readers must not implement against interim buckets | Final layout (Phase 6) section |
