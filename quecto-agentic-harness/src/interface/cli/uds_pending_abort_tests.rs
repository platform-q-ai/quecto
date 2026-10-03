//! A failed explicit turn that a stale abort drain ends is dated after the
//! turn (moved out of `uds_pending_swarm_tests.rs` for its line budget).
use super::*;

#[tokio::test]
async fn a_stale_abort_drain_dates_a_failed_explicit_turn() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let mut env = Env::new(
        super::dispatch_test_env::make_workflow(),
        std::sync::Arc::new(FailingProvider),
    );
    let mut ctx = env.ctx();
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(4));
    super::pending::queue_prompt(&mut ctx, Some("f3"), "follow_up", "carry on".into(), false).await;
    // Every status probe sees a later control generation (6, 16, 26, ...):
    // the prompt's own arming probe, the drain's admission probe, the
    // drained turn's arming probe, then the post-failure dating probe.
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Rising(std::sync::atomic::AtomicU64::new(6))),
        )),
    );
    *ctx.cancel_handle.lock().unwrap() = crate::interface::cli::uds_cancel::CancelSlot::Fired;
    super::handle_prompt(
        &mut ctx,
        super::PromptCommand {
            id: Some("p".into()),
            type_name: "prompt".into(),
            message: "hello".into(),
            streaming_behavior: None,
        },
    )
    .await;
    assert_eq!(
        ctx.session.control_receipt_status("f3"),
        Some(crate::interface::cli::protocol::ControlStatus::Failed),
        "the follow-up drained on the stale-abort path and failed"
    );
    assert!(!ctx.session.automatic_turns_allowed);
    assert!(
        !ctx.session.resume_after_control_change(26),
        "dated after the failed drained turn (36), not at a generation seen before or during it"
    );
    assert!(ctx.session.resume_after_control_change(37));
}

/// A control port whose generation rises by ten on every probe.
struct Rising(std::sync::atomic::AtomicU64);
impl SwarmRunControl for Rising {
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async {
            Ok(RunControlReceipt {
                budget: None,
                outcome: None,
                reason: None,
                wake_allowed: false,
                status: RunStatus::Running,
                generation: self.0.fetch_add(10, std::sync::atomic::Ordering::SeqCst),
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
            })
        })
    }
}
