//! Step definitions for `tui_cold_start_deadline.feature` (#808).
//!
//! These steps drive the production message/status builders in quecto-tui
//! (`agent_socket_timeout_message`, `agent_starting_status`) and assert on their
//! returned strings — behaviour, not source text. The 30s deadline value, the
//! message composition through `format_agent_startup_failure`, the run-tui.sh
//! pre-warm, and the README docs are verified by focused unit/repo-lint tests
//! (quecto-tui/src/interface/cli.rs and tests/repo_docs.rs), not here.

use crate::QuectoWorld;
use cucumber::{then, when};

#[when("the agent does not announce its socket before the deadline")]
fn when_agent_times_out(world: &mut QuectoWorld) {
    world.stdout = quecto_tui::interface::cli::agent_socket_timeout_message();
}

#[then("the timeout message names the cold-binary first-run cause")]
fn then_message_names_cause(world: &mut QuectoWorld) {
    let lower = world.stdout.to_lowercase();
    assert!(
        lower.contains("cold") || lower.contains("first run") || lower.contains("first launch"),
        "the timeout message must name the cold-binary / first-run cause: {:?}",
        world.stdout
    );
}

#[then(regex = r#"^the timeout message suggests running "([^"]+)" to warm the binary$"#)]
fn then_message_suggests_warm(world: &mut QuectoWorld, cmd: String) {
    assert!(
        world.stdout.contains(&cmd),
        "the timeout message must suggest running `{cmd}` to warm the binary: {:?}",
        world.stdout
    );
}

#[then("the timeout message offers to retry")]
fn then_message_offers_retry(world: &mut QuectoWorld) {
    let lower = world.stdout.to_lowercase();
    assert!(
        lower.contains("retry") || lower.contains("try again"),
        "the timeout message must offer to retry: {:?}",
        world.stdout
    );
}

#[when("the TUI is waiting for the agent socket path")]
fn when_tui_waiting(world: &mut QuectoWorld) {
    world.stdout = quecto_tui::interface::cli::agent_starting_status().to_string();
}

#[then(regex = r#"^the TUI surfaces a "([^"]+)" status indicator$"#)]
fn then_status_indicator(world: &mut QuectoWorld, text: String) {
    assert!(
        world.stdout.to_lowercase().contains(&text.to_lowercase()),
        "the TUI must surface a {text:?} status while waiting for the agent socket: {:?}",
        world.stdout
    );
}
