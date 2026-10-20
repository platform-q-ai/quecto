//! #2478 slice B: the wakes `handle_wake` sends are the approved texts.
//! #2531: a recoverable provider failure never strands a member: the next
//! wake, its own delayed self-wake or (without a swarm) the next automatic
//! note re-arms it, a bounded number of times.
use super::*;
use crate::domain::inference::services::turn_suspension::{
    MAX_RECOVERABLE_REARMS, RECOVERABLE_SELF_WAKE_DELAY, WakeOutcome,
};
use crate::interface::cli::uds_swarm_control::{RECOVERED_WAKE, handle_wake};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The failure an Envoy 507 or a 503 leaves once its retries ran out.
fn unavailable() -> crate::domain::shared::error::DomainError {
    crate::domain::shared::error::DomainError::Provider(
        "HTTP 503 from OpenAI: service unavailable".into(),
    )
}

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
/// A running run whose wake cursor already holds every generation: a wake
/// claims nothing, as a coordinator's or the last member's self-wake does.
struct Claimed;

fn running(wake_allowed: bool) -> RunControlReceipt {
    RunControlReceipt {
        budget: None,
        outcome: None,
        reason: None,
        wake_warnings: Vec::new(),
        resume_blockers: Vec::new(),
        wake_allowed,
        status: RunStatus::Running,
        generation: 3,
    }
}

impl SwarmRunControl for Claimed {
    fn coordinator_board(&self) -> crate::application::swarm::ports::lifecycle::CoordinatorBoardFuture<'_> {
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
        Box::pin(async { Ok(running(false)) })
    }
}

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
        Box::pin(async { Ok(running(true)) })
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
        ctx.session.automatic_turns().take_owed_turn().is_none(),
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

/// The `resumed` reasons the swarm-wake events carried.
fn resumed_reasons(events: &mut tokio::sync::broadcast::Receiver<String>) -> Vec<String> {
    let mut reasons = Vec::new();
    while let Ok(line) = events.try_recv() {
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        if event["command"] == "swarm_wake" && event["data"]["status"] == "resumed" {
            reasons.push(event["data"]["reason"].as_str().unwrap().to_owned());
        }
    }
    reasons
}

/// #2531: a member whose turn failed with a failure the provider may
/// recover from (an Envoy 507 whose retries ran out) is not stranded: the
/// next wake re-arms it, says why, and runs a turn that continues the
/// interrupted work.
#[tokio::test]
async fn the_next_wake_runs_a_member_a_recoverable_failure_suspended() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    let (sender, mut events) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(sender);
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
        [RECOVERED_WAKE],
        "re-armed by the wake, which owes the run a continuation turn"
    );
    assert_eq!(
        resumed_reasons(&mut events),
        ["automatic turns re-armed by a swarm wake after a recoverable provider failure"],
        "the approved reason"
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
        .filter(|p| p.as_str() == RECOVERED_WAKE)
        .count();
    assert_eq!(woken, rounds as usize, "every wake re-armed and ran");
}

/// A provider that is always unavailable (a 503 whose retries ran out),
/// counting its calls.
#[derive(Debug)]
struct Unavailable(Arc<AtomicUsize>);
impl crate::application::providers::ports::LlmProvider for Unavailable {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "unavailable"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn chat(
        &self,
        _: crate::application::providers::ports::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        crate::domain::conversation::value_objects::message::LlmResponse,
                        crate::domain::shared::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(unavailable()) })
    }
}

fn unavailable_env(calls: &Arc<AtomicUsize>) -> Env {
    Env::new(
        super::super::dispatch_test_env::make_workflow(),
        Arc::new(Unavailable(calls.clone())),
    )
}

/// #2531 review: an always-unavailable provider is called on the first
/// [`MAX_RECOVERABLE_REARMS`] wakes only; past the bound the member stays
/// suspended (as a terminal failure) and no wake calls the provider again.
/// A wake turn's failure never gives the bound back.
#[tokio::test]
async fn an_always_unavailable_provider_stops_at_the_recoverable_bound() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut env = unavailable_env(&calls);
    let mut ctx = env.ctx();
    ctx.turn_control = Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(Arc::new(Waking))),
    );
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    let mut seen = Vec::new();
    for _ in 0..=MAX_RECOVERABLE_REARMS {
        ctx.turn_control.queue_swarm_wake(3);
        handle_wake(&mut ctx, 3).await;
        seen.push(calls.load(Ordering::SeqCst));
    }
    let bound = MAX_RECOVERABLE_REARMS as usize;
    assert!(seen[0] > 0, "the first wake re-armed and called: {seen:?}");
    assert!(
        seen.windows(2).take(bound - 1).all(|w| w[1] > w[0]),
        "each wake within the bound called: {seen:?}"
    );
    assert_eq!(
        seen[bound],
        seen[bound - 1],
        "the wake past the bound calls nothing: {seen:?}"
    );
    assert_eq!(user_prompts(&ctx).len(), bound, "one turn per re-arm");
    assert!(!ctx.session.automatic_turns_allowed());
    let probe = ctx
        .session
        .automatic_turns()
        .probe_while_suspended(&Waking)
        .await
        .unwrap();
    assert_eq!(probe.outcome, WakeOutcome::StillSuspended);
}

/// #2531 review: a coordinator, or the last active member, has no one to
/// wake it: suspended by a recoverable failure it wakes itself once after
/// the delay, and that wake runs the continuation turn although it claims
/// no generation.
#[tokio::test(start_paused = true)]
async fn a_member_no_one_wakes_wakes_itself_after_a_recoverable_failure() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    let turn_control = Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(Arc::new(Claimed))),
    );
    let (waker, mut wakes) = tokio::sync::mpsc::channel(4);
    turn_control.bind_self_waker(waker.downgrade());
    ctx.turn_control = turn_control;
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    let early = RECOVERABLE_SELF_WAKE_DELAY - std::time::Duration::from_secs(1);
    assert!(
        tokio::time::timeout(early, wakes.recv()).await.is_err(),
        "not before the delay"
    );
    let woken = tokio::time::timeout(RECOVERABLE_SELF_WAKE_DELAY, wakes.recv()).await;
    let Ok(Some(crate::interface::cli::uds_multi::ClientMessage::SwarmWake { generation })) = woken
    else {
        panic!("a self-wake after the delay");
    };
    assert_eq!(generation, 3, "at the generation it last saw");
    assert!(
        wakes.try_recv().is_err(),
        "one self-wake for the suspension"
    );
    // The dispatch loop hands the wake to `handle_wake`, as a reader's.
    handle_wake(&mut ctx, generation).await;
    assert_eq!(user_prompts(&ctx), [RECOVERED_WAKE]);
    assert!(ctx.session.automatic_turns_allowed());
}

/// #2531 review: a terminal failure schedules no self-wake.
#[tokio::test(start_paused = true)]
async fn a_terminal_failure_does_not_wake_itself() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    let turn_control = Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(Arc::new(Claimed))),
    );
    let (waker, mut wakes) = tokio::sync::mpsc::channel(4);
    turn_control.bind_self_waker(waker.downgrade());
    ctx.turn_control = turn_control;
    ctx.session.automatic_turns().suspend_after_failure(
        &crate::domain::shared::error::DomainError::Provider(
            "provider error (401): invalid credentials".into(),
        ),
    );
    crate::interface::cli::uds_swarm_control::date_provider_suspension(&mut ctx).await;
    let window = RECOVERABLE_SELF_WAKE_DELAY * 2;
    assert!(tokio::time::timeout(window, wakes.recv()).await.is_err());
}

/// #2531 review: without a swarm no wake comes, so the next automatic note
/// re-arms a recoverable failure, within the same bound: an always
/// unavailable provider is called for the first notes only, and the note
/// past the bound waits for an instruction.
#[tokio::test]
async fn without_a_swarm_the_next_note_re_arms_a_recoverable_failure() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut env = unavailable_env(&calls);
    let mut ctx = env.ctx();
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    let mut seen = Vec::new();
    for sequence in 1..=u64::from(MAX_RECOVERABLE_REARMS) + 1 {
        ctx.session.enqueue_subagent_notification(
            format!("worker-{sequence}"),
            sequence,
            "worker ended".into(),
            crate::interface::cli::uds_session::NoteClass::Completion,
        );
        drain_and_run_pending(&mut ctx).await;
        seen.push(calls.load(Ordering::SeqCst));
    }
    let bound = MAX_RECOVERABLE_REARMS as usize;
    assert!(seen[0] > 0, "the first note re-armed and ran: {seen:?}");
    assert!(
        seen.windows(2).take(bound - 1).all(|w| w[1] > w[0]),
        "each note within the bound ran: {seen:?}"
    );
    assert_eq!(seen[bound], seen[bound - 1], "past the bound: {seen:?}");
    assert!(!ctx.session.automatic_turns_allowed());
    assert!(
        !ctx.session.drain_pending().is_empty(),
        "the note past the bound is kept"
    );
}

/// #2531 review: one self-wake is scheduled at a time, and another may be
/// scheduled once it fired; none without a bound channel.
#[tokio::test(start_paused = true)]
async fn one_self_wake_is_scheduled_at_a_time() {
    let turn_control = Arc::new(crate::interface::cli::uds_cancel::TurnControl::default());
    assert!(
        !turn_control.schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, 0),
        "no channel bound"
    );
    let (waker, mut wakes) = tokio::sync::mpsc::channel(4);
    turn_control.bind_self_waker(waker.downgrade());
    assert!(turn_control.schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, 0));
    assert!(
        !turn_control.schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, 0),
        "one at a time"
    );
    let first = tokio::time::timeout(RECOVERABLE_SELF_WAKE_DELAY * 2, wakes.recv()).await;
    assert!(matches!(first, Ok(Some(_))), "the first fired");
    turn_control.take_swarm_wake(0);
    assert!(
        turn_control.schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, 0),
        "another once it fired"
    );
}

#[path = "uds_self_wake_tests.rs"]
mod self_wake;
