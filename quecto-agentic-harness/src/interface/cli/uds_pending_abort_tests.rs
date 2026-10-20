//! A failed explicit turn that a stale abort drain ends is dated after the
//! turn (moved out of `uds_pending_swarm_tests.rs` for its line budget).
use super::*;

#[tokio::test]
async fn a_stale_abort_drain_dates_a_failed_explicit_turn() {
    let mut env = Env::new(
        super::dispatch_test_env::make_workflow(),
        std::sync::Arc::new(FailingProvider),
    );
    let mut ctx = env.ctx();
    suspend_failed_at(&mut ctx, 4).await;
    super::pending::queue_prompt(
        &mut ctx,
        Some("f3"),
        "follow_up",
        "carry on".into(),
        crate::domain::inference::services::turn_suspension::Work::FOLLOW_UP,
    )
    .await;
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
            work: crate::domain::inference::services::turn_suspension::Work::Instruction(
                crate::domain::inference::services::turn_suspension::Instruction::Prompt,
            ),
        },
    )
    .await;
    assert_eq!(
        ctx.session.control_receipt_status("f3"),
        Some(crate::interface::uds::wire::protocol::ControlStatus::Failed),
        "the follow-up drained on the stale-abort path and failed"
    );
    assert!(!ctx.session.automatic_turns_allowed());
    assert!(
        !resumed_at(&mut ctx, 26).await,
        "dated after the failed drained turn (36), not at a generation seen before or during it"
    );
    assert!(resumed_at(&mut ctx, 37).await);
}

/// A control port whose generation rises by ten on every probe.
struct Rising(std::sync::atomic::AtomicU64);
impl SwarmRunControl for Rising {
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
                wake_allowed: false,
                status: RunStatus::Running,
                generation: self.0.fetch_add(10, std::sync::atomic::Ordering::SeqCst),
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
            })
        })
    }
}

/// Suspend as a failed turn does, dated at `generation` by the run, as the
/// dispatch loop dates it after the failure (#1721).
pub(super) async fn suspend_failed_at(ctx: &mut DispatchCtx<'_>, generation: u64) {
    let turns = ctx.session.automatic_turns();
    turns.suspend_after_failure();
    turns
        .date_suspension(Some(&super::RunningAt(generation)))
        .await;
}

/// Suspend as a durable store rejection does, at `generation`.
pub(super) fn suspend_rejected_at(ctx: &mut DispatchCtx<'_>, generation: u64) {
    let turns = ctx.session.automatic_turns();
    turns.observe_control_generation(Some(generation));
    turns.suspend_after_store_rejection();
}

/// Whether a wake at `generation` re-arms the suspended session by a resume.
pub(super) async fn resumed_at(ctx: &mut DispatchCtx<'_>, generation: u64) -> bool {
    use crate::application::agent_turn::use_cases::resume_after_provider_failure::SuspendedProbe;
    let probe = ctx
        .session
        .automatic_turns()
        .probe_while_suspended(&super::RunningAt(generation))
        .await;
    matches!(probe, Ok(SuspendedProbe::Rearmed { .. }))
}

fn prompt(text: &str) -> super::PromptCommand {
    use crate::domain::inference::services::turn_suspension::{Instruction, Work};
    super::PromptCommand {
        id: None,
        type_name: "prompt".into(),
        message: text.into(),
        streaming_behavior: None,
        work: Work::Instruction(Instruction::Prompt),
    }
}

/// #2618 review: a client's prompt re-arms a suspended session, and a
/// failure in its turn is dated at the control generation known when the
/// prompt arrived, not at the old suspension's.
#[tokio::test]
async fn a_prompt_re_arms_and_dates_a_failure_in_its_turn() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    suspend_failed_at(&mut ctx, 1).await;
    super::handle_prompt(&mut ctx, prompt("hello")).await;
    assert!(
        ctx.session.automatic_turns_allowed(),
        "the prompt re-armed it"
    );

    let mut env = Env::new(
        super::dispatch_test_env::make_workflow(),
        std::sync::Arc::new(FailingProvider),
    );
    let mut ctx = env.ctx();
    suspend_failed_at(&mut ctx, 1).await;
    // The session alone knows an older generation: only the one the prompt
    // hands the turn dates its failure at 5 (#2618 review).
    ctx.session
        .automatic_turns()
        .observe_control_generation(Some(2));
    ctx.turn_control.observe_control_generation(5);
    super::handle_prompt(&mut ctx, prompt("hello")).await;
    assert!(!ctx.session.automatic_turns_allowed(), "the turn failed");
    assert!(!resumed_at(&mut ctx, 5).await, "dated at 5, the prompt's");
    assert!(resumed_at(&mut ctx, 6).await);
}
