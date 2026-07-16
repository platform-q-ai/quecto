//! Tests for the "explicit control beats automated nudges" semantics
//! (#895 abort = full stop, #896 steer outranks the auto-continue nudge).
//!
//! These exercise the dispatch-level seam: the reader task sets
//! [`TurnControl`] flags before the matching command is dispatched, and the
//! post-turn idle drain (`drain_pending_and_nudge`) must honour them.

use super::dispatch_test_env::{DispatchTestEnv as Env, make_dispatch_test_agent};
use crate::interface::cli::protocol::{AgentCommand, StreamingBehavior};
use crate::interface::cli::uds_session::PendingMessage;
use crate::interface::shared::WorkflowStateHandle;

// ─── #895: abort = full stop ────────────────────────────────────────────────

/// With no abort/steer pending, the idle drain runs the workflow auto-continue
/// nudge — establishes that the suppression tests below actually suppress
/// something (AC: no regression to workflow progress when nothing is pending).
#[tokio::test]
async fn baseline_idle_drain_runs_auto_continue_nudge() {
    let mut env = Env::with_selected_feature();
    let mut ctx = env.ctx();
    super::drain_pending_and_nudge(&mut ctx).await;
    assert!(
        !ctx.messages.is_empty(),
        "auto-continue nudge should have driven at least one turn"
    );
}

/// A pending abort suppresses the workflow auto-continue nudge AND discards any
/// queued work, so the bound workflow does not resume (#895 AC1/AC2).
#[tokio::test]
async fn abort_suppresses_nudge_and_discards_pending() {
    let mut env = Env::with_selected_feature();
    env.session.enqueue_pending("queued follow-up work".into());
    env.turn_control.mark_abort();

    let mut ctx = env.ctx();
    super::drain_pending_and_nudge(&mut ctx).await;

    assert!(
        ctx.messages.is_empty(),
        "abort must suppress the nudge and not run queued work"
    );
    assert!(
        ctx.session.drain_pending().is_empty(),
        "abort must discard queued pending work"
    );
    assert!(
        !ctx.turn_control.is_abort_pending(),
        "abort flag is consumed by the idle drain"
    );
}

/// Ack honesty (#1105): a mid-turn follow_up must be retained in the pending
/// queue; `success:true` must not be emitted for work that vanished.
#[tokio::test]
async fn mid_turn_follow_up_is_queued() {
    let mut env = Env::with_unselected_workflow();
    env.session.set_streaming(true);
    {
        let mut ctx = env.ctx();
        super::uds_dispatch::handle_follow_up(
            &mut ctx,
            Some("f"),
            "follow_up",
            "say followed".into(),
        )
        .await;
    }

    let pending = env.session.drain_pending();
    assert!(
        matches!(pending.as_slice(), [PendingMessage::User(msg)] if msg == "say followed"),
        "follow_up success must leave an observable queued message: {pending:?}"
    );
}

/// Ack honesty (#1105): a prompt with `streamingBehavior=steer` received while
/// busy is the transport shape used by parents for steering; it must be queued
/// just like a direct steer instead of being silently acknowledged and lost.
#[tokio::test]
async fn busy_prompt_with_steer_behavior_is_queued() {
    let mut env = Env::with_unselected_workflow();
    env.session.set_streaming(true);
    {
        let mut ctx = env.ctx();
        super::uds_dispatch::dispatch_command(
            AgentCommand::Prompt {
                id: Some("s".into()),
                message: "say steered".into(),
                streaming_behavior: Some(StreamingBehavior::Steer),
            },
            &mut ctx,
        )
        .await;
    }

    let pending = env.session.drain_pending();
    assert!(
        matches!(pending.as_slice(), [PendingMessage::User(msg)] if msg == "say steered"),
        "steer success must leave an observable queued message: {pending:?}"
    );
}

/// `handle_abort` clears queued work and both control flags so a later idle
/// drain cannot re-drive the agent (#895 AC2).
#[tokio::test]
async fn handle_abort_clears_pending_and_flags() {
    let mut env = Env::with_selected_feature();
    env.session.enqueue_pending("queued".into());
    env.turn_control.mark_abort();
    env.turn_control.mark_steer();

    let mut ctx = env.ctx();
    super::uds_dispatch::handle_abort(&mut ctx, Some("a"), "abort").await;

    assert!(ctx.session.drain_pending().is_empty());
    assert!(!ctx.turn_control.is_abort_pending());
    assert!(!ctx.turn_control.is_steer_pending());
}

// ─── #896: steer outranks the auto-continue nudge ───────────────────────────

/// While a steer is pending, the idle drain yields — the auto-continue nudge
/// must NOT run ahead of the steer (#896 AC1).
#[tokio::test]
async fn pending_steer_yields_the_auto_continue_nudge() {
    let mut env = Env::with_selected_feature();
    env.turn_control.mark_steer();

    let mut ctx = env.ctx();
    super::drain_pending_and_nudge(&mut ctx).await;

    assert!(
        ctx.messages.is_empty(),
        "auto-continue nudge must yield to a pending steer"
    );
    assert!(
        ctx.turn_control.is_steer_pending(),
        "the steer gate stays set until its own handler runs"
    );
}

/// Deterministic unique-marker test (#896 AC2): a steer issued while a turn was
/// mid-flight (gate set, queue empty at the idle drain) is obeyed next — the
/// marker instruction runs — instead of being overridden by the workflow nudge.
#[tokio::test]
async fn steer_marker_is_obeyed_after_mid_turn_cancel() {
    const MARKER: &str = "STEER-MARKER-7f3a9c21-unique";
    let mut env = Env::with_selected_feature();

    // Reader saw the steer and set the gate; the in-flight turn's idle drain
    // runs first and must NOT advance the workflow ahead of the steer.
    env.turn_control.mark_steer();
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
        assert!(
            ctx.messages.is_empty(),
            "workflow must not run ahead of the queued steer"
        );
    }

    // Now the steer command is dispatched (agent idle after the cancel unwind).
    {
        let mut ctx = env.ctx();
        super::uds_dispatch::handle_steer(&mut ctx, Some("s"), "steer", MARKER.to_string()).await;
    }

    let obeyed = env
        .messages
        .iter()
        .any(|m| m.role == crate::domain::message::Role::User && m.content == MARKER);
    assert!(obeyed, "the unique steer marker instruction must be obeyed");
    assert!(
        !env.turn_control.is_steer_pending(),
        "handling the steer releases the gate"
    );
}

/// Regression for the stuck-steer-gate hazard: the reader marks `steer_pending`
/// from a loose `"type":"steer"` substring match, so a non-steer command whose
/// body merely quotes the protocol can leave the gate stuck `true`. A genuine
/// `follow_up` must clear it so the auto-continue nudge is not permanently
/// suppressed (#896 AC3 — no regression to workflow progress).
#[tokio::test]
async fn follow_up_clears_stuck_steer_gate() {
    let mut env = Env::with_selected_feature();
    // Simulate a false-positive substring classification leaving the gate set.
    env.turn_control.mark_steer();

    let mut ctx = env.ctx();
    super::uds_dispatch::handle_follow_up(&mut ctx, Some("f"), "follow_up", "work".into()).await;

    assert!(
        !ctx.turn_control.is_steer_pending(),
        "a genuine follow_up must release a stale steer gate"
    );
    assert!(
        !ctx.messages.is_empty(),
        "with the gate cleared the follow_up drains and the nudge runs"
    );
}

/// The reader-side classifier and the drain seam are wired together end-to-end
/// for abort (#895 AC4): a real `"type":"abort"` line classifies, sets the flag
/// via `mark_abort`, and the subsequent idle drain suppresses the nudge and
/// discards queued work — defending the wiring, not just hand-set flags.
#[tokio::test]
async fn abort_command_line_classified_then_suppresses_nudge() {
    let abort_line = r#"{"type":"abort"}"#;
    assert!(
        super::is_abort_command(abort_line),
        "abort line must classify as abort"
    );
    assert!(
        !super::is_steer_command(abort_line),
        "abort line must not classify as steer"
    );

    let mut env = Env::with_selected_feature();
    env.session.enqueue_pending("queued".into());
    // Drive the exact reader path: classify → mark the control flag.
    if super::is_abort_command(abort_line) {
        env.turn_control.mark_abort();
    } else if super::is_steer_command(abort_line) {
        env.turn_control.mark_steer();
    }

    let mut ctx = env.ctx();
    super::drain_pending_and_nudge(&mut ctx).await;

    assert!(
        ctx.messages.is_empty(),
        "classified abort must suppress the auto-continue nudge"
    );
    assert!(
        ctx.session.drain_pending().is_empty(),
        "classified abort must discard queued work"
    );
}

/// Abort beats steer at the drain seam (#895): with both flags pending, the
/// drain takes the abort path (full stop, queue discarded) rather than merely
/// yielding to the steer.
#[tokio::test]
async fn abort_beats_steer_at_drain_seam() {
    let mut env = Env::with_selected_feature();
    env.session.enqueue_pending("queued".into());
    env.turn_control.mark_abort();
    env.turn_control.mark_steer();

    let mut ctx = env.ctx();
    super::drain_pending_and_nudge(&mut ctx).await;

    assert!(ctx.messages.is_empty(), "abort path must win over steer");
    assert!(
        ctx.session.drain_pending().is_empty(),
        "abort discards queued work even when a steer is also pending"
    );
    assert!(!ctx.turn_control.is_abort_pending(), "abort flag consumed");
}

// ─── #930: abort stops an already-running auto-continue loop ──────────────────

/// Provider that advances the bound workflow one step per turn (so the
/// auto-continue nudge loop keeps iterating) and fires a full-stop abort on its
/// SECOND turn — simulating an abort that lands WHILE the loop is mid-flight,
/// which the idle-drain entry guard cannot catch.
struct AdvanceThenAbortProvider {
    workflow: WorkflowStateHandle,
    turn_control: crate::interface::cli::uds_cancel::TurnControlHandle,
    calls: std::sync::atomic::AtomicU32,
}

impl std::fmt::Debug for AdvanceThenAbortProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdvanceThenAbortProvider").finish()
    }
}

impl crate::domain::provider::LlmProvider for AdvanceThenAbortProvider {
    fn name(&self) -> &str {
        "advance-then-abort"
    }

    fn chat(
        &self,
        _request: crate::domain::provider::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        // 0-based turn index. Advance one workflow step so the loop's progress
        // fingerprint changes and it would otherwise keep nudging.
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut engine) = self.workflow.lock() {
            let _ = engine.check(n + 1);
        }
        // On the second turn an abort lands mid-loop (full stop).
        if n == 1 {
            self.turn_control.mark_abort();
        }
        Box::pin(async {
            Ok(crate::domain::message::LlmResponse {
                content: Some("step".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// #930 regression: an abort fired WHILE the workflow is auto-continuing must
/// stop it. Before the fix the abort flag was only checked at the idle-drain
/// entry, not inside the nudge loop, so the workflow advanced past the abort
/// (e.g. 5/17 → 9/17). Here the provider advances one step per turn and aborts on
/// its second turn; without the in-loop check the loop would run to 17/17.
#[tokio::test]
async fn abort_stops_an_already_running_auto_continue_loop() {
    let mut env = Env::with_selected_feature();
    env.agent = make_dispatch_test_agent(std::sync::Arc::new(AdvanceThenAbortProvider {
        workflow: env.workflow.clone(),
        turn_control: env.turn_control.clone(),
        calls: std::sync::atomic::AtomicU32::new(0),
    }));

    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    let progress = env.workflow.lock().unwrap().progress();
    assert!(
        progress.done < progress.total,
        "abort must stop the auto-continue loop, but it ran to {}/{}",
        progress.done,
        progress.total
    );
    assert!(
        progress.done <= 3,
        "loop should stop right after the mid-flight abort, got {}/{}",
        progress.done,
        progress.total
    );
    assert!(
        !env.turn_control.is_abort_pending(),
        "the mid-loop abort flag is consumed by the full stop"
    );
}
