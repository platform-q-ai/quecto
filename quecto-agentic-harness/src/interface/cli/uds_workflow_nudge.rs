//! Workflow auto-continue / completion nudge helpers for the UDS dispatch loop
//! (split out of `uds.rs`): whether — and with what message — the bound engine
//! nudges the agent to continue at an idle boundary.
use super::uds::DispatchCtx;
use crate::domain::conversation::services::turn_origin::{instruction, progress_nudge};
use crate::domain::conversation::value_objects::message::Message;

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

    /// The message to inject: a progress nudge (#2226) on the auto-continue
    /// path, its corrective wording after a `previous_turn_stalled`; the
    /// single-shot completion nudge asks for the report, an ordinary turn.
    pub(super) fn into_message(self, previous_turn_stalled: bool) -> Message {
        match self {
            WorkflowNudge::AutoContinue { corrective, .. } if previous_turn_stalled => {
                progress_nudge(corrective)
            }
            WorkflowNudge::AutoContinue { standard, .. } => progress_nudge(standard),
            WorkflowNudge::Completion(message) => instruction(message),
        }
    }
}

/// Whether a delegated child of the active session is still starting or
/// running; the parent identity is the active session key's (D10 #1979).
pub(super) async fn has_active_workflow_descendant(ctx: &DispatchCtx<'_>) -> bool {
    let session_key = ctx.sessions.current_session_key().await;
    crate::infrastructure::tools::subagent_identity::parent_identity_from_session_key(&session_key)
        .is_some_and(|current_identity| {
            crate::infrastructure::tools::subagent_registry::has_active_descendant_for_agent(
                &ctx.subagent_registry,
                current_identity,
            )
        })
}

/// The next workflow nudge, if auto-continue or completion nudging is
/// enabled and the engine still has something to say.
pub(super) async fn workflow_nudge_message(ctx: &DispatchCtx<'_>) -> Option<WorkflowNudge> {
    let (Some(ws), Some(_)) = (&ctx.workflow_state, &ctx.workflow_config) else {
        return None;
    };
    if has_active_workflow_descendant(ctx).await {
        return None;
    }
    // #2226: auto-continue names the workflow tool; only a shown one.
    let workflow_tool_shown = ctx
        .agent
        .is_tool_model_visible(crate::infrastructure::tools::workflow_tool::WORKFLOW_TOOL_NAME);
    let engine = crate::domain::workflow::lock_engine(ws);
    // The engine owns all nudge policy (kept live by `set_automation`): it
    // yields the template selector even with auto-continue disabled, the sole
    // proactive selection channel (#1113 AC3), so a `wc.auto_continue`
    // pre-filter here would leave a session never told to select a template.
    (|| {
        workflow_tool_shown.then_some(())?;
        Some(WorkflowNudge::AutoContinue {
            standard: engine.auto_continue_nudge()?,
            corrective: engine.corrective_nudge()?,
        })
    })()
    .or_else(|| engine.completion_nudge().map(WorkflowNudge::Completion))
}

/// A serialized fingerprint of workflow progress: whether a nudge advanced
/// the workflow, so a stuck workflow isn't nudged forever.
pub(super) fn workflow_progress_fingerprint(ctx: &DispatchCtx<'_>) -> Option<String> {
    let ws = ctx.workflow_state.as_ref()?;
    let engine = crate::domain::workflow::lock_engine(ws);
    let mut snapshot = engine.snapshot(true);
    snapshot.steps = engine.all_step_statuses();
    serde_json::to_string(&snapshot).ok()
}

/// The reason on the terminal `workflow_idle` event (#1082 review):
/// `Completed` when no workflow is bound or it reached a terminal state,
/// `Exhausted` when the drain gave up (no-progress tolerance, nudge cap,
/// nothing runnable) with it unfinished, the only stall-worthy outcome. A
/// poisoned engine is read as it was left (#2192).
pub(super) fn workflow_idle_reason(ctx: &DispatchCtx<'_>) -> super::protocol::WorkflowIdleReason {
    use super::protocol::WorkflowIdleReason;
    use crate::domain::workflow::WorkflowMode;
    let Some(ws) = ctx.workflow_state.as_ref() else {
        return WorkflowIdleReason::Completed;
    };
    let engine = crate::domain::workflow::lock_engine(ws);
    match engine.mode() {
        WorkflowMode::Complete => WorkflowIdleReason::Completed,
        WorkflowMode::Active | WorkflowMode::SelectingTemplate => WorkflowIdleReason::Exhausted,
    }
}
