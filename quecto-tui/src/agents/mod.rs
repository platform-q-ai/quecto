//! Agents ownership for `quecto-tui` (#1222 / #1257 Phase 4).
//!
//! Owns subagent roster, lifecycle, feeds, ledger sync, focus, retention, and
//! view projection. App slices remain mounted inside `shell::app` until
//! later controller-extraction phases.

pub(crate) mod feed;
pub(crate) mod focus;
pub(crate) mod ledger;
pub(crate) mod roster;
pub(crate) mod runtime;
pub(crate) mod view;

/// Whether a tool call renders without its own tool box. The decision depends
/// only on the tool name, so pure policy callers never parse arguments.
pub(crate) fn suppress_tool_box(tool_name: &str) -> bool {
    // #871: every model-issued `agent_cmd` invocation renders as a normal tool
    // call, including the control/destructive commands (`prompt`/`steer`/
    // `abort`/`kill`) that used to be hidden. Hiding them left the transcript
    // incomplete and made it hard to see why a sub-agent stopped. Only `spawn`
    // stays suppressed — the sub-agent status bar/panel shows it instead. The
    // TUI's OWN internal `get_state`/stats polling flows through Response events
    // (`app_response.rs`), not this tool path, so it remains box-free regardless.
    tool_name == "spawn"
}
