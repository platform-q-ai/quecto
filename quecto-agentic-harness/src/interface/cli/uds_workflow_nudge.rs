//! Workflow auto-continue / completion nudge helpers for the UDS dispatch loop.
//!
//! Split out of `uds.rs` to keep that module within the source line cap. These
//! free functions read the bound workflow engine to decide whether — and with
//! what message — the agent should be nudged to continue at an idle boundary.

use super::uds::DispatchCtx;

/// The next workflow nudge message, if auto-continue or completion nudging is
/// enabled and the engine still has something to say.
pub(super) fn workflow_nudge_message(ctx: &DispatchCtx<'_>) -> Option<String> {
    let (Some(ws), Some(wc)) = (&ctx.workflow_state, &ctx.workflow_config) else {
        return None;
    };
    if !wc.auto_continue && !wc.completion_nudge {
        return None;
    }
    let Ok(engine) = ws.lock() else { return None };
    wc.auto_continue
        .then(|| engine.auto_continue_nudge())
        .flatten()
        .or_else(|| {
            wc.completion_nudge
                .then(|| engine.completion_nudge())
                .flatten()
        })
}

/// A serialized fingerprint of workflow progress, used to detect whether a
/// nudge actually advanced the workflow (so a stuck workflow isn't nudged
/// forever).
pub(super) fn workflow_progress_fingerprint(ctx: &DispatchCtx<'_>) -> Option<String> {
    let ws = ctx.workflow_state.as_ref()?;
    let engine = ws.lock().ok()?;
    let snapshot = engine.snapshot(true);
    serde_json::to_string(&snapshot).ok()
}
