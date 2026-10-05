//! Reload runtime configuration on the dispatch loop (#1849): the use
//! case's rebuild phase (config read + provider composition) runs under
//! `tokio::task::spawn_blocking`, so the current-thread UDS runtime keeps
//! accepting, reading and flushing for other clients while a rebuild is in
//! flight; the apply phase then swaps the result into the agent on the
//! dispatch task. The use case itself knows no scheduler.

use std::sync::Arc;

use crate::application::catalogue::dto::ReloadOutcome;
use crate::application::catalogue::use_cases::{ReloadRuntimeConfiguration, ReloadStep};

use super::uds::DispatchCtx;

/// The error a forced reload reports when the rebuild worker panicked.
pub(super) const WORKER_PANICKED: &str = "provider reload worker panicked";

/// Run one rebuild phase — forced or polled — off the runtime's scheduler
/// and hand back the step to apply. A worker that panicked yields the
/// forced-failure step, or the polled retain-last-good step.
pub(super) async fn rebuild_off_runtime(
    reload: Arc<ReloadRuntimeConfiguration>,
    forced: bool,
) -> ReloadStep {
    let joined = tokio::task::spawn_blocking(move || {
        if forced {
            reload.rebuild()
        } else {
            reload.rebuild_if_changed()
        }
    })
    .await;
    match joined {
        Ok(step) => step,
        Err(join_error) => {
            tracing::warn!(target: "reload", error = %join_error, "reload rebuild worker did not complete; keeping last-good");
            if forced {
                ReloadStep::Failed(WORKER_PANICKED.to_string())
            } else {
                ReloadStep::Unchanged
            }
        }
    }
}

/// The forced reload (UDS `reload`): rebuild off the runtime, apply here.
pub(super) async fn force_reload(ctx: &mut DispatchCtx<'_>) -> ReloadOutcome {
    let reload = ctx.catalogue.reload.clone();
    let step = rebuild_off_runtime(reload.clone(), true).await;
    reload.apply(ctx.agent, step)
}

/// The pull-based poll before a prompt or `set_model` (ADR-0002): rebuild
/// off the runtime only if a watched file changed, apply here.
pub(super) async fn poll_reload(ctx: &mut DispatchCtx<'_>) -> ReloadOutcome {
    let reload = ctx.catalogue.reload.clone();
    let step = rebuild_off_runtime(reload.clone(), false).await;
    reload.apply(ctx.agent, step)
}

/// Before `list_models`, the same poll (#2451): the listing offers what the
/// router can reach, so a provider configured since the last poll is
/// offered when `/model` opens rather than after the next prompt. Any
/// other command polls nothing here.
pub(super) async fn poll_before_listing(
    cmd: &crate::interface::cli::protocol::AgentCommand,
    ctx: &mut DispatchCtx<'_>,
) {
    if matches!(
        cmd,
        crate::interface::cli::protocol::AgentCommand::ListModels { .. }
    ) {
        poll_reload(ctx).await;
    }
}

#[cfg(test)]
#[path = "uds_dispatch_reload_tests.rs"]
mod tests;
