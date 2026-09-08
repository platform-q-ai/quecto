use super::dispatch_test_env::DispatchTestEnv as Env;
use super::*;
use crate::domain::swarm::{RunControlAction, RunControlReceipt, RunStatus, SwarmRunControl};
struct Control(RunStatus);
impl SwarmRunControl for Control {
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
                wake_allowed: false,
                status: self.0,
                generation: 1,
            })
        })
    }
}
#[tokio::test]
async fn paused_pending_controls_are_retained_without_attempting_inference() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Control(RunStatus::Paused)),
        )),
    );
    ctx.session
        .enqueue_control(Some("approval"), "follow_up", "approved".into(), false);
    drain_and_run_pending(&mut ctx).await;
    assert!(ctx.messages.is_empty());
    assert_eq!(ctx.session.drain_pending().len(), 1);
}
#[tokio::test]
async fn terminal_notifications_do_not_create_extra_report_turns() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Control(RunStatus::Failed)),
        )),
    );
    ctx.session
        .enqueue_subagent_notification("worker".into(), 1, "worker ended".into(), true);
    ctx.session
        .enqueue_control(Some("report"), "follow_up", "final report".into(), false);
    drain_and_run_pending(&mut ctx).await;
    let prompts: Vec<_> = ctx
        .messages
        .iter()
        .filter(|message| message.role == crate::domain::message::Role::User)
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(prompts, ["final report"]);
}
