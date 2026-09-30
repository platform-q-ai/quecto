//! Consolidated real-process and runtime integration target.
//!
//! One test binary instead of one per file: every `tests/*.rs` target links the
//! whole harness (plus tui and the dev-deps) separately, so a one-file edit
//! relinked 36 binaries. Each former target is a module here and keeps its
//! test names under its old name, so `--test integration uds_termination::`
//! still selects it.
//!
//! Modules here share one process, so none may mutate process-wide state
//! (`std::env::set_var`, `set_current_dir`); tests that depend on running
//! alone stay separate targets (guarded by `tests/architecture.rs`):
//! `selected_termination` sets process environment that every concurrently
//! spawned `quecto` child would inherit, and `parent_loss` kills a launcher
//! within ~100 ms of its child binding and, under the load of the other
//! modules, met the child before it was bound (it then exits at the 30 s bind
//! deadline instead of on connection loss, past the test's 20 s bound).

mod compiler_environment;
mod container_cache_wiring;
mod docker_create_admission_mask;
mod docker_create_image_contract;
mod docker_kill_script;
mod docker_runtime_answers;
mod fleet_teardown;
mod inference_admission_anthropic_malformed_terminal;
mod inference_admission_attempts;
mod inference_admission_broker;
mod inference_admission_characterization;
mod inference_admission_container;
mod inference_admission_error_compatibility;
mod inference_admission_feedback_transport;
mod inference_admission_http_fallback;
mod inference_admission_openai_retry_owner;
mod inference_admission_openai_sse;
mod inference_admission_processes;
mod inference_admission_redirects;
mod inference_admission_responses_root_error;
mod inference_admission_retry;
mod inference_admission_runtime;
mod inference_admission_runtime_oauth;
mod inference_admission_sse_observer;
mod inference_admission_transport;
mod issue_1193_completion;
mod legacy_session_startup;
mod repl_production;
mod shared_cache_security;
mod standard_shared_cache;
mod swarm_agent_loop;
mod swarm_board_admission_race;
mod swarm_board_claim_race;
mod swarm_board_diff_completion;
mod swarm_board_diff_control;
mod swarm_board_diff_files;
mod swarm_board_diff_loose;
mod swarm_board_diff_loose_completion;
mod swarm_board_diff_loose_files;
mod swarm_board_diff_loose_loss;
mod swarm_board_diff_loose_messages;
mod swarm_board_diff_loose_reads;
mod swarm_board_diff_loose_runs;
mod swarm_board_diff_loose_tasks;
mod swarm_board_diff_loss;
mod swarm_board_diff_loss_checks;
mod swarm_board_diff_membership;
mod swarm_board_diff_membership_loose;
mod swarm_board_diff_messages;
mod swarm_board_diff_reads;
mod swarm_board_diff_reads_calls;
mod swarm_board_diff_runs;
mod swarm_board_diff_submissions;
mod swarm_board_diff_tasks;
mod swarm_board_diff_usage;
mod swarm_board_diff_wakes;
mod swarm_board_diff_wire;
mod swarm_board_goldens;
mod swarm_board_legacy;
mod swarm_board_mixed;
mod swarm_board_mixed_harness;
mod swarm_board_notification_race;
mod swarm_board_py_json;
mod swarm_board_store;
mod swarm_product_contract;
mod test_fixture_bin;
mod uds_event_reader;
mod uds_termination;
