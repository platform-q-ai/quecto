//! #1729: a run the coordinator ended is a pause that still takes explicit
//! instructions, so the coordinator can report; automatic work waits.
use super::*;

/// A control port for a run the coordinator ended: paused holding `outcome`.
struct Ended(RunStatus);
impl SwarmRunControl for Ended {
    fn apply(
        &self,
        _: RunControlAction,
    ) -> crate::domain::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async {
            Ok(RunControlReceipt {
                budget: None,
                outcome: Some(self.0),
                reason: Some("proposed".into()),
                wake_allowed: false,
                status: RunStatus::Paused,
                generation: 3,
                wake_warnings: Vec::new(),
            })
        })
    }
}

/// #1729: a run the coordinator ended still runs explicit instructions (so
/// the coordinator can report) while automatic notes stay queued; a plain
/// pause keeps everything queued.
#[tokio::test]
async fn an_ended_run_admits_explicit_instructions_but_a_plain_pause_admits_nothing() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Ended(RunStatus::Succeeded)),
        )),
    );
    ctx.session
        .enqueue_subagent_notification("worker".into(), 1, "worker ended".into(), true);
    ctx.session
        .enqueue_control(Some("report"), "follow_up", "final report".into(), false);
    drain_and_run_pending(&mut ctx).await;
    assert_eq!(
        user_prompts(&ctx),
        ["final report"],
        "explicit work runs, the note waits"
    );
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Control(RunStatus::Paused)),
        )),
    );
    ctx.session
        .enqueue_control(Some("later"), "follow_up", "while paused".into(), false);
    drain_and_run_pending(&mut ctx).await;
    assert_eq!(
        user_prompts(&ctx),
        ["final report"],
        "a plain pause keeps everything queued"
    );
}
