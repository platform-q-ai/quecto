//! #2478 slice B: the wakes `handle_wake` sends are the approved texts.
use super::*;

/// #2478 slice B: a wake the board allows runs one turn on the approved
/// change-wake text.
#[tokio::test]
async fn an_allowed_wake_sends_the_change_wake() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Waking),
        )),
    );
    ctx.turn_control.queue_swarm_wake(3);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 3).await;
    let prompts: Vec<_> = ctx
        .messages
        .iter()
        .filter(|m| m.role == crate::domain::conversation::value_objects::message::Role::User)
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        prompts,
        [crate::interface::cli::uds_swarm_control::CHANGED_WAKE]
    );
}

/// A running run that allows the wake.
struct Waking;
impl SwarmRunControl for Waking {
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
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
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
                wake_allowed: true,
                status: RunStatus::Running,
                generation: 3,
            })
        })
    }
}
