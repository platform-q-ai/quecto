//! Workflow auto-continue / completion nudge helpers for the UDS dispatch loop.
//!
//! Split out of `uds.rs` to keep that module within the source line cap. These
//! free functions read the bound workflow engine to decide whether — and with
//! what message — the agent should be nudged to continue at an idle boundary.

use super::uds::DispatchCtx;

/// A nudge to inject at an idle boundary, tagged with the automation path
/// that produced it: the auto-continue path participates in the no-progress
/// tolerance loop (corrective retries), the completion path is single-shot.
pub(super) enum WorkflowNudge {
    AutoContinue(String),
    Completion(String),
}

impl WorkflowNudge {
    pub(super) fn is_auto_continue(&self) -> bool {
        matches!(self, WorkflowNudge::AutoContinue(_))
    }

    pub(super) fn into_message(self) -> String {
        match self {
            WorkflowNudge::AutoContinue(message) | WorkflowNudge::Completion(message) => message,
        }
    }
}

/// The next workflow nudge, if auto-continue or completion nudging is
/// enabled and the engine still has something to say.
pub(super) fn workflow_nudge_message(ctx: &DispatchCtx<'_>) -> Option<WorkflowNudge> {
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
        .map(WorkflowNudge::AutoContinue)
        .or_else(|| {
            wc.completion_nudge
                .then(|| engine.completion_nudge())
                .flatten()
                .map(WorkflowNudge::Completion)
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
