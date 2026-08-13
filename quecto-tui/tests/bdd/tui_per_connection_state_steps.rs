//! Step definitions for `tui_per_connection_state.feature` (#1463).
//!
//! Phase 2 of the multi-session TUI (epic #1467): connection-scoped state
//! moves off `App` into the per-tab `Connection` structures, and every minted
//! correlation id gains a connection namespace so a broadcast response can
//! never match a pending latch on another tab. The master tab is `TabId(0)`,
//! so its namespace prefix is `tab0:`.

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;

/// Namespace prefix minted correlation ids must carry for the master tab
/// (`TabId(0)`), pinned by #1463.
const MASTER_NAMESPACE: &str = "tab0:";

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("TUI harness").0
}

#[when("a resume response mints a solicited transcript fetch id")]
fn resume_response_mints_solicited_fetch_id(world: &mut TuiWorld) {
    let minted = {
        let h = harness(world);
        h.event(Event::Response {
            id: Some("resume".into()),
            command: "resume_session".into(),
            success: true,
            data: Some(serde_json::json!({ "session": "s-1463" })),
            error: None,
        });
        h.app_mut()
            .test_pending_resume_messages_id()
            .expect("resume_session response mints a pending solicited id")
            .to_string()
    };
    world.tui_minted_correlation_id = Some(minted);
}

#[then("the minted correlation id should begin with the master connection namespace")]
fn minted_id_begins_with_master_namespace(world: &mut TuiWorld) {
    let id = world
        .tui_minted_correlation_id
        .as_deref()
        .expect("a correlation id was minted");
    assert!(
        id.starts_with(MASTER_NAMESPACE),
        "minted correlation ids must be namespaced to their connection \
         (#1463): expected prefix {MASTER_NAMESPACE:?}, got {id:?}"
    );
}
