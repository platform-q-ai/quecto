//! Agents presentation policy extracted from the legacy `App` slice (#1222).
//!
//! Runtime glue still lives beside `App`; these modules own the pure roster,
//! feed-synchronization, ledger, and focus state used by that glue.

pub(crate) mod feed;
pub(crate) mod focus;
pub(crate) mod ledger;
pub(crate) mod roster;
pub(crate) mod ui;

pub(crate) fn suppress_tool_box(tool_name: &str, _args: &serde_json::Value) -> bool {
    // #871: every model-issued `agent_cmd` invocation renders as a normal tool
    // call, including the control/destructive commands (`prompt`/`steer`/
    // `abort`/`kill`) that used to be hidden. Hiding them left the transcript
    // incomplete and made it hard to see why a sub-agent stopped. Only `spawn`
    // stays suppressed — the sub-agent status bar/panel shows it instead. The
    // TUI's OWN internal `get_state`/stats polling flows through Response events
    // (`app_response.rs`), not this tool path, so it remains box-free regardless.
    tool_name == "spawn"
}
