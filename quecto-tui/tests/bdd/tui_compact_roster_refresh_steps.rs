//! Step definitions for `tui_compact_roster_refresh.feature`.
//!
//! Compact `get_subagents` payloads (`agentId`/`status`/`environmentRef`,
//! optional `unchanged`) must keep already-visible left-panel rows. These
//! steps drive the REAL `get_subagents` response path through the headless
//! harness. Panel assertions reuse the environment-grouping steps.

use super::*;

fn compact_get_subagents_response_line(agents: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": {
            "subagents": agents,
            "sequence": 3,
        },
    })
    .to_string()
}

#[when(
    expr = "a compact get_subagents roster refresh reports {string} as running in environment {string}"
)]
fn when_compact_refresh(world: &mut TuiWorld, id: String, env_ref: String) {
    let line = compact_get_subagents_response_line(vec![serde_json::json!({
        "agentId": id,
        "status": "running",
        "environmentRef": env_ref,
    })]);
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .event_line(&line);
}

#[when(expr = "an unchanged compact get_subagents roster refresh arrives")]
fn when_unchanged_compact_refresh(world: &mut TuiWorld) {
    let line = serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": {
            "subagents": [],
            "sequence": 4,
            "unchanged": true,
        },
    })
    .to_string();
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .event_line(&line);
}
