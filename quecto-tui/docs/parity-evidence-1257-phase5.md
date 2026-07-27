# Parity evidence — #1257 Phase 5

## Structural goals (review-time)

| Goal | Evidence | Verdict |
|---|---|---|
| Four top-level modules `sessions/`, `workflow/`, `inference/`, `workspace/` | `ls quecto-tui/src/{sessions,workflow,inference,workspace}/mod.rs`; architecture `tui_architecture_layers_exist`; BDD Phase 5 scenario | PASS |
| `infrastructure/` deleted | `! test -d quecto-tui/src/infrastructure`; architecture + BDD negative layer checks | PASS |
| `lib.rs` exact module set | `tui_lib_rs_exposes_only_architecture_layers` + BDD library-root step | PASS |
| App slices `#[path]`-mounted under `interface::app` | `shell/app.rs` mounts `../{sessions,workflow,inference,workspace}/…` | PASS |
| Architecture/BDD/doc lockstep | architecture 39/39; `QUECTO_TAG=tui` BDD architecture feature green; owner map exact match 95/95 | PASS |
| Feature/view raw-JSON seed lowered only by genuine mapper conversions | Pre-phase feature-view total 109 → post-phase 55 with extended scan roots; protocol total 69 → 121 (absorbed mapper sites); net feature-view burn-down 54 sites; combined total 176 remains below the historical ceiling 178 | PASS |

## Behavioural surfaces

| Surface | Evidence | Verdict |
|---|---|---|
| Sessions / workflow / inference / workspace characterization | `cargo test -p quecto-tui --lib` → **1667 passed** (includes moved suites + new protocol mapper tests) | PASS |
| Architecture ratchets | `tui_interface_raw_json_parsing_sites_do_not_grow` (55), `tui_protocol_raw_json_parsing_sites_do_not_grow` (121), combined raw-JSON inventory 176 ≤ historical ceiling 178, `tui_wire_dto_usage_does_not_grow` (122) | PASS |
| TUI architecture BDD | `QUECTO_TAG=tui cargo test -p quecto-agentic-harness --features test-support --test bdd` → Phase 5 scenarios green | PASS |
| Formatting / lint | `cargo fmt --all -- --check`; `cargo clippy -p quecto-tui --all-targets -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines`; architecture clippy `-D warnings` | PASS |

## Mapper conversion inventory (genuine burn-down)

| Call site | Before | After |
|---|---|---|
| `Footer::apply_get_state` | raw `.get("model"/"effort"/"maxContextTokens")` | `protocol::state_payloads::parse_get_state_footer` + `apply_get_state_fields` |
| `App::handle_get_state` | raw effortLevels / sessionKey / maxContextTokens | `parse_get_state` |
| `App::handle_set_effort_success` | raw `data.effort` | `parse_set_effort_level` |
| `App::handle_set_model_success` | raw `data.model` | `parse_set_model_id` |
| `App::handle_resume_success` | raw `data.session` | `parse_resume_session_name` |
| `App::sync_workflow_automation` | raw automation keys | `parse_workflow_automation` |
| `workflow_bar::parse_workflow_event` | full raw JSON walk | `parse_workflow_snapshot` → component DTO |
| `agents/app_subagent_stream` get_state/set_effort/set_model | hand-rolled field extract | shared `state_payloads` mappers (consolidation from review) |

## Characterization hash deltas

Frozen pre-move bodies were path-relocated with content preserved (tests moved with owners). Post-move hashes of characterization bodies (content-only) remain valid for behavioural parity; path strings inside test files were not behaviour assertions.

## Performance

| Code | Characteristic preserved |
|---|---|
| `list_workspace_files` | git-first + BTreeSet + `MAX_WORKSPACE_FILES=5000` + fs_walk skips |
| Git branch poll | 2s interval, 4KiB HEAD read, same strip/sanitize |
| Footer/get_state mappers | single-pass field extraction |
| Workflow snapshot parse | single pass over steps; same field fallbacks |

## Deletion ledger

| Deleted | Invariant | New location |
|---|---|---|
| `infrastructure/` | workspace filesystem adapter available | `workspace/workspace_files.rs` |
| `interface/app_sessions.rs` (path) | SessionsFlow defaults | `sessions/controller_sessions.rs` (#[path] remount) |
| `interface/app_workflow.rs` (path) | WorkflowFlow automation mirrors | `workflow/controller_workflow.rs` |
| `interface/app_{inference,models,effort}.rs` (paths) | model/effort flows | `inference/` |
| `interface/app_{workspace,git}.rs` (paths) | workspace flow + git branch | `workspace/` |
