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
    /// Carries both engine-owned wordings: the standard nudge and the
    /// corrective variant sent after a no-progress nudged turn.
    AutoContinue {
        standard: String,
        corrective: String,
    },
    Completion(String),
}

impl WorkflowNudge {
    pub(super) fn is_auto_continue(&self) -> bool {
        matches!(self, WorkflowNudge::AutoContinue { .. })
    }

    /// The message to inject. With `previous_turn_stalled` the auto-continue
    /// path yields its corrective wording instead of the verbatim repeat; the
    /// completion nudge is single-shot and has no corrective form.
    pub(super) fn into_message(self, previous_turn_stalled: bool) -> String {
        match self {
            WorkflowNudge::AutoContinue { corrective, .. } if previous_turn_stalled => corrective,
            WorkflowNudge::AutoContinue { standard, .. } => standard,
            WorkflowNudge::Completion(message) => message,
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
        .then(|| {
            Some(WorkflowNudge::AutoContinue {
                standard: engine.auto_continue_nudge()?,
                corrective: engine.corrective_nudge()?,
            })
        })
        .flatten()
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

/// The reason to attach to the terminal `workflow_idle` boundary event
/// (#1082 review): `Completed` when no workflow is bound or the bound
/// workflow reached a terminal state, `Exhausted` otherwise — the drain gave
/// up (no-progress tolerance, nudge cap, or nothing runnable) with the
/// workflow still unfinished, which is the only stall-worthy outcome.
pub(super) fn workflow_idle_reason(ctx: &DispatchCtx<'_>) -> super::protocol::WorkflowIdleReason {
    use super::protocol::WorkflowIdleReason;
    use crate::domain::workflow::WorkflowMode;
    let Some(ws) = ctx.workflow_state.as_ref() else {
        return WorkflowIdleReason::Completed;
    };
    let Ok(engine) = ws.lock() else {
        return WorkflowIdleReason::Completed;
    };
    match engine.mode() {
        WorkflowMode::Complete => WorkflowIdleReason::Completed,
        WorkflowMode::Active | WorkflowMode::SelectingTemplate => WorkflowIdleReason::Exhausted,
    }
}
