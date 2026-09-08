//! Pending turn delivery yields to admitted explicit steering.
use super::{DispatchCtx, run_drained_message};

pub(super) async fn drain_and_run_pending(ctx: &mut DispatchCtx<'_>) {
    let _busy = crate::interface::cli::uds_multi::BusyGuard::new(&ctx.busy); // #828
    loop {
        // Coalesce a burst of buffered sub-agent completion notes into ONE
        // informational note at the idle flush (#894) — K separate notes would
        // send the parent into a catch-up loop. Per-agent dedup already happened
        // at enqueue; #816 deferral is preserved (this only runs at idle).
        // The reader has admitted an explicit replacement instruction. Yield
        // to command dispatch before consuming buffered automated work.
        if ctx.turn_control.is_steer_pending() {
            return;
        }
        let pending =
            crate::interface::cli::uds_session::coalesce_pending(ctx.session.drain_pending());
        if pending.is_empty() {
            break;
        }
        let mut remaining = pending.into_iter();
        while let Some(pending_msg) = remaining.next() {
            if ctx.turn_control.is_steer_pending() {
                ctx.session
                    .restore_pending(std::iter::once(pending_msg).chain(remaining));
                return;
            }
            run_drained_message(ctx, pending_msg.into_message()).await;
        }
    }
}

/// A second-stage dispatch acknowledgment must reflect actual retention.
pub(super) async fn queue_prompt(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    message: String,
    steer: bool,
) -> bool {
    let type_name = if steer { "steer" } else { type_name };
    let retained = if steer {
        ctx.turn_control.clear_steer();
        ctx.session.prepend_pending(message)
    } else {
        ctx.session.enqueue_pending(message)
    };
    let event = if retained {
        super::AgentEvent::ok(id, type_name, Some(serde_json::json!({"status":"queued"})))
    } else {
        super::AgentEvent::err(
            id,
            type_name,
            "pending prompt queue is full; instruction was not retained",
        )
    };
    super::emit_event_to_broadcast_or_writer(ctx, &event).await;
    retained
}
