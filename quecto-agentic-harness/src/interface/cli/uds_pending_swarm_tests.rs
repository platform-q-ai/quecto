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
                wake_warnings: Vec::new(),
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
                wake_warnings: Vec::new(),
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

/// A control port answering with a fixed status/generation or an error.
struct Answer(Result<(RunStatus, u64), &'static str>);
impl SwarmRunControl for Answer {
    fn apply(
        &self,
        _: RunControlAction,
    ) -> crate::domain::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async {
            match self.0 {
                Ok((status, generation)) => Ok(RunControlReceipt {
                    budget: None,
                    wake_warnings: Vec::new(),
                    wake_allowed: false,
                    status,
                    generation,
                }),
                Err(message) => Err(crate::domain::error::DomainError::Tool(message.into())),
            }
        })
    }
}

fn with_control(ctx: &mut DispatchCtx<'_>, control: Answer) {
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(control),
        )),
    );
}

fn user_prompts(ctx: &DispatchCtx<'_>) -> Vec<String> {
    ctx.messages
        .iter()
        .filter(|m| m.role == crate::domain::message::Role::User)
        .map(|m| m.content.clone())
        .collect()
}

/// #1721: a resume seen while the run is still paused re-arms the member but
/// the owed turn waits until the run admits inference again.
#[tokio::test]
async fn a_re_arm_while_paused_owes_the_turn_until_the_run_runs() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(5));
    with_control(&mut ctx, Answer(Ok((RunStatus::Paused, 7))));
    ctx.turn_control.queue_swarm_wake(40);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 40).await;
    assert!(
        ctx.session.automatic_turns_allowed,
        "re-armed at generation 7"
    );
    assert!(user_prompts(&ctx).is_empty(), "no turn while paused");
    with_control(&mut ctx, Answer(Ok((RunStatus::Running, 7))));
    ctx.turn_control.queue_swarm_wake(41);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 41).await;
    let prompts = user_prompts(&ctx);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].contains("resumed after a provider failure"));
}

/// #1721: a status probe that fails while suspended keeps the member
/// suspended; contention defers the generation to the next wake the reader
/// delivers (the coalescing slot stays free so that wake still sends its
/// message), a durable rejection drops it.
#[tokio::test]
async fn a_failed_status_probe_keeps_a_suspended_member_suspended() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(5));
    with_control(&mut ctx, Answer(Err("database is locked")));
    ctx.turn_control.queue_swarm_wake(40);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 40).await;
    assert!(!ctx.session.automatic_turns_allowed);
    assert!(
        ctx.turn_control.queue_swarm_wake(1),
        "the reader's next wake still opens the slot and sends its message"
    );
    assert_eq!(
        ctx.turn_control.take_swarm_wake(0),
        40,
        "the deferred generation folds into that wake"
    );
    assert_eq!(ctx.turn_control.take_swarm_wake(0), 0, "consumed once");
    with_control(&mut ctx, Answer(Err("run not found")));
    ctx.turn_control.queue_swarm_wake(41);
    crate::interface::cli::uds_swarm_control::handle_wake(&mut ctx, 41).await;
    assert!(!ctx.session.automatic_turns_allowed);
    assert_eq!(ctx.turn_control.take_swarm_wake(0), 0, "durably dropped");
    assert!(user_prompts(&ctx).is_empty());
}

/// #1721: a provider-failure suspension is dated by the control generation
/// current after the failed turn, so a pause/resume during that turn cannot
/// re-arm it; anything else is left alone.
#[tokio::test]
async fn a_provider_suspension_is_dated_after_the_failed_turn() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.session.observe_control_generation(Some(4));
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, None);
    with_control(&mut ctx, Answer(Ok((RunStatus::Running, 6))));
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    assert_eq!(ctx.turn_control.control_generation(), Some(6));
    assert!(
        !ctx.session.resume_after_control_change(6),
        "the pause/resume during the turn is not a resume after it"
    );
    assert!(ctx.session.resume_after_control_change(7));
    // A suspension already dated after its failure is not re-dated by a
    // later idle drain: the resume it should honour stays visible.
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(4));
    with_control(&mut ctx, Answer(Ok((RunStatus::Running, 9))));
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    assert!(
        ctx.session.resume_after_control_change(9),
        "still dated at 4"
    );
    // Store rejections and healthy sessions are untouched.
    ctx.session
        .suspend_automatic_turns(SuspensionCause::StoreRejection, Some(1));
    with_control(&mut ctx, Answer(Ok((RunStatus::Running, 9))));
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    assert!(!ctx.session.resume_after_control_change(10));
    ctx.session.resume_automatic_turns();
    with_control(&mut ctx, Answer(Err("run not found")));
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    assert!(ctx.session.automatic_turns_allowed);
}

/// A provider that always fails terminally.
#[derive(Debug)]
struct FailingProvider;
impl crate::domain::provider::LlmProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn chat(
        &self,
        _: crate::domain::provider::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Err(crate::domain::error::DomainError::Provider(
                "provider error (500): boom".into(),
            ))
        })
    }
}

/// #1721: a drained follow-up that fails is dated like a prompt: by the
/// generation current after the failure, not the pre-turn one.
#[tokio::test]
async fn a_drained_turn_failure_is_dated_after_the_turn() {
    let mut env = Env::new(
        super::dispatch_test_env::make_workflow(),
        std::sync::Arc::new(FailingProvider),
    );
    let mut ctx = env.ctx();
    ctx.session.observe_control_generation(Some(4));
    with_control(&mut ctx, Answer(Ok((RunStatus::Running, 6))));
    ctx.session
        .enqueue_control(Some("f1"), "follow_up", "carry on".into(), false);
    drain_pending_and_nudge(&mut ctx).await;
    assert!(
        !ctx.session.automatic_turns_allowed,
        "the drained turn failed"
    );
    assert!(!ctx.session.needs_provider_dating());
    assert!(
        !ctx.session.resume_after_control_change(6),
        "a pause/resume during the drained turn does not re-arm"
    );
    assert!(ctx.session.resume_after_control_change(7));
}

/// #1712: an explicit instruction (a parent's fast-acked prompt becomes a
/// queued follow-up) re-arms a member idle after a provider failure and
/// runs, while buffered automatic notifications alone never do; a pending
/// steer still outranks the drain.
#[tokio::test]
async fn an_explicit_follow_up_re_arms_a_provider_suspended_idle_member() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut env = Env::new(
        super::dispatch_test_env::make_workflow(),
        std::sync::Arc::new(CountingProvider(calls.clone())),
    );
    let mut ctx = env.ctx();
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(1));
    ctx.session
        .enqueue_subagent_notification("worker".into(), 1, "worker ended".into(), true);
    drain_and_run_pending(&mut ctx).await;
    assert!(
        !ctx.session.automatic_turns_allowed,
        "an automatic notification alone stays suspended"
    );
    assert!(user_prompts(&ctx).is_empty());
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no provider request for an automatic note while suspended"
    );
    let kept = ctx.session.drain_pending();
    assert_eq!(kept.len(), 1, "the note is kept for later: {kept:?}");
    ctx.session.restore_pending(kept.into_iter());

    ctx.turn_control.mark_steer();
    ctx.session
        .enqueue_control(Some("f1"), "follow_up", "carry on".into(), false);
    drain_and_run_pending(&mut ctx).await;
    assert!(
        user_prompts(&ctx).is_empty(),
        "a pending steer outranks the drain"
    );
    assert!(!ctx.session.automatic_turns_allowed);
    ctx.turn_control.clear_steer();

    drain_and_run_pending(&mut ctx).await;
    let prompts = user_prompts(&ctx);
    assert_eq!(
        prompts.first().map(String::as_str),
        Some("carry on"),
        "{prompts:?}"
    );
    assert!(
        ctx.session.automatic_turns_allowed,
        "re-armed by the explicit instruction"
    );
    assert_eq!(
        prompts.len(),
        2,
        "once re-armed the kept note drains after the instruction: {prompts:?}"
    );
    assert!(ctx.session.drain_pending().is_empty());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(
        ctx.session.control_receipt_status("f1"),
        Some(crate::interface::cli::protocol::ControlStatus::Completed)
    );
}

/// #1712 (fast-ack path): an idle suspended member receiving the parent's
/// converted follow-up executes it rather than parking it as queued.
#[tokio::test]
async fn an_idle_suspended_member_executes_a_forwarded_follow_up() {
    use crate::interface::cli::uds_session::SuspensionCause;
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    ctx.session
        .suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(1));
    super::uds_dispatch::handle_follow_up(
        &mut ctx,
        Some("p1"),
        "follow_up",
        "continue the work".into(),
    )
    .await;
    assert_eq!(user_prompts(&ctx), ["continue the work"]);
    assert!(ctx.session.automatic_turns_allowed);
}

/// A provider that succeeds and counts its requests.
#[derive(Debug)]
struct CountingProvider(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl crate::domain::provider::LlmProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn chat(
        &self,
        _: crate::domain::provider::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async {
            Ok(crate::domain::message::LlmResponse {
                content: Some("ok".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}
