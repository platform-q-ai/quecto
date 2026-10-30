//! #2762 review round 1: the guard's queued-work, correction and swarm
//! cases on the real UDS dispatch path, over the #2708 rig.
use super::issue_2708_uds_parent_tests::{PROMISE_2648, Reply, Root, of_type, position};
use crate::domain::agents::value_objects::turn_outcome::LIVE_REASON;
use crate::interface::cli::uds_session::PendingMessage;
use serde_json::json;

type Snapshot = crate::interface::cli::uds_snapshots::StateSnapshot;

/// A connection that, as each `turn_outcome` frame is written to it, reads
/// the state snapshot at that very moment and records whether it already
/// carried that outcome as the latest.
struct OutcomeWitness {
    snapshot: Snapshot,
    pending: Vec<u8>,
    seen: Vec<bool>,
}

impl tokio::io::AsyncWrite for OutcomeWitness {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let Ok(frame) = serde_json::from_slice::<serde_json::Value>(&line) else {
                continue;
            };
            if frame["type"] == "turn_outcome" {
                let snapshot = self.snapshot.clone();
                let state = snapshot.try_read().expect("no refresh is mid-write");
                let state =
                    crate::interface::cli::uds_state_projection::slim_state_projection(&state);
                let carried = state["latestTurnOutcome"]["sequence"] == frame["sequence"];
                self.seen.push(carried);
            }
        }
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// Review #2762 round 3 (3a): the state snapshot carries a published
/// outcome before its `turn_outcome` frame goes out, so a client reacting to
/// outcome N by asking `get_state` mid-turn is answered with latest N.
#[tokio::test]
async fn review_2762_the_snapshot_carries_an_outcome_before_its_frame() {
    let mut root = Root::new(vec![
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Merged"}),
        ),
        Reply::Text("All merged."),
    ]);
    let state = root
        .env
        .session
        .state_snapshot("cli:test", 0, None, 0, None);
    let snapshot: Snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(state));
    let mut witness = OutcomeWitness {
        snapshot: snapshot.clone(),
        pending: Vec::new(),
        seen: Vec::new(),
    };
    let command = crate::interface::uds::wire::protocol::parse_command_line(
        r#"{"type":"prompt","id":"ep-1","message":"Merge it"}"#,
    )
    .unwrap();
    {
        let mut ctx = root.env.ctx();
        ctx.broadcast_tx = None;
        ctx.stdout = Some(&mut witness);
        ctx.state_snapshot = snapshot;
        super::super::dispatch_command(command, &mut ctx).await;
    }
    assert_eq!(witness.seen, [true], "one outcome, already in the snapshot");
}

/// Review #2762 (4): the correction runs as its own turn; the prompt's
/// control receipt keeps the prompt's result, not the correction's.
#[tokio::test]
async fn review_2762_a_failed_correction_leaves_the_prompts_receipt_completed() {
    let mut root = Root::new(vec![Reply::Text(PROMISE_2648), Reply::Fail]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Carry on"}"#)
        .await;
    let _ = root.frames();
    root.send(r#"{"type":"get_state","id":"s1"}"#).await;
    let state = root.frames().remove(0);
    let receipts = state["data"]["controlReceipts"].as_array().unwrap().clone();
    let receipt = receipts
        .iter()
        .find(|receipt| receipt["id"] == "ep-1")
        .expect("the prompt's receipt");
    assert_eq!(receipt["status"], "completed", "{receipts:?}");
}

/// Review #2762 (4): the correction is saved before it runs, as any other
/// drained turn's opener is.
#[tokio::test]
async fn review_2762_the_correction_note_is_saved_before_its_turn_runs() {
    let mut root = Root::new(vec![Reply::Text(PROMISE_2648), Reply::Text(PROMISE_2648)]);
    *root.provider.probe.lock().unwrap() = Some(root.env.tmp.path().to_path_buf());
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Carry on"}"#)
        .await;
    assert_eq!(
        *root.provider.persisted.lock().unwrap(),
        vec![false, true],
        "the correction is on disk when its request is made"
    );
}

/// Review #2762 (9): a queued instruction alone is work: an undeclared turn
/// waits rather than being corrected.
#[tokio::test]
async fn review_2762_a_queued_instruction_alone_makes_an_undeclared_turn_wait() {
    let mut root = Root::new(vec![Reply::Text("Noted."), Reply::Text("Next.")]);
    let body = crate::interface::cli::uds_session::PromptBody {
        text: "Then this".into(),
        images: vec![],
    };
    let follow_up = crate::domain::inference::services::turn_suspension::Work::FOLLOW_UP;
    assert!(
        root.env
            .session
            .enqueue_control(Some("ep-2"), "follow_up", body, follow_up)
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"First"}"#)
        .await;
    let frames = root.frames();
    let first = of_type(&frames, "turn_outcome")[0].clone();
    assert_eq!(
        (&first["episodeId"], &first["outcome"], &first["reason"]),
        (&json!("ep-1"), &json!("waiting"), &json!(LIVE_REASON))
    );
    let requests = root.provider.requests();
    assert_eq!(
        requests[1], "Then this",
        "no correction before it: {requests:?}"
    );
}

/// Review #2762 (A): a swarm the parent launched is live while its idle
/// coordinator's last report holds work: the parent's wake turn waits.
#[tokio::test]
async fn review_2762_an_open_swarm_with_an_idle_coordinator_keeps_a_wake_turn_waiting() {
    let mut root = Root::new(vec![
        Reply::Text("Started the swarm."),
        Reply::Text("Noted."),
    ]);
    with_idle_coordinator(
        &root,
        Some(crate::domain::swarm::services::parent_wake::WakeKind::Hold),
    );
    root.env
        .session
        .restore_pending(std::iter::once(PendingMessage::Automatic(
            "swarm report".into(),
        )));
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Run the swarm"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 2, "{frames:?}");
    assert_eq!(outcomes[1]["outcome"], "waiting", "{frames:?}");
    assert_eq!(root.provider.requests().len(), 2, "no correction");
}

/// The parent's registry with one idle coordinator it launched, whose last
/// report was `kind` (`None`: it has not reported yet).
fn with_idle_coordinator(
    root: &Root,
    kind: Option<crate::domain::swarm::services::parent_wake::WakeKind>,
) {
    let registry = root.env.subagent_registry.clone().unwrap();
    let mut coordinator =
        crate::infrastructure::tools::agents::subagent_registry::SubagentEntry::new("/s".into(), 0);
    coordinator.status =
        crate::infrastructure::tools::agents::subagent_registry::SubagentStatus::Idle;
    coordinator.coordinator_wake.launched = true;
    if let Some(kind) = kind {
        coordinator.coordinator_wake.report_for_test(kind);
    }
    registry
        .lock()
        .unwrap()
        .insert("coordinator".into(), coordinator);
}

/// Review #2762 round 2 (A): once the coordinator reported its run finished,
/// an idle coordinator is no work: the undeclared turn is corrected, then
/// stalls, so Pocket settles instead of waiting forever.
#[tokio::test]
async fn review_2762_a_finished_wake_with_an_idle_coordinator_is_corrected_then_stalls() {
    let mut root = Root::new(vec![
        Reply::Text("Swarm finished."),
        Reply::Text("Swarm finished."),
    ]);
    with_idle_coordinator(
        &root,
        Some(crate::domain::swarm::services::parent_wake::WakeKind::Finished),
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Report"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "{frames:?}");
    assert_eq!(outcomes[0]["outcome"], "stalled");
    assert_eq!(root.provider.requests().len(), 2, "one correction");
}

/// Review #2762 round 3 (1): an idle coordinator that never reported (it
/// ended its first turn without a board, so it sends no `swarm_state`) is
/// no work: the undeclared turn is corrected, then stalls, rather than
/// waiting for ever.
#[tokio::test]
async fn review_2762_a_never_reporting_idle_coordinator_is_corrected_then_stalls() {
    let mut root = Root::new(vec![
        Reply::Text("Swarm started."),
        Reply::Text("Swarm started."),
    ]);
    with_idle_coordinator(&root, None);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Report"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "{frames:?}");
    assert_eq!(outcomes[0]["outcome"], "stalled");
    assert_eq!(root.provider.requests().len(), 2, "one correction");
}

/// Review #2762 round 4 (L5): an idle coordinator whose last report was
/// unknown may still have workers running: the parent is not corrected
/// (it could relaunch running work) but needs the user, with the diagnostic.
#[tokio::test]
async fn review_2762_an_unknown_coordinator_report_needs_the_user() {
    let mut root = Root::new(vec![Reply::Text("Swarm running.")]);
    with_idle_coordinator(
        &root,
        Some(crate::domain::swarm::services::parent_wake::WakeKind::Unknown),
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Report"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "{frames:?}");
    assert_eq!(outcomes[0]["outcome"], "needs_user");
    assert_eq!(
        outcomes[0]["reason"],
        crate::domain::agents::value_objects::turn_outcome::UNKNOWN_REASON
    );
    assert_eq!(root.provider.requests().len(), 1, "no correction");
}

/// Review #2762 round 2 (A): while its last report is a hold, the idle
/// coordinator's run is work: the parent waits.
#[tokio::test]
async fn review_2762_a_hold_wake_with_an_idle_coordinator_keeps_the_parent_waiting() {
    let mut root = Root::new(vec![Reply::Text("Swarm running.")]);
    with_idle_coordinator(
        &root,
        Some(crate::domain::swarm::services::parent_wake::WakeKind::Hold),
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Report"}"#)
        .await;
    let frames = root.frames();
    let outcomes = of_type(&frames, "turn_outcome");
    assert_eq!(outcomes.len(), 1, "{frames:?}");
    assert_eq!(outcomes[0]["outcome"], "waiting");
    assert_eq!(root.provider.requests().len(), 1, "no correction");
}

/// Review #2762 round 2 (LOW 1): off the guard the prompt runs exactly as on
/// master: its acknowledgement comes before the post-turn refresh's
/// `ledger_advanced`.
#[tokio::test]
async fn review_2762_off_path_acknowledges_before_the_post_turn_refresh() {
    let mut root = Root::legacy(vec![Reply::Text("Answer.")]);
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Ask"}"#)
        .await;
    let frames = root.frames();
    let end = position(&frames, |f| f["type"] == "agent_end");
    let ack = position(&frames, |f| f["type"] == "response" && f["id"] == "ep-1");
    assert!(
        frames[end..ack]
            .iter()
            .all(|f| f["type"] != "ledger_advanced"),
        "{frames:?}"
    );
}

/// Review #2762 (1): a note turn that ends terminal fences the next note,
/// but never the human's instruction queued behind it.
#[tokio::test]
async fn review_2762_instruction_queued_behind_two_notes_is_admitted_after_a_terminal_note_turn() {
    let mut root = Root::new(vec![
        Reply::Text("Working on it."),
        Reply::Call(
            "declare_turn_outcome",
            json!({"outcome":"done","reason":"Note handled"}),
        ),
        Reply::Text("Done."),
        Reply::Text("Following up."),
    ]);
    root.env.session.restore_pending(
        [
            PendingMessage::Automatic("note A".into()),
            PendingMessage::Automatic("note B".into()),
        ]
        .into_iter(),
    );
    let body = crate::interface::cli::uds_session::PromptBody {
        text: "Then this".into(),
        images: vec![],
    };
    let follow_up = crate::domain::inference::services::turn_suspension::Work::FOLLOW_UP;
    assert!(
        root.env
            .session
            .enqueue_control(Some("ep-2"), "follow_up", body, follow_up)
    );
    root.send(r#"{"type":"prompt","id":"ep-1","message":"Start"}"#)
        .await;
    let frames = root.frames();
    let done = position(&frames, |f| {
        f["type"] == "turn_outcome" && f["outcome"] == "done"
    });
    let second = position(&frames, |f| {
        f["type"] == "turn_admitted" && f["episodeId"] == "ep-2"
    });
    assert!(
        done < second,
        "the queued instruction is admitted: {frames:?}"
    );
    assert!(
        frames[done + 1..second]
            .iter()
            .all(|f| f["type"] != "agent_start"),
        "fenced note B does not run before it: {frames:?}"
    );
    let requests = root.provider.requests();
    assert_eq!(requests[3], "Then this", "{requests:?}");
}

/// Count the real configuration source polls at an idle prompt boundary,
/// while proving the configuration rebuilt by that poll reaches execution.
#[tokio::test]
async fn run42_round3_idle_prompt_polls_configuration_once() {
    use super::issue_2708_uds_parent_tests::Scripted;
    use crate::application::catalogue::ports::{ReloadedConfiguration, RuntimeConfigurationSource};
    use crate::application::catalogue::use_cases::ReloadRuntimeConfiguration;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Source {
        polls: Arc<AtomicUsize>,
        configuration: Option<ReloadedConfiguration>,
    }
    impl RuntimeConfigurationSource for Source {
        fn changed(&mut self) -> bool {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.configuration.is_some()
        }
        fn rebuild(&mut self) -> Result<ReloadedConfiguration, String> {
            self.configuration
                .take()
                .ok_or_else(|| "already rebuilt".into())
        }
    }
    let mut root = Root::legacy(vec![Reply::Text("obsolete provider")]);
    let replacement = Scripted::new(vec![Reply::Text("latest configuration")]);
    let polls = Arc::new(AtomicUsize::new(0));
    let reload = Arc::new(ReloadRuntimeConfiguration::new(Box::new(Source {
        polls: polls.clone(),
        configuration: Some(ReloadedConfiguration {
            provider: replacement.clone(),
            tool_policy: Default::default(),
        }),
    })));
    let command = crate::interface::uds::wire::protocol::parse_command_line(
        r#"{"type":"prompt","id":"reload-once","message":"Use latest config"}"#,
    )
    .unwrap();
    {
        let mut ctx = root.env.ctx();
        ctx.broadcast_tx = Some(root.tx.clone());
        ctx.stdout = None;
        ctx.catalogue.reload = reload;
        super::super::dispatch_command(command, &mut ctx).await;
    }
    assert_eq!(replacement.requests(), ["Use latest config"]);
    assert!(
        root.provider.requests().is_empty(),
        "old provider must be replaced"
    );
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "one idle prompt must poll/reconcile configuration once"
    );
}

async fn run42_response(
    outcome: super::PromptOutcome,
    diagnostics: &[&str],
) -> Vec<serde_json::Value> {
    let mut root = Root::new(vec![]);
    root.env.session.begin_completion_response(true);
    for diagnostic in diagnostics {
        assert_eq!(root.env.session.prompt_error((*diagnostic).into()), None);
    }
    {
        let mut ctx = root.env.ctx();
        ctx.broadcast_tx = Some(root.tx.clone());
        ctx.stdout = None;
        super::respond_prompt(
            &mut ctx,
            Some("original-id"),
            "prompt",
            &outcome,
            true,
            false,
        )
        .await;
    }
    root.frames()
}

#[tokio::test]
async fn run42_round3_error_appends_diagnostic_to_original() {
    let frames = run42_response(super::PromptOutcome::Error, &["provider down"]).await;
    assert_eq!(
        frames.len(),
        1,
        "no extra uncorrelated agent_error: {frames:?}"
    );
    assert_eq!(frames[0]["id"], "original-id");
    assert_eq!(frames[0]["success"], false);
    assert_eq!(frames[0]["error"], "prompt execution failed\nprovider down");
}

#[tokio::test]
async fn run42_round3_cancelled_appends_diagnostic_to_original() {
    let frames = run42_response(super::PromptOutcome::Cancelled, &["save diagnostic"]).await;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["id"], "original-id");
    assert_eq!(
        frames[0]["error"],
        "prompt execution cancelled\nsave diagnostic"
    );
}

#[tokio::test]
async fn run42_round3_completion_response_controls() {
    for (outcome, error, success) in [
        (
            super::PromptOutcome::Error,
            Some("prompt execution failed"),
            false,
        ),
        (
            super::PromptOutcome::Cancelled,
            Some("prompt execution cancelled"),
            false,
        ),
        (super::PromptOutcome::Success, None, true),
    ] {
        let frames = run42_response(outcome, &[]).await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["id"], "original-id");
        assert_eq!(frames[0]["success"], success);
        assert_eq!(frames[0]["error"].as_str(), error);
    }
    let frames = run42_response(super::PromptOutcome::Success, &["correction failed"]).await;
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0]["success"], true,
        "receipt describes original execution"
    );
    assert_eq!(
        frames[0]["error"], "correction failed",
        "diagnostic appears once, without empty prefix"
    );
}
