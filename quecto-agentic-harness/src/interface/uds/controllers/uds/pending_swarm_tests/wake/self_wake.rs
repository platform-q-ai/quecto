//! #2626 review: a self-wake is never lost to a locked store, never re-arms
//! a newer suspension than the one it was scheduled for, holds no strong
//! sender on the loop's channel and sends nothing while the process ends;
//! a paused run's wake re-arms no recoverable failure.
use super::*;
use crate::interface::uds::controllers::uds_swarm_control::date_provider_suspension;
use crate::interface::uds::framing::uds_multi::ClientMessage;

/// A running run whose store is locked for the first `status` status reads
/// and the first `wake` wake claims; a wake claims no generation.
struct LockedOnce {
    status: AtomicUsize,
    wake: AtomicUsize,
}

impl LockedOnce {
    fn new(status: usize, wake: usize) -> Arc<Self> {
        Arc::new(Self {
            status: AtomicUsize::new(status),
            wake: AtomicUsize::new(wake),
        })
    }
}

impl SwarmRunControl for LockedOnce {
    fn coordinator_board(
        &self,
    ) -> crate::application::swarm::ports::lifecycle::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        action: RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::shared::error::DomainError>,
    > {
        let counter = match action {
            RunControlAction::Wake { .. } => &self.wake,
            _ => &self.status,
        };
        let locked = take_one(counter);
        Box::pin(async move {
            match locked {
                true => Err(crate::domain::shared::error::DomainError::Tool(
                    "database is locked".into(),
                )),
                false => Ok(running(false)),
            }
        })
    }
}

/// A session a recoverable failure suspended, over `control`, with the
/// loop's channel bound for self-wakes.
fn suspended_over(
    env: &mut Env,
    control: Arc<dyn SwarmRunControl>,
) -> (
    super::super::super::DispatchCtx<'_>,
    tokio::sync::mpsc::Sender<ClientMessage>,
    tokio::sync::mpsc::Receiver<ClientMessage>,
) {
    let mut ctx = env.ctx();
    let turn_control = Arc::new(
        crate::interface::uds::controllers::uds_cancel::TurnControl::with_swarm_control(Some(
            control,
        )),
    );
    let (waker, wakes) = tokio::sync::mpsc::channel(4);
    turn_control.bind_self_waker(waker.downgrade());
    ctx.turn_control = turn_control;
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    (ctx, waker, wakes)
}

/// The self-wake the loop's channel receives within `window`, if any.
async fn self_wake_within(
    wakes: &mut tokio::sync::mpsc::Receiver<ClientMessage>,
    window: std::time::Duration,
) -> Option<u64> {
    match tokio::time::timeout(window, wakes.recv()).await {
        Ok(Some(ClientMessage::SwarmWake { generation })) => Some(generation),
        _ => None,
    }
}

/// A probe the locked store refuses defers the wake and wakes again after
/// the delay; that wake re-arms the member and runs its continuation.
#[tokio::test(start_paused = true)]
async fn a_probe_a_locked_store_refuses_wakes_again_after_the_delay() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, LockedOnce::new(1, 0));
    ctx.turn_control.queue_swarm_wake(3);
    handle_wake(&mut ctx, 3).await;
    assert!(!ctx.session.automatic_turns_allowed(), "still suspended");
    let early = RECOVERABLE_SELF_WAKE_DELAY - std::time::Duration::from_secs(1);
    assert_eq!(self_wake_within(&mut wakes, early).await, None);
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY)
        .await
        .expect("a self-wake after the delay");
    handle_wake(&mut ctx, generation).await;
    assert_eq!(user_prompts(&ctx), [RECOVERED_WAKE]);
}

/// A re-arm whose wake claim the locked store refuses keeps its owed turn
/// and wakes again after the delay to take it.
#[tokio::test(start_paused = true)]
async fn an_owed_turn_a_locked_claim_held_back_is_taken_after_the_delay() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, LockedOnce::new(0, 1));
    ctx.turn_control.queue_swarm_wake(3);
    handle_wake(&mut ctx, 3).await;
    assert!(
        ctx.session.automatic_turns_allowed(),
        "re-armed by the probe"
    );
    assert!(user_prompts(&ctx).is_empty(), "the claim was locked");
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY * 2)
        .await
        .expect("a self-wake after the delay");
    handle_wake(&mut ctx, generation).await;
    assert_eq!(user_prompts(&ctx), [RECOVERED_WAKE]);
}

/// A self-wake scheduled for one suspension is dropped once a newer one
/// began (it would re-arm the newer one at once, without its delay), and
/// the newer suspension's self-wake, which the older one's pending slot
/// kept from being scheduled when it was dated, is scheduled then
/// (#2626 review round 3, #2531).
#[tokio::test(start_paused = true)]
async fn a_stale_self_wake_hands_over_to_the_newer_suspension() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, Arc::new(Claimed));
    let older = ctx.session.automatic_turns().suspension_epoch();
    assert!(
        ctx.turn_control
            .schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, older)
    );
    // A newer suspension, dated as production dates it, while the older
    // self-wake is still pending: its own self-wake cannot be scheduled.
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    date_provider_suspension(&mut ctx).await;
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY * 2)
        .await
        .expect("the older self-wake fires");
    handle_wake(&mut ctx, generation).await;
    assert!(
        user_prompts(&ctx).is_empty(),
        "the stale wake re-armed nothing"
    );
    assert!(!ctx.session.automatic_turns_allowed());
    let early = RECOVERABLE_SELF_WAKE_DELAY - std::time::Duration::from_secs(1);
    assert_eq!(
        self_wake_within(&mut wakes, early).await,
        None,
        "it keeps its delay"
    );
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY)
        .await
        .expect("the newer suspension's self-wake");
    handle_wake(&mut ctx, generation).await;
    assert_eq!(user_prompts(&ctx), [RECOVERED_WAKE]);
}

/// A stale self-wake keeps a deferred outside wake's generation for the
/// next wake rather than losing it.
#[tokio::test(start_paused = true)]
async fn a_stale_self_wake_keeps_a_deferred_wake() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, _wakes) = suspended_over(&mut env, Arc::new(Claimed));
    let older = ctx.session.automatic_turns().suspension_epoch();
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    ctx.turn_control.defer_swarm_wake(41);
    assert!(
        ctx.turn_control
            .schedule_self_wake(std::time::Duration::ZERO, older)
    );
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    handle_wake(&mut ctx, 0).await;
    assert_eq!(ctx.turn_control.take_swarm_wake(0), 41, "kept");
}

/// A durable probe error (not contention) schedules no further self-wake.
#[tokio::test(start_paused = true)]
async fn a_durable_probe_error_schedules_no_self_wake() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(
        &mut env,
        Arc::new(super::super::Answer(Err("run not found"))),
    );
    ctx.turn_control.queue_swarm_wake(3);
    handle_wake(&mut ctx, 3).await;
    assert!(!ctx.session.automatic_turns_allowed());
    assert_eq!(
        self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY * 2).await,
        None
    );
}

/// #2626 review round 4: a self-wake for an older suspension that fires
/// once the member takes turns again is claimed like any other wake, so the
/// outside wake it carries (deferred by a locked store) runs its change
/// wake, never dropped.
#[tokio::test(start_paused = true)]
async fn a_stale_self_wake_while_allowed_claims_the_deferred_change_wake() {
    use crate::domain::inference::services::turn_suspension::{Instruction, Work};
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, Arc::new(Waking));
    let older = ctx.session.automatic_turns().suspension_epoch();
    assert!(
        ctx.turn_control
            .schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, older)
    );
    // A locked store deferred an outside wake; a newer suspension came and
    // an instruction re-armed it before the older self-wake fired.
    ctx.turn_control.defer_swarm_wake(5);
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    ctx.session
        .automatic_turns()
        .start_turn(Work::Instruction(Instruction::Prompt), None);
    assert!(ctx.session.automatic_turns_allowed());
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY * 2)
        .await
        .expect("the older self-wake fires");
    handle_wake(&mut ctx, generation).await;
    assert_eq!(
        user_prompts(&ctx),
        [crate::interface::uds::controllers::uds_swarm_control::CHANGED_WAKE]
    );
}

/// A stale self-wake that hands over says so on the event stream.
#[tokio::test(start_paused = true)]
async fn a_hand_over_is_reported() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, Arc::new(Claimed));
    let (sender, mut events) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(sender);
    let older = ctx.session.automatic_turns().suspension_epoch();
    assert!(
        ctx.turn_control
            .schedule_self_wake(std::time::Duration::ZERO, older)
    );
    ctx.session
        .automatic_turns()
        .suspend_after_failure(&unavailable());
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY)
        .await
        .expect("the older self-wake");
    handle_wake(&mut ctx, generation).await;
    let mut statuses = Vec::new();
    while let Ok(line) = events.try_recv() {
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        statuses.push(event["data"]["status"].clone());
    }
    assert!(
        statuses.contains(&serde_json::json!("handed_over")),
        "{statuses:?}"
    );
}

/// #2626 review round 5: a stale self-wake whose newer suspension takes no
/// self-wake (a terminal failure) schedules nothing and so reports no
/// hand-over.
#[tokio::test(start_paused = true)]
async fn a_stale_self_wake_with_nothing_to_hand_over_reports_none() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, Arc::new(Claimed));
    let (sender, mut events) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(sender);
    let older = ctx.session.automatic_turns().suspension_epoch();
    assert!(
        ctx.turn_control
            .schedule_self_wake(std::time::Duration::ZERO, older)
    );
    ctx.session.automatic_turns().suspend_after_failure(
        &crate::domain::shared::error::DomainError::Provider(
            "provider error (401): invalid credentials".into(),
        ),
    );
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY)
        .await
        .expect("the older self-wake");
    handle_wake(&mut ctx, generation).await;
    let mut statuses = Vec::new();
    while let Ok(line) = events.try_recv() {
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        statuses.push(event["data"]["status"].clone());
    }
    assert!(
        !statuses.contains(&serde_json::json!("handed_over")),
        "{statuses:?}"
    );
    assert_eq!(
        self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY * 2).await,
        None,
        "nothing was scheduled"
    );
}

/// A running run at a control generation the test moves; a wake claims no
/// generation.
struct Moving(std::sync::atomic::AtomicU64);

impl SwarmRunControl for Moving {
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
        let generation = self.0.load(Ordering::SeqCst);
        Box::pin(async move {
            Ok(RunControlReceipt {
                generation,
                ..running(false)
            })
        })
    }
}

/// #2626 review round 6 (#2531): an outside or resume wake that joins a
/// pending self-wake on a full channel makes it an outside wake: after a
/// newer (terminal) suspension it is not dropped as stale, so its probe
/// sees the resume and re-arms the member.
#[tokio::test(start_paused = true)]
async fn a_resume_joining_a_pending_self_wake_re_arms_a_newer_suspension() {
    let mut env = Env::with_unselected_workflow();
    let run = Arc::new(Moving(std::sync::atomic::AtomicU64::new(3)));
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, run.clone());
    let older = ctx.session.automatic_turns().suspension_epoch();
    assert!(
        ctx.turn_control
            .schedule_self_wake(std::time::Duration::ZERO, older)
    );
    let generation = self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY)
        .await
        .expect("the self-wake message is pending");
    // A newer, terminal suspension, dated at generation 3.
    ctx.session.automatic_turns().suspend_after_failure(
        &crate::domain::shared::error::DomainError::Provider(
            "provider error (401): invalid credentials".into(),
        ),
    );
    date_provider_suspension(&mut ctx).await;
    // The run is resumed (generation 7) and the resume's own wake joins
    // the pending message on a full channel.
    run.0.store(7, Ordering::SeqCst);
    assert!(ctx.turn_control.join_pending_swarm_wake(7));
    handle_wake(&mut ctx, generation).await;
    assert!(
        ctx.session.automatic_turns_allowed(),
        "re-armed by the resume"
    );
    assert_eq!(
        user_prompts(&ctx),
        [crate::interface::uds::controllers::uds_swarm_control::RESUMED_WAKE]
    );
}

/// #2626 verify round 7 (#2531), the mirror of round 6: a self-wake that
/// joins a pending outside wake leaves it an outside wake, never stamping
/// its own epoch on it. After a newer (terminal) suspension the wake is not
/// dropped as stale, so its probe sees the resume and re-arms the member.
#[tokio::test(start_paused = true)]
async fn a_self_wake_joining_a_pending_outside_wake_keeps_it_an_outside_wake() {
    let mut env = Env::with_unselected_workflow();
    let run = Arc::new(Moving(std::sync::atomic::AtomicU64::new(3)));
    let (mut ctx, _waker, mut wakes) = suspended_over(&mut env, run.clone());
    let older = ctx.session.automatic_turns().suspension_epoch();
    // An outside wake is pending; the older self-wake fires and joins it
    // (it sends no message of its own).
    assert!(ctx.turn_control.queue_swarm_wake(3));
    assert!(
        ctx.turn_control
            .schedule_self_wake(std::time::Duration::ZERO, older)
    );
    assert_eq!(
        self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY).await,
        None,
        "it joined the pending message"
    );
    // A newer, terminal suspension dated at 3; then the run is resumed.
    ctx.session.automatic_turns().suspend_after_failure(
        &crate::domain::shared::error::DomainError::Provider(
            "provider error (401): invalid credentials".into(),
        ),
    );
    date_provider_suspension(&mut ctx).await;
    run.0.store(7, Ordering::SeqCst);
    handle_wake(&mut ctx, 3).await;
    assert!(
        ctx.session.automatic_turns_allowed(),
        "re-armed by the resume"
    );
    assert_eq!(
        user_prompts(&ctx),
        [crate::interface::uds::controllers::uds_swarm_control::RESUMED_WAKE]
    );
}

/// A paused run takes no turn: its wake leaves a recoverable failure
/// suspended, spending none of the bound, and says nothing re-armed.
#[tokio::test]
async fn a_paused_runs_wake_re_arms_no_recoverable_failure() {
    let mut env = Env::with_unselected_workflow();
    let (mut ctx, _waker, _wakes) = suspended_over(&mut env, Arc::new(PausedWaking));
    let (sender, mut events) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(sender);
    ctx.turn_control.queue_swarm_wake(9);
    handle_wake(&mut ctx, 9).await;
    assert!(!ctx.session.automatic_turns_allowed(), "still suspended");
    assert!(user_prompts(&ctx).is_empty());
    assert!(resumed_reasons(&mut events).is_empty());
}

/// The self-wake holds the loop's channel weakly: once the loop's own
/// senders are gone, its receiver ends though a self-wake is pending.
#[tokio::test(start_paused = true)]
async fn a_pending_self_wake_never_keeps_the_loop_alive() {
    let turn_control =
        Arc::new(crate::interface::uds::controllers::uds_cancel::TurnControl::default());
    let (waker, mut wakes) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    turn_control.bind_self_waker(waker.downgrade());
    assert!(turn_control.schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, 0));
    drop(waker);
    let ended = tokio::time::timeout(RECOVERABLE_SELF_WAKE_DELAY / 2, wakes.recv()).await;
    assert!(matches!(ended, Ok(None)), "the channel closed at once");
}

/// A process that is ending sends no self-wake.
#[tokio::test(start_paused = true)]
async fn an_ending_process_sends_no_self_wake() {
    let turn_control =
        Arc::new(crate::interface::uds::controllers::uds_cancel::TurnControl::default());
    let (waker, mut wakes) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    turn_control.bind_self_waker(waker.downgrade());
    turn_control.mark_shutting_down();
    assert!(turn_control.schedule_self_wake(RECOVERABLE_SELF_WAKE_DELAY, 0));
    assert_eq!(
        self_wake_within(&mut wakes, RECOVERABLE_SELF_WAKE_DELAY * 2).await,
        None
    );
    drop(waker);
}

/// Take one from `counter` unless it is already zero: whether one was taken.
/// A compare-exchange loop, not `fetch_update` (deprecated from Rust 1.99)
/// or `try_update` (stable from 1.95, above the clippy MSRV).
fn take_one(counter: &std::sync::atomic::AtomicUsize) -> bool {
    let mut left = counter.load(Ordering::SeqCst);
    loop {
        let Some(less) = left.checked_sub(1) else {
            return false;
        };
        match counter.compare_exchange_weak(left, less, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(now) => left = now,
        }
    }
}
