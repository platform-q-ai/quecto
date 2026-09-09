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

#[tokio::test]
async fn production_swarm_wiring_accounts_for_a_turn_and_denies_paused_inference() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let mut env = Env::with_unselected_workflow();
    env.agent =
        crate::interface::cli::swarm_composition::wire_agent(env.agent, Some(context.clone()));
    let mut ctx = env.ctx();
    handle_prompt(
        &mut ctx,
        PromptCommand {
            id: Some("working".into()),
            type_name: "prompt".into(),
            message: "inspect".into(),
            streaming_behavior: None,
        },
    )
    .await;
    assert_eq!(context.usage_report().unwrap()["totals"]["requests"], 1);
    context.pause("approval").unwrap();
    handle_prompt(
        &mut ctx,
        PromptCommand {
            id: Some("paused".into()),
            type_name: "prompt".into(),
            message: "wait".into(),
            streaming_behavior: None,
        },
    )
    .await;
    let report = context.usage_report().unwrap();
    assert_eq!(report["totals"]["attempts"], 1);
    assert_eq!(
        report["recent_requests"][0]["observation"]["outcome"],
        "rejected"
    );
}

#[tokio::test]
async fn stale_and_unavailable_wakes_cannot_create_model_turns() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(context),
        )),
    );
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 0).await;
    assert!(ctx.messages.is_empty());
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, u64::MAX).await;
    assert!(ctx.messages.is_empty());
    assert!(!ctx.session.automatic_turns_allowed);
}

#[test]
fn composed_suspension_rechecks_durable_generation_before_cancelling() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let paused = context.pause("approval").unwrap();
    let generation = paused["generation"].as_u64().unwrap();
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    let handle = std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::ScopedArmed(
        sender,
        RunStatus::Running,
        0,
    )));
    let suspend =
        crate::interface::cli::swarm_composition::suspension_callback(&handle, context.clone());
    suspend(RunStatus::Paused, generation);
    assert_eq!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    );
    context.resume().unwrap();
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    *handle.lock().unwrap() = CancelSlot::ScopedArmed(sender, RunStatus::Running, 0);
    suspend(RunStatus::Paused, generation);
    assert_eq!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );
}

/// A running run at a given control generation that never grants a wake, so
/// the re-arming can be observed without a model turn.
struct RunningAt(u64);
impl SwarmRunControl for RunningAt {
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
                status: RunStatus::Running,
                generation: self.0,
            })
        })
    }
}

/// #1721: a status receipt newer than the generation a provider-failure
/// suspension was dated under re-arms automatic turns and the member then
/// runs one resume turn even though the wake targets nobody; an equal
/// generation (no pause/resume since) stays a plain notification; a store
/// rejection never re-arms this way.
#[tokio::test]
async fn a_resume_wake_re_arms_a_provider_suspended_member() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(RunningAt(5)),
        )),
    );
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(5));
    ctx.turn_control.queue_swarm_wake(40);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 40).await;
    assert!(
        !ctx.session.automatic_turns_allowed,
        "a wake at the suspension's own control generation is just a notification"
    );
    assert!(ctx.messages.is_empty());
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(RunningAt(7)),
        )),
    );
    ctx.turn_control.queue_swarm_wake(41);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 41).await;
    assert!(
        ctx.session.automatic_turns_allowed,
        "resumed at generation 7"
    );
    assert_eq!(ctx.turn_control.control_generation(), Some(7));
    let prompts: Vec<_> = ctx
        .messages
        .iter()
        .filter(|m| m.role == crate::domain::message::Role::User)
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(prompts.len(), 1, "exactly one resume turn: {prompts:?}");
    assert!(prompts[0].contains("resumed after a provider failure"));
    assert!(
        !ctx.session.take_pending_resume_turn(),
        "the owed turn was consumed"
    );
    ctx.session
        .suspend_automatic_turns(SuspensionCause::StoreRejection, Some(7));
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(RunningAt(9)),
        )),
    );
    ctx.turn_control.queue_swarm_wake(42);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 42).await;
    assert!(
        !ctx.session.automatic_turns_allowed,
        "store rejections wait for a human"
    );
}
