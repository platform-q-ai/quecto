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
    fn coordinator_board(
        &self,
    ) -> crate::application::swarm::ports::lifecycle::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::shared::error::DomainError>,
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

/// A run paused again after the resume, whose board still allows the wake.
struct PausedWaking;
impl SwarmRunControl for PausedWaking {
    fn coordinator_board(
        &self,
    ) -> crate::application::swarm::ports::lifecycle::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::shared::error::DomainError>,
    > {
        Box::pin(async {
            Ok(RunControlReceipt {
                budget: None,
                outcome: None,
                reason: None,
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
                wake_allowed: true,
                status: RunStatus::Paused,
                generation: 9,
            })
        })
    }
}

/// #2618 review: a wake that re-arms a member by a resume while the run is
/// paused again runs the change wake the board allows; that turn is the one
/// the resume owed, so no resume turn is left to run later.
#[tokio::test]
async fn the_change_wake_does_the_turn_a_resume_owed() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    super::abort_tests::suspend_failed_at(&mut ctx, 7).await;
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(PausedWaking),
        )),
    );
    ctx.turn_control.queue_swarm_wake(9);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 9).await;
    assert!(
        ctx.session.automatic_turns_allowed(),
        "re-armed by the resume"
    );
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
    assert!(
        !ctx.session.automatic_turns().take_owed_turn(),
        "the change wake did the owed turn"
    );
}

fn user_prompts(ctx: &super::super::DispatchCtx<'_>) -> Vec<String> {
    ctx.messages
        .iter()
        .filter(|m| m.role == crate::domain::conversation::value_objects::message::Role::User)
        .map(|m| m.content.clone())
        .collect()
}

/// #2531: a member whose turn failed with a failure the provider may
/// recover from (an Envoy 507 whose retries ran out) is not stranded: the
/// next wake re-arms it and runs, on the approved change-wake text.
#[tokio::test]
async fn the_next_wake_runs_a_member_a_recoverable_failure_suspended() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Waking),
        )),
    );
    ctx.session.automatic_turns().suspend_after_failure(
        &crate::domain::shared::error::DomainError::Provider(
            "HTTP 507 from Codex: exceeded request buffer limit while retrying upstream".into(),
        ),
    );
    assert!(!ctx.session.automatic_turns_allowed());
    ctx.turn_control.queue_swarm_wake(3);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 3).await;
    assert_eq!(
        user_prompts(&ctx),
        [crate::interface::cli::uds_swarm_control::CHANGED_WAKE],
        "re-armed by the wake, which runs"
    );
}

/// A terminal failure stays suspended through the same wake: only an
/// instruction or a newer resume re-arms it.
#[tokio::test]
async fn the_next_wake_leaves_a_terminal_failure_suspended() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Waking),
        )),
    );
    ctx.session.automatic_turns().suspend_after_failure(
        &crate::domain::shared::error::DomainError::Provider(
            "provider error (401): invalid credentials".into(),
        ),
    );
    ctx.session
        .automatic_turns()
        .observe_control_generation(Some(3));
    ctx.turn_control.queue_swarm_wake(3);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 3).await;
    assert!(user_prompts(&ctx).is_empty(), "no turn ran");
    assert!(!ctx.session.automatic_turns_allowed());
}

/// #2531: the bound counts re-arms without a turn that succeeded; a wake
/// turn that succeeds gives it back, so a member that keeps recovering is
/// never escalated to a terminal suspension.
#[tokio::test]
async fn a_wake_turn_that_succeeds_gives_the_recoverable_bound_back() {
    use crate::domain::inference::services::turn_suspension::MAX_RECOVERABLE_REARMS;
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Waking),
        )),
    );
    let rounds = MAX_RECOVERABLE_REARMS + 1;
    for _ in 0..rounds {
        ctx.session.automatic_turns().suspend_after_failure(
            &crate::domain::shared::error::DomainError::Provider(
                "HTTP 503 from OpenAI: service unavailable".into(),
            ),
        );
        ctx.turn_control.queue_swarm_wake(3);
        crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 3).await;
    }
    let woken = user_prompts(&ctx)
        .iter()
        .filter(|p| p.as_str() == crate::interface::cli::uds_swarm_control::CHANGED_WAKE)
        .count();
    assert_eq!(woken, rounds as usize, "every wake re-armed and ran");
}
