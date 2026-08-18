//! Tests for the no-progress tolerance of the workflow auto-continue loop in
//! `drain_pending_and_nudge`.
//!
//! The old loop broke on the FIRST nudged turn that did not advance the
//! workflow fingerprint. Literal instruction-following models (e.g. GPT-5.6)
//! reply to a nudge with a bare status message and no tool calls, so a single
//! such turn silently killed auto-continue for the rest of the run. The loop
//! must now tolerate up to 2 consecutive no-progress turns (breaking on the
//! third), send a corrective nudge after a no-progress turn instead of the
//! verbatim repeat, and reset the streak whenever a turn advances progress.

use super::dispatch_test_env::{
    DispatchTestEnv, make_completed_feature_workflow, make_selected_feature_workflow,
};
use super::*;
use crate::domain::provider::{LlmProvider, StreamEvent};
use crate::interface::shared::WorkflowStateHandle;

/// Fragment of the standard auto-continue nudge (first nudge, and any nudge
/// after a turn that advanced the workflow).
const STANDARD_NUDGE_FRAGMENT: &str = "Workflow incomplete";
/// Fragments of the corrective nudge sent after a no-progress nudged turn.
const CORRECTIVE_FRAGMENT: &str = "did not advance the workflow";
const CORRECTIVE_NO_STATUS_FRAGMENT: &str = "Do not reply with only a status message";

/// Provider that follows a per-turn progress script (`true` = check the next
/// unchecked workflow step, `false` = make no progress) and records the last
/// user message of every request — i.e. the exact nudge text each nudged turn
/// was driven with. Turns beyond the script make no progress. With
/// `toggle_forever` the script is ignored and step 1 is alternately
/// checked/unchecked, so EVERY turn changes the progress fingerprint without
/// the workflow ever completing.
struct ScriptedProgressProvider {
    workflow: WorkflowStateHandle,
    script: Vec<bool>,
    toggle_forever: bool,
    calls: std::sync::atomic::AtomicU32,
    checked: std::sync::atomic::AtomicU32,
    seen_user_messages: std::sync::Mutex<Vec<String>>,
}

impl ScriptedProgressProvider {
    fn new(workflow: WorkflowStateHandle, script: Vec<bool>, toggle_forever: bool) -> Self {
        Self {
            workflow,
            script,
            toggle_forever,
            calls: std::sync::atomic::AtomicU32::new(0),
            checked: std::sync::atomic::AtomicU32::new(0),
            seen_user_messages: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl std::fmt::Debug for ScriptedProgressProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedProgressProvider").finish()
    }
}

#[tokio::test]
async fn scripted_progress_provider_trait_surface_methods_are_invoked() {
    let provider =
        ScriptedProgressProvider::new(make_selected_feature_workflow(), vec![false, false], false);
    assert_eq!(provider.name(), "scripted-progress");
    assert!(format!("{provider:?}").contains("ScriptedProgressProvider"));
    assert!(provider.as_any().is::<()>());

    let request = crate::domain::provider::ChatRequest {
        messages: &[],
        tools: &[],
        model: "test",
        max_tokens: 1,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    assert_eq!(
        provider
            .chat_stream(request)
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("status")
    );

    let request = crate::domain::provider::ChatRequest {
        messages: &[],
        tools: &[],
        model: "test",
        max_tokens: 1,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Done(resp) => assert_eq!(resp.content.as_deref(), Some("status")),
        other => panic!("unexpected stream event: {other:?}"),
    }
}

impl crate::domain::provider::LlmProvider for ScriptedProgressProvider {
    fn name(&self) -> &str {
        "scripted-progress"
    }

    fn chat(
        &self,
        request: crate::domain::provider::ChatRequest<'_>,
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
        if let Some(last_user) = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::domain::message::Role::User)
        {
            self.seen_user_messages
                .lock()
                .unwrap()
                .push(last_user.content.clone());
        }
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as usize;
        if self.toggle_forever {
            if let Ok(mut engine) = self.workflow.lock() {
                let _ = if n % 2 == 0 {
                    engine.check(1)
                } else {
                    engine.uncheck(1)
                };
            }
        } else if self.script.get(n).copied().unwrap_or(false) {
            let step = self
                .checked
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if let Ok(mut engine) = self.workflow.lock() {
                let _ = engine.check(step);
            }
        }
        Box::pin(async {
            Ok(crate::domain::message::LlmResponse {
                content: Some("status".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// Test env: the shared [`DispatchTestEnv`] plus a handle on the scripted
/// provider so tests can inspect calls/messages after the drain returns.
struct Env {
    inner: DispatchTestEnv,
    provider: std::sync::Arc<ScriptedProgressProvider>,
}

impl Env {
    /// Build an `Env` with a selected (incomplete) `feature` workflow and a
    /// scripted provider, so the auto-continue nudge fires at the idle drain.
    fn with_progress_script(script: Vec<bool>) -> Self {
        Self::build(script, false)
    }

    /// Build an `Env` whose provider changes the progress fingerprint EVERY
    /// turn (toggling step 1) without ever completing the workflow, so only
    /// the nudge cap can terminate the loop.
    fn with_toggling_progress() -> Self {
        Self::build(Vec::new(), true)
    }

    /// Build an `Env` whose workflow is already COMPLETE (every step checked),
    /// so only the completion nudge can fire at the idle drain.
    fn with_completed_workflow() -> Self {
        Self::around_workflow(make_completed_feature_workflow(), Vec::new(), false)
    }

    fn build(script: Vec<bool>, toggle_forever: bool) -> Self {
        Self::around_workflow(make_selected_feature_workflow(), script, toggle_forever)
    }

    fn around_workflow(
        workflow: WorkflowStateHandle,
        script: Vec<bool>,
        toggle_forever: bool,
    ) -> Self {
        let provider = std::sync::Arc::new(ScriptedProgressProvider::new(
            workflow.clone(),
            script,
            toggle_forever,
        ));
        Self {
            inner: DispatchTestEnv::new(workflow, provider.clone()),
            provider,
        }
    }

    fn ctx(&mut self) -> DispatchCtx<'_> {
        self.inner.ctx()
    }

    fn calls(&self) -> u32 {
        self.provider
            .calls
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn seen_user_messages(&self) -> Vec<String> {
        self.provider.seen_user_messages.lock().unwrap().clone()
    }
}

/// AC-c: a nudged turn that makes no progress triggers a CORRECTIVE nudge on
/// the next iteration instead of breaking the loop — the corrective text tells
/// the model to check the step off or keep working, not to status-reply.
#[tokio::test]
async fn no_progress_turn_sends_corrective_nudge_instead_of_breaking() {
    let mut env = Env::with_progress_script(vec![]); // never progresses
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    let msgs = env.seen_user_messages();
    assert!(
        msgs.len() >= 2,
        "a no-progress turn must be followed by a corrective nudge, not break the loop; got {} nudged turn(s): {msgs:?}",
        msgs.len()
    );
    assert!(
        msgs[0].contains(STANDARD_NUDGE_FRAGMENT),
        "first nudge is the standard auto-continue nudge: {:?}",
        msgs[0]
    );
    assert!(
        msgs[1].contains(CORRECTIVE_FRAGMENT),
        "second nudge after a no-progress turn must be corrective: {:?}",
        msgs[1]
    );
    assert!(
        msgs[1].contains(CORRECTIVE_NO_STATUS_FRAGMENT),
        "corrective nudge must forbid a bare status reply: {:?}",
        msgs[1]
    );
}

/// AC-d: THREE consecutive no-progress nudged turns break the loop — bounded
/// tolerance, well below MAX_WORKFLOW_NUDGES, so a model that truly ignores
/// the workflow is not nudged forever.
#[tokio::test]
async fn three_consecutive_no_progress_turns_break_the_loop() {
    let mut env = Env::with_progress_script(vec![]); // never progresses
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        3,
        "the loop must give up after exactly 3 consecutive no-progress turns"
    );
    // EVERY nudge whose previous nudged turn made no progress is corrective —
    // the second consecutive no-progress turn must not revert to the standard
    // wording.
    let msgs = env.seen_user_messages();
    assert!(
        msgs[2].contains(CORRECTIVE_FRAGMENT),
        "third nudge (after two no-progress turns) must still be corrective: {:?}",
        msgs[2]
    );
}

/// The MAX_WORKFLOW_NUDGES bound stays: with the 3-strike tolerance, the cap
/// is the only termination guard against a model that keeps changing the
/// fingerprint without ever finishing (here: toggling one step forever, so
/// the no-progress streak never trips and the workflow never completes).
#[tokio::test]
async fn nudge_loop_stops_at_the_max_workflow_nudges_cap() {
    let mut env = Env::with_toggling_progress();
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls() as usize,
        super::MAX_WORKFLOW_NUDGES,
        "a fingerprint-toggling model must be stopped by the nudge cap"
    );
}

/// AC-e: a turn that advances the workflow RESETS the consecutive no-progress
/// counter, and the nudge after a progress turn is the standard one again.
#[tokio::test]
async fn progress_resets_the_no_progress_counter() {
    // Turns: no, no, progress (reset), no, no, no (3rd consecutive → break).
    let mut env = Env::with_progress_script(vec![false, false, true]);
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        6,
        "progress on turn 3 must reset the streak, allowing a fresh tolerance window"
    );
    let msgs = env.seen_user_messages();
    assert!(
        msgs[3].contains(STANDARD_NUDGE_FRAGMENT),
        "after a progress turn the nudge returns to the standard wording: {:?}",
        msgs[3]
    );
    assert!(
        !msgs[3].contains(CORRECTIVE_FRAGMENT),
        "after a progress turn the corrective wording must not be reused: {:?}",
        msgs[3]
    );
    assert!(
        msgs[4].contains(CORRECTIVE_FRAGMENT),
        "a post-reset no-progress turn is followed by the corrective nudge again: {:?}",
        msgs[4]
    );
}

/// The completion nudge is single-shot: it asks for a final report and a stop,
/// which never advances the fingerprint, so the loop must send it exactly once
/// and break — never retry it with the corrective wording.
#[tokio::test]
async fn completion_nudge_is_sent_exactly_once() {
    let mut env = Env::with_completed_workflow();
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        1,
        "a complete workflow gets exactly one completion nudge despite the unchanged fingerprint"
    );
    let msgs = env.seen_user_messages();
    assert!(
        msgs[0].contains("report your result and stop"),
        "the single nudged turn must carry the completion wording: {:?}",
        msgs[0]
    );
}

/// Progress made by an UNRELATED turn — here a buffered sub-agent completion
/// note that arrives during a nudged turn and drains at the next idle boundary
/// — must not be attributed to the nudge: the note runs OUTSIDE the measured
/// fingerprint window, so it neither resets the no-progress streak nor flips
/// the next nudge back to the standard wording.
#[tokio::test]
async fn unrelated_pending_turn_progress_is_not_attributed_to_the_nudge() {
    // Provider script by call index: call 0 is the first nudged turn (no
    // progress), call 1 is the drained sub-agent note's turn (advances the
    // workflow), calls 2-3 are the remaining nudged turns (no progress).
    let mut env = Env::with_progress_script(vec![false, true, false, false]);
    let (tx, rx) = crate::infrastructure::tools::subagent_registry::new_notification_channel();
    tx.try_send(
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification::new(
            1,
            crate::infrastructure::tools::subagent_registry::SubagentNotification::Completed {
                agent_id: "child-1".into(),
            },
        ),
    )
    .unwrap();
    env.inner.notification_rx = Some(rx);
    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    // Turns: nudge (no progress, streak 1), note (progress, unmeasured),
    // corrective nudge (streak 2), corrective nudge (streak 3 → break).
    assert_eq!(
        env.calls(),
        4,
        "the note's progress must not reset the streak or extend the tolerance"
    );
    // seen_user_messages records the LAST user message per provider call, so
    // the note's system-message turn (call 1) re-records the first nudge.
    let msgs = env.seen_user_messages();
    assert!(
        msgs[2].contains(CORRECTIVE_FRAGMENT),
        "the nudge after the stalled nudged turn stays corrective despite the note's progress: {:?}",
        msgs[2]
    );
    assert!(
        msgs[3].contains(CORRECTIVE_FRAGMENT),
        "the final nudge is still corrective: {:?}",
        msgs[3]
    );
}

#[tokio::test]
#[serial_test::serial(workflow_nudge_injection_hook)]
async fn selected_nudge_is_cancelled_when_direct_child_becomes_active_before_injection() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };

    let mut env = Env::with_progress_script(vec![]);
    let reg = new_registry();
    env.inner.subagent_registry = Some(reg.clone());
    super::set_before_workflow_nudge_injection_test_hook(Box::new(move || {
        let mut child = SubagentEntry::new("/tmp/racing-child.sock".into(), 1);
        child.status = SubagentStatus::Starting;
        child.parent_id = Some("test".to_string());
        reg.lock()
            .unwrap()
            .insert("racing-child".to_string(), child);
    }));

    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        0,
        "a selected nudge must be revalidated immediately before injection"
    );
}

#[tokio::test]
#[serial_test::serial(workflow_nudge_injection_hook)]
async fn selected_nudge_is_cancelled_when_transitive_descendant_becomes_active_before_injection() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };

    let mut env = Env::with_progress_script(vec![]);
    let reg = new_registry();
    env.inner.subagent_registry = Some(reg.clone());
    super::set_before_workflow_nudge_injection_test_hook(Box::new(move || {
        let mut child = SubagentEntry::new("/tmp/idle-child.sock".into(), 1);
        child.status = SubagentStatus::Idle;
        child.parent_id = Some("test".to_string());
        let mut grandchild = SubagentEntry::new("/tmp/racing-grandchild.sock".into(), 2);
        grandchild.status = SubagentStatus::Running;
        grandchild.parent_id = Some("child".to_string());
        let mut guard = reg.lock().unwrap();
        guard.insert("child".to_string(), child);
        guard.insert("grandchild".to_string(), grandchild);
    }));

    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        0,
        "transitive descendant activity must cancel the selected nudge"
    );
}

#[tokio::test]
#[serial_test::serial(workflow_nudge_injection_hook)]
async fn selected_nudge_runs_when_unrelated_child_becomes_active_before_injection() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };

    let mut env = Env::with_progress_script(vec![]);
    let reg = new_registry();
    env.inner.subagent_registry = Some(reg.clone());
    super::set_before_workflow_nudge_injection_test_hook(Box::new(move || {
        let mut unrelated = SubagentEntry::new("/tmp/unrelated-racing-child.sock".into(), 1);
        unrelated.status = SubagentStatus::Running;
        unrelated.parent_id = Some("other-session".to_string());
        reg.lock()
            .unwrap()
            .insert("unrelated".to_string(), unrelated);
    }));

    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        3,
        "unrelated activity must not suppress workflow nudges"
    );
}

#[tokio::test]
#[serial_test::serial(workflow_nudge_injection_hook)]
async fn selected_nudge_is_cancelled_when_child_becomes_active_after_final_recheck_before_turn_admission()
 {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };

    let mut env = Env::with_progress_script(vec![]);
    let reg = new_registry();
    env.inner.subagent_registry = Some(reg.clone());
    super::set_before_guarded_turn_admission_test_hook(Box::new(move || {
        let mut child = SubagentEntry::new("/tmp/post-recheck-child.sock".into(), 1);
        child.status = SubagentStatus::Starting;
        child.parent_id = Some("test".to_string());
        reg.lock()
            .unwrap()
            .insert("post-recheck-child".to_string(), child);
    }));

    {
        let mut ctx = env.ctx();
        super::drain_pending_and_nudge(&mut ctx).await;
    }

    assert_eq!(
        env.calls(),
        0,
        "descendant activity after the final snapshot recheck but before turn admission must cancel the nudge"
    );
}
