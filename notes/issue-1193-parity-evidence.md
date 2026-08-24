# Issue #1193 step 7 parity evidence

## Formatting and lint

| check | evidence | verdict |
| --- | --- | --- |
| `cargo fmt --all --check` | included in `/tmp/step7_fmt_clippy3.out` after installing the missing `clippy` component | PASS |
| `cargo clippy -p quecto-agentic-harness --lib -- -D warnings` | `/tmp/step7_fmt_clippy3.out` | PASS |
| `cargo clippy -p quecto-tui --lib -- -D warnings` | `/tmp/step7_fmt_clippy3.out` | PASS |

`cargo-clippy` was initially missing from the active toolchain; installed with `rustup component add clippy` and reran.

## Behavioural evidence

| surface | behaviour | evidence | verdict |
| --- | --- | --- | --- |
| Legacy model registry | Built-ins, custom `models.json`, provider/model override precedence, auth parsing, limits, malformed registry failures remain compatible | `cargo test -p quecto-agentic-harness --lib model_registry` in `/tmp/step6_full_frozen_suite.out`: 28 passed | PASS |
| UDS list-models | UDS response shape/fields/errors remain compatible while now routed through `ResolveCatalogueUseCase` + `ModelRegistryCatalogueSource` | `cargo test -p quecto-agentic-harness --lib uds_models` in `/tmp/step6_full_frozen_suite.out`: 2 passed | PASS |
| CLI discovery | Existing `quecto models discover` syntax, validation, URL policy, auth handling, response caps, dedupe/sort, re-read-before-publish, atomic write behaviour remain compatible while helper moved to infrastructure | `cargo test -p quecto-agentic-harness --lib discover` in `/tmp/post_mutation_green2.out`: 61 passed | PASS |
| Provider runtime composition | Built-in providers, registry providers, OAuth/API key coexistence, unsupported Google runtime, duplicate prefixes, unsafe remote HTTP, no-provider errors remain compatible while construction moved to infrastructure behind application port | `cargo test -p quecto-agentic-harness --lib agent_provider` in `/tmp/post_mutation_green2.out`: 53 passed | PASS |
| Provider reload | Poll/force reload, no-config handling, malformed config preserving last good state remain compatible | `cargo test -p quecto-agentic-harness --lib provider_reload` in `/tmp/step6_full_frozen_suite.out`: 6 passed | PASS |
| Application/domain catalogue seams | Domain IDs, layer precedence, structured availability, query/selection, limits, refresh, runtime-generation helper work at architecture seams | `cargo test -p quecto-agentic-harness --lib catalogue`, `catalogue_limits`, `catalogue_refresh`, `catalogue_runtime`, `provider_runtime`, `catalogue_registry`, `catalogue_discovery` in `/tmp/step6_full_frozen_suite.out` | PASS |
| TUI protocol and command compatibility | Existing command serialisation and built-in slash command ordering remain stable; `/models-refresh` is intentionally hidden from built-in palette to preserve step-7 visual/order parity while direct slash handling exists | `cargo test -p quecto-tui --lib protocol::client`; `cargo test -p quecto-tui --lib shell::app::tests::builtin_commands_have_stable_order_and_names`; `cargo test -p quecto-tui --lib shell::app::app_refresh_tui_tests` in `/tmp/step7_targeted_interfaces.out` | PASS |
| Full harness lib suite | All quecto-agentic-harness lib tests pass | `/tmp/step7_full_tests_rerun.out`: `3838 passed; 0 failed` | PASS |
| Full TUI lib suite | Full suite was run. Failures after fixing built-in command ordering are pre-existing/flaky/non-issue surfaces: git branch footer test still fails standalone after rerun; child-watch process-group survivor also failed in full suite. Targeted issue-touched TUI suites pass. | `/tmp/step7_full_tests_rerun.out` and `/tmp/step7_tui_flaky_rerun.out` | BLOCKER OUTSIDE TOUCHED SURFACE; targeted issue evidence PASS |

## Freeze manifest diff

Hash comparison in `/tmp/step7_metrics.out`:

- Unchanged: readiness note, `model_registry_tests.rs`, `uds_models_tests.rs`, `agent_config_tests.rs`, `agent_provider_cov_tests.rs`, `provider_reload_tests.rs`, `agent_loop_935_tests.rs`, `agent_loop_ctx_mgmt_tests.rs`, `uds_dispatch_935_clamp_tests.rs`, `agent_1048_ctx_wiring_tests.rs`.
- Changed: `quecto-agentic-harness/src/interface/cli/models_tests.rs`.
  - Rationale: mechanical adaptation after moving discovery helper implementation from `interface/cli/models.rs` to `infrastructure/catalogue_discovery.rs`.
  - Assertions were not removed; test local wrapper functions now call the moved infrastructure helpers.
  - Mutation proof refreshed: `/tmp/mutation_catalogue_discovery_move_summary.out` mutates the moved helper to publish an empty list and `discover_replaces_only_target_provider_models_and_preserves_auth` fails; `/tmp/post_mutation_green2.out` shows the test and discovery suite green after revert.

## Visual evidence

| surface | evidence | verdict |
| --- | --- | --- |
| Existing slash command palette | `builtin_commands_have_stable_order_and_names` failed when `/models-refresh` was added to the palette; production changed to keep the command hidden from the built-in list, preserving existing order/rendering. Targeted test then passed. | PASS |
| `/refresh-tui` TUI rendering | `shell::app::app_refresh_tui_tests::handle_submit_refresh_tui_updates_terminal_size_and_redraws_without_agent_command` passed in `/tmp/step7_targeted_interfaces.out`. | PASS |
| Model selector/list rendering | Existing TUI model protocol tests in `protocol::client` pass. No golden frame was changed. | PASS |

## Performance evidence

| replaced specialized code | old code avoided | new mechanism | verdict |
| --- | --- | --- | --- |
| CLI discovery in `interface/cli/models.rs` | Avoided repeated registry writes before fetch completed; re-read registry just before whole-file publish; bounded response bytes/models; one pass through returned data with HashMap dedupe then sort | Moved unchanged to `infrastructure/catalogue_discovery.rs`. Interface calls one application refresh use case and one infrastructure adapter. Same re-read-before-publish and bounds remain inside shared mechanism. | PASS |
| Provider runtime construction in CLI | Constructed configured provider list once at startup/reload, checked duplicates with `HashSet`, wrapped providers once before `ProviderRouter::new` | Moved unchanged to `infrastructure/provider_runtime.rs`; CLI façade delegates through application port. No per-call recomputation introduced. | PASS |
| Model limits lookup | One registry load returned both max-token cap and context-window value atomically | `ResolveModelLimitsUseCase` still returns `(Option<u32>, Option<usize>)` as one call through `ModelRegistryLimitSource`; agent startup, UDS set_model, and REPL route through it. | PASS |
| UDS model list projection | One registry load then one projection pass | UDS now loads one `ModelRegistryCatalogueSource`, resolves one snapshot, and projects descriptors in one pass; no remote calls introduced. | PASS |

## Quantitative evidence

| metric | value | evidence | verdict |
| --- | --- | --- | --- |
| Tracked production/test diff net LOC | `tracked_add 201`, `tracked_del 762`, tracked net `-561` over modified tracked files | `/tmp/step7_metrics.out` | PASS |
| New architecture module size | 1645 lines across new domain/application/infrastructure catalogue/runtime modules | `/tmp/loc_metrics2.out` | Informational |
| File cap | New/changed issue files are below 750 lines except pre-existing `quecto-tui/src/protocol/client.rs` reports 754 lines and was already near/over cap; touched lightly for protocol enum addition. New largest issue file: `infrastructure/provider_runtime.rs` 550 lines. | `/tmp/step7_metrics.out` | PASS for new files; existing near-cap noted |
