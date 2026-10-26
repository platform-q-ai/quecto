//! #2762 review round 1: the guard's queued-work, correction and swarm
//! cases on the real UDS dispatch path, over the #2708 rig.
use super::issue_2708_uds_parent_tests::{PROMISE_2648, Reply, Root, of_type, position};
use crate::domain::agents::value_objects::turn_outcome::LIVE_REASON;
use crate::interface::cli::uds_session::PendingMessage;
use serde_json::json;

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

/// Review #2762 (A): a swarm the parent launched is live while its
/// coordinator is, idle or not: the parent's wake turn waits.
#[tokio::test]
async fn review_2762_an_open_swarm_with_an_idle_coordinator_keeps_a_wake_turn_waiting() {
    let mut root = Root::new(vec![
        Reply::Text("Started the swarm."),
        Reply::Text("Noted."),
    ]);
    {
        let registry = root.env.subagent_registry.clone().unwrap();
        let mut coordinator =
            crate::infrastructure::tools::agents::subagent_registry::SubagentEntry::new(
                "/s".into(),
                0,
            );
        coordinator.status =
            crate::infrastructure::tools::agents::subagent_registry::SubagentStatus::Idle;
        coordinator.coordinator_wake.launched = true;
        registry
            .lock()
            .unwrap()
            .insert("coordinator".into(), coordinator);
    }
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
