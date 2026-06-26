use super::*;
use quecto::infrastructure::tools::subagent_monitor::spawn_monitor_task;
use quecto::infrastructure::tools::subagent_registry::{
    SequencedSubagentNotification, SubagentEntry, SubagentNotification, extract_summary,
    new_notification_channel, new_registry,
};
use quecto::interface::cli::uds_session::AgentSession;
use tokio::io::AsyncWriteExt;

// ===========================================================================
// Subagent Notify BDD Steps (#523)
// ===========================================================================

fn sequenced(sequence: u64, notification: SubagentNotification) -> SequencedSubagentNotification {
    SequencedSubagentNotification::new(sequence, notification)
}

fn drive_monitor_with_lines(
    world: &mut QuectoWorld,
    agent_id: &str,
    lines: &[&str],
    close_after: bool,
) {
    let tx = world.notify_tx.as_ref().expect("no notify tx").clone();
    let temp = tempfile::TempDir::new().expect("monitor socket temp dir");
    let socket_path = temp.path().join("child.sock");
    let registry = new_registry();
    registry.lock().unwrap().insert(
        agent_id.to_string(),
        SubagentEntry::new(socket_path.clone(), 0),
    );

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind monitor socket");
        let handle = spawn_monitor_task(
            agent_id.to_string(),
            socket_path.clone(),
            registry,
            Some(tx),
            None,
            None,
        );
        let (mut stream, _) = listener.accept().await.expect("accept monitor connection");
        for line in lines {
            stream
                .write_all(line.as_bytes())
                .await
                .expect("write event");
            stream.write_all(b"\n").await.expect("write newline");
        }
        if close_after {
            stream.shutdown().await.expect("shutdown monitor stream");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        handle.abort();
    });
}

// --- Given ---

#[given(expr = "a Completed notification for agent {string} with summary {string}")]
fn given_completed_notification(world: &mut QuectoWorld, agent_id: String, summary: String) {
    let notif = SubagentNotification::Completed { agent_id, summary };
    world.notify_message = Some(notif.to_message());
}

#[given(expr = "an Errored notification for agent {string} with error {string}")]
fn given_errored_notification(world: &mut QuectoWorld, agent_id: String, error: String) {
    let notif = SubagentNotification::Errored { agent_id, error };
    world.notify_message = Some(notif.to_message());
}

#[given(expr = "an Exited notification for agent {string}")]
fn given_exited_notification(world: &mut QuectoWorld, agent_id: String) {
    let notif = SubagentNotification::Exited { agent_id };
    world.notify_message = Some(notif.to_message());
}

#[given(expr = "an agent_end event with messages containing assistant text {string}")]
fn given_agent_end_with_text(world: &mut QuectoWorld, text: String) {
    world.notify_messages_json = Some(serde_json::json!([
        {"role": "assistant", "content": text}
    ]));
}

#[given("an agent_end event with assistant text of 300 characters")]
fn given_agent_end_long_text(world: &mut QuectoWorld) {
    let long = "x".repeat(300);
    world.notify_messages_json = Some(serde_json::json!([
        {"role": "assistant", "content": long}
    ]));
}

#[given("an agent_end event with empty messages array")]
fn given_agent_end_empty_messages(world: &mut QuectoWorld) {
    world.notify_messages_json = Some(serde_json::json!([]));
}

#[given("an agent_end event with only tool messages")]
fn given_agent_end_tool_only(world: &mut QuectoWorld) {
    world.notify_messages_json = Some(serde_json::json!([
        {"role": "tool", "content": "tool output"}
    ]));
}

#[given(expr = "a SubagentNotification channel with capacity {int}")]
fn given_channel_with_capacity(world: &mut QuectoWorld, _capacity: i32) {
    let (tx, rx) = new_notification_channel();
    world.notify_tx = Some(tx);
    world.notify_rx = Some(rx);
}

#[given(expr = "a SubagentNotification channel with {int} pending notifications")]
fn given_channel_with_pending(world: &mut QuectoWorld, count: i32) {
    let (tx, rx) = new_notification_channel();
    for i in 0..count {
        let _ = tx.try_send(sequenced(
            i as u64,
            SubagentNotification::Exited {
                agent_id: format!("bot-{}", i),
            },
        ));
    }
    world.notify_tx = Some(tx);
    world.notify_rx = Some(rx);
}

#[given("a monitor with notification sender")]
fn given_monitor_with_sender(world: &mut QuectoWorld) {
    let (tx, rx) = new_notification_channel();
    world.notify_tx = Some(tx);
    world.notify_rx = Some(rx);
}

// --- When ---

#[when("I extract the summary")]
fn when_extract_summary(world: &mut QuectoWorld) {
    let json = world
        .notify_messages_json
        .as_ref()
        .expect("no messages json");
    world.notify_extracted_summary = Some(extract_summary(json));
}

#[when("I drain all notifications")]
fn when_drain_notifications(world: &mut QuectoWorld) {
    let mut rx = world.notify_rx.take().expect("no notification rx");
    drop(world.notify_tx.take()); // close sender so recv returns None
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut count = 0;
    rt.block_on(async {
        while rx.recv().await.is_some() {
            count += 1;
        }
    });
    world.notify_drain_count = Some(count);
}

#[when("the monitor processes an agent_end event with messages")]
fn when_monitor_agent_end(world: &mut QuectoWorld) {
    let line = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"Done"}]}"#;
    drive_monitor_with_lines(world, "child-1", &[line], false);
}

#[when("the monitor processes a tool_execution_end event with is_error true")]
fn when_monitor_tool_error(world: &mut QuectoWorld) {
    let line = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":true}"#;
    drive_monitor_with_lines(world, "child-1", &[line], false);
}

#[when(expr = "the monitor detects connection closed for agent {string}")]
fn when_monitor_connection_closed_for(world: &mut QuectoWorld, agent_id: String) {
    drive_monitor_with_lines(world, &agent_id, &[], true);
}

#[when("the monitor processes an agent_start event")]
fn when_monitor_agent_start(world: &mut QuectoWorld) {
    let line = r#"{"type":"agent_start"}"#;
    drive_monitor_with_lines(world, "child-1", &[line], false);
}

// --- Then ---

#[then(expr = "the notification message should contain {string}")]
fn then_notify_message_contains(world: &mut QuectoWorld, expected: String) {
    let msg = world.notify_message.as_ref().expect("no notify message");
    assert!(
        msg.contains(&expected),
        "expected message to contain '{}', got: {}",
        expected,
        msg
    );
}

#[then(expr = "the notification message should start with {string}")]
fn then_notify_message_starts_with(world: &mut QuectoWorld, expected: String) {
    let msg = world.notify_message.as_ref().expect("no notify message");
    assert!(
        msg.starts_with(&expected),
        "expected message to start with '{}', got: {}",
        expected,
        msg
    );
}

#[then(expr = "the extracted summary should be {string}")]
fn then_extracted_summary(world: &mut QuectoWorld, expected: String) {
    let summary = world
        .notify_extracted_summary
        .as_ref()
        .expect("no extracted summary");
    assert_eq!(summary, &expected);
}

#[then(expr = "the extracted summary should be at most {int} characters")]
fn then_extracted_summary_max_len(world: &mut QuectoWorld, max_len: i32) {
    let summary = world
        .notify_extracted_summary
        .as_ref()
        .expect("no extracted summary");
    assert!(
        summary.len() <= max_len as usize,
        "expected at most {} chars, got {} ({})",
        max_len,
        summary.len(),
        summary
    );
}

#[then(expr = "sending {int} notifications should succeed")]
fn then_sending_n_notifications(world: &mut QuectoWorld, count: i32) {
    let tx = world.notify_tx.as_ref().expect("no notify tx");
    for i in 0..count {
        let result = tx.try_send(sequenced(
            i as u64,
            SubagentNotification::Completed {
                agent_id: format!("bot-{}", i),
                summary: "done".into(),
            },
        ));
        assert!(result.is_ok(), "send {} failed: {:?}", i, result);
    }
}

#[then("the channel should not block on bounded sends")]
fn then_channel_bounded(world: &mut QuectoWorld) {
    // The previous step verified all 64 sends succeeded. A 65th send should
    // fail (channel is full), proving it is bounded.
    let tx = world.notify_tx.as_ref().expect("no notify tx");
    let result = tx.try_send(sequenced(
        65,
        SubagentNotification::Exited {
            agent_id: "overflow".into(),
        },
    ));
    assert!(result.is_err(), "expected channel full, but send succeeded");
}

#[then(expr = "I should receive {int} notifications")]
fn then_receive_n_notifications(world: &mut QuectoWorld, expected: i32) {
    let count = world.notify_drain_count.expect("no drain count");
    assert_eq!(count, expected as usize);
}

#[then("a Completed notification should be sent")]
fn then_completed_notification(world: &mut QuectoWorld) {
    let mut rx = world.notify_rx.take().expect("no notify rx");
    let notif = rx.try_recv().expect("no notification received");
    match notif.notification {
        SubagentNotification::Completed { .. } => {}
        other => panic!("expected Completed, got: {:?}", other),
    }
    world.notify_rx = Some(rx);
}

#[then("an Errored notification should be sent")]
fn then_errored_notification(world: &mut QuectoWorld) {
    let mut rx = world.notify_rx.take().expect("no notify rx");
    let notif = rx.try_recv().expect("no notification received");
    match notif.notification {
        SubagentNotification::Errored { .. } => {}
        other => panic!("expected Errored, got: {:?}", other),
    }
    world.notify_rx = Some(rx);
}

#[then(expr = "an Exited notification should be sent for {string}")]
fn then_exited_notification(world: &mut QuectoWorld, expected_id: String) {
    let mut rx = world.notify_rx.take().expect("no notify rx");
    let notif = rx.try_recv().expect("no notification received");
    match notif.notification {
        SubagentNotification::Exited { agent_id } => {
            assert_eq!(agent_id, expected_id);
        }
        other => panic!("expected Exited, got: {:?}", other),
    }
    world.notify_rx = Some(rx);
}

#[then("no notification should be sent")]
fn then_no_notification(world: &mut QuectoWorld) {
    let rx = world.notify_rx.as_mut().expect("no notify rx");
    assert!(
        rx.try_recv().is_err(),
        "expected no notification, but one was received"
    );
}

// ===========================================================================
// #816: auto-await — completion notes surface at the parent's idle boundary
// ===========================================================================

/// Next monotonic completion sequence for `agent_id` — kept here, out of the
/// Gherkin, so scenarios read in business language and never surface the
/// internal `notification_sequence` counter.
fn next_sequence(world: &mut QuectoWorld, agent_id: &str) -> u64 {
    let seq = world.notify_seq.entry(agent_id.to_string()).or_insert(0);
    *seq += 1;
    *seq
}

#[given("a parent session with no pending notes")]
fn given_parent_session(world: &mut QuectoWorld) {
    world.notify_parent_session = Some(AgentSession::new(
        "model".to_string(),
        "cli:parent".to_string(),
    ));
}

#[given("the parent is busy processing a turn")]
fn given_parent_busy(_world: &mut QuectoWorld) {
    // Marker step: the assertions that follow verify the note is only buffered
    // (never injected into a turn) until the parent reaches idle. Buffering is a
    // property of enqueue itself, so no extra state is needed here.
}

#[when(expr = "subagent {string} reports completion with note {string}")]
fn when_subagent_completes(world: &mut QuectoWorld, agent_id: String, note: String) {
    let sequence = next_sequence(world, &agent_id);
    let session = world
        .notify_parent_session
        .as_mut()
        .expect("no parent session");
    world.notify_last_enqueued =
        Some(session.enqueue_subagent_notification(agent_id, sequence, note));
}

#[when(expr = "subagent {string} reports a newer completion with note {string}")]
fn when_subagent_newer_completion(world: &mut QuectoWorld, agent_id: String, note: String) {
    when_subagent_completes(world, agent_id, note);
}

#[when(expr = "subagent {string} reports the same completion again")]
fn when_subagent_same_completion(world: &mut QuectoWorld, agent_id: String) {
    let sequence = *world
        .notify_seq
        .get(&agent_id)
        .expect("no prior completion");
    let session = world
        .notify_parent_session
        .as_mut()
        .expect("no parent session");
    world.notify_last_enqueued =
        Some(session.enqueue_subagent_notification(agent_id, sequence, "done".to_string()));
}

#[when("the parent drains its subagent notifications")]
fn when_parent_drains_channel(world: &mut QuectoWorld) {
    // Route monitor-produced notifications into the parent session exactly as the
    // live dispatch loop does on receive — proving the spawn→monitor→note path
    // with no manual await call.
    let mut rx = world.notify_rx.take().expect("no notification rx");
    drop(world.notify_tx.take());
    let session = world
        .notify_parent_session
        .as_mut()
        .expect("no parent session");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        while let Some(notif) = rx.recv().await {
            let (agent_id, sequence) = notif.dedupe_key();
            session.enqueue_subagent_notification(agent_id, sequence, notif.to_message());
        }
    });
}

#[then(expr = "the parent should have {int} pending subagent note")]
#[then(expr = "the parent should have {int} pending subagent notes")]
fn then_pending_note_count(world: &mut QuectoWorld, expected: usize) {
    let session = world
        .notify_parent_session
        .as_ref()
        .expect("no parent session");
    let count = session.state_snapshot(0, None, 0).pending_message_count;
    assert_eq!(count, expected, "unexpected pending note count");
}

#[then("the second report should be ignored")]
fn then_second_report_ignored(world: &mut QuectoWorld) {
    assert_eq!(
        world.notify_last_enqueued,
        Some(false),
        "a duplicate completion must be ignored, not re-delivered"
    );
}

#[then("the busy parent should not have consumed the note yet")]
fn then_busy_not_consumed(world: &mut QuectoWorld) {
    let session = world
        .notify_parent_session
        .as_ref()
        .expect("no parent session");
    assert_eq!(
        session.state_snapshot(0, None, 0).pending_message_count,
        1,
        "while busy the note stays buffered and is not injected into the turn"
    );
}

/// Drain the first idle note once and cache it, so several assertions in one
/// scenario inspect the same delivered note instead of re-draining the queue.
fn idle_note(world: &mut QuectoWorld) -> &quecto::domain::message::Message {
    if world.notify_drained_note.is_none() {
        let session = world
            .notify_parent_session
            .as_mut()
            .expect("no parent session");
        let msg = session
            .drain_pending()
            .into_iter()
            .next()
            .expect("no pending note")
            .into_message();
        world.notify_drained_note = Some(msg);
    }
    world.notify_drained_note.as_ref().unwrap()
}

#[then("the parent's next idle note should be delivered on the operator channel")]
fn then_next_note_is_system(world: &mut QuectoWorld) {
    use quecto::domain::message::Role;
    assert_eq!(idle_note(world).role, Role::System);
}

#[then("the parent's next idle note should be a single line")]
fn then_next_note_single_line(world: &mut QuectoWorld) {
    // The operator-facing summary is a single line; only the provenance envelope
    // tags add structural newlines around it.
    let content = idle_note(world).content.clone();
    let body: Vec<&str> = content
        .lines()
        .filter(|l| {
            !l.starts_with("<subagent_notification") && !l.starts_with("</subagent_notification")
        })
        .collect();
    assert_eq!(
        body.len(),
        1,
        "expected a single-line summary, got: {content:?}"
    );
}

#[then(expr = "the parent's next idle note should contain {string}")]
fn then_next_note_contains(world: &mut QuectoWorld, needle: String) {
    let content = idle_note(world).content.clone();
    assert!(
        content.contains(&needle),
        "note {content:?} did not contain {needle:?}"
    );
}
