//! Master attach-backfill unit tests (#1050), split from
//! `app_rewind_response_tests.rs` to stay within the line-count gate.

use super::app_response::ATTACH_BACKFILL_ID;
use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn chat_text(app: &mut App) -> String {
    let lines = app.master_session.chat.render(120);
    lines
        .iter()
        .map(|l| super::app_render_helpers::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn respond(
    app: &mut App,
    id: Option<&str>,
    command: &str,
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<&str>,
) {
    // Tests that deliver synthetic own-client responses arm exact pending so
    // correlation matches production minting (#1237).
    if command == "get_messages" {
        match id {
            Some(ATTACH_BACKFILL_ID) => app.test_arm_attach_backfill(ATTACH_BACKFILL_ID),
            Some("resume-messages") => app.test_arm_resume_messages("resume-messages"),
            Some("rewind-refresh") => app.test_arm_rewind_refresh("rewind-refresh"),
            _ => {}
        }
    }
    app.handle_response(
        id.map(String::from),
        command.to_string(),
        success,
        data,
        error.map(String::from),
    );
}

fn attach_backfill_data(pairs: &[(&str, &str)]) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = pairs
        .iter()
        .enumerate()
        .flat_map(|(i, (u, a))| {
            [
                serde_json::json!({ "role": "user", "content": u, "id": format!("u{i}") }),
                serde_json::json!({ "role": "assistant", "content": a, "id": format!("a{i}") }),
            ]
        })
        .collect();
    serde_json::json!({ "messages": messages })
}

fn is_attach_backfill_get_messages(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    if v.get("type").and_then(|t| t.as_str()) != Some("get_messages") {
        return false;
    }
    v.get("id").and_then(|i| i.as_str()).is_some_and(|id| {
        let id = id.strip_prefix("tab0:").unwrap_or(id);
        id == ATTACH_BACKFILL_ID || id.starts_with("attach-backfill-")
    })
}

#[tokio::test]
async fn request_master_attach_backfill_sends_get_messages_with_dedicated_id() {
    // On --socket attach the master must request durable history with a
    // dedicated request id so the response path can reconcile (not resume).
    let mut h = harness().await;
    h.app_mut().request_master_attach_backfill();
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|line| is_attach_backfill_get_messages(line)),
        "attach must request get_messages with an attach-backfill family id, got: {cmds:?}"
    );
    let id = cmds
        .iter()
        .find_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let id = v.get("id")?.as_str()?;
            let local = id.strip_prefix("tab0:").unwrap_or(id);
            (local == ATTACH_BACKFILL_ID || local.starts_with("attach-backfill-"))
                .then(|| id.to_string())
        })
        .expect("attach id");
    assert!(
        id.starts_with("tab0:attach-backfill-") && id != ATTACH_BACKFILL_ID,
        "attach id must be uniquely minted, got {id}"
    );
    assert_eq!(
        h.app_mut().test_pending_attach_backfill_id(),
        Some(id.as_str())
    );
}

#[tokio::test]
async fn run_startup_requests_master_attach_backfill() {
    // `App::run` startup must request durable master history so `--socket`
    // attach shows prior session content without waiting for new events.
    let mut h = harness().await;
    let app = h.app_mut();
    app.should_exit = true;
    assert_eq!(app.run().await, 0);
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|line| is_attach_backfill_get_messages(line)),
        "run() startup must send get_messages with attach-backfill family id, got: {cmds:?}"
    );
}

#[tokio::test]
async fn attach_backfill_prepends_history_and_preserves_live_tokens() {
    // Live tokens can race ahead of the attach get_messages response. The
    // backfill must PREPEND history above live content — never wholesale
    // replace that drops the live stream (#1050, parity with #828).
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::AgentStart);
    a.handle_event(Event::Token {
        token: "LIVE_AFTER_ATTACH".into(),
    });
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[(
            "earlier question",
            "earlier answer",
        )])),
        None,
    );

    let frame = chat_text(a);
    assert!(
        frame.contains("earlier question") && frame.contains("earlier answer"),
        "attach backfill must render prior history:\n{frame}"
    );
    assert!(
        frame.contains("LIVE_AFTER_ATTACH"),
        "late attach backfill must NOT drop live tokens:\n{frame}"
    );
    let hist = frame.find("earlier answer").expect("history present");
    let live = frame.find("LIVE_AFTER_ATTACH").expect("live present");
    assert!(
        hist < live,
        "history must be PREPENDED above live content:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "attach backfill must not use the resume replace path:\n{frame}"
    );
}

#[tokio::test]
async fn attach_backfill_is_idempotent_and_does_not_duplicate_history() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::Token {
        token: "LIVEONE".into(),
    });
    let data = attach_backfill_data(&[("the question", "the answer")]);
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(data.clone()),
        None,
    );
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(data),
        None,
    );

    let frame = chat_text(a);
    assert_eq!(
        frame.matches("the answer").count(),
        1,
        "re-delivered attach backfill must not duplicate history:\n{frame}"
    );
    assert!(
        frame.contains("LIVEONE"),
        "re-delivered attach backfill must not drop live content:\n{frame}"
    );
}

#[tokio::test]
async fn empty_attach_backfill_does_not_latch_guard_against_later_history() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[])),
        None,
    );
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("real question", "real answer")])),
        None,
    );

    let frame = chat_text(a);
    assert!(
        frame.contains("real question") && frame.contains("real answer"),
        "an empty attach backfill must not suppress a later populated history:\n{frame}"
    );
    // Pin attach-backfill routing: wholesale replace would still show the real
    // pair after the second response, so require reconcile (no "Session resumed").
    assert!(
        !frame.contains("Session resumed"),
        "empty→populated attach backfill must reconcile, not replace:\n{frame}"
    );
}

/// Busy mid-turn `--socket` attach: the agent accept loop pushes an unsolicited
/// id-less `get_messages` snapshot before the solicited `attach-backfill` reply.
/// Both must reconcile (not replace then prepend) so history is not duplicated.
#[tokio::test]
async fn busy_connect_idless_snapshot_then_attach_backfill_does_not_duplicate() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Live tokens can already be streaming when the snapshot arrives.
    a.handle_event(Event::AgentStart);
    a.handle_event(Event::Token {
        token: "LIVE_MID_TURN".into(),
    });
    // Unsolicited busy-connect snapshot: no id, snapshot:true (uds_snapshots).
    let mut snapshot = attach_backfill_data(&[("prior question", "prior answer")]);
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("snapshot".into(), serde_json::json!(true));
    }
    respond(a, None, "get_messages", true, Some(snapshot), None);
    // Solicited attach-backfill arrives after the snapshot.
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("prior question", "prior answer")])),
        None,
    );

    let frame = chat_text(a);
    assert_eq!(
        frame.matches("prior answer").count(),
        1,
        "busy-connect snapshot + attach-backfill must not double-apply history:\n{frame}"
    );
    assert!(
        frame.contains("LIVE_MID_TURN"),
        "busy-connect path must preserve live tokens:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "busy-connect snapshot must reconcile, not wholesale-replace:\n{frame}"
    );
    let hist = frame.find("prior answer").expect("history present");
    let live = frame.find("LIVE_MID_TURN").expect("live present");
    assert!(
        hist < live,
        "history must remain above live mid-turn content:\n{frame}"
    );
}

/// Oversized busy-connect snapshots set `trimmed: true` and omit the oldest
/// messages (`uds_snapshots` frame budget). That partial tail must NOT latch
/// completion — the later full attach-backfill must restore omitted history
/// exactly once without duplicating the snapshot tail (#1050 review).
#[tokio::test]
async fn trimmed_busy_connect_snapshot_then_full_attach_backfill_restores_older_history() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::AgentStart);
    a.handle_event(Event::Token {
        token: "LIVE_AFTER_TRIMMED".into(),
    });
    // Unsolicited busy-connect snapshot: only the newest tail, marked trimmed.
    let mut snapshot = attach_backfill_data(&[("recent question", "recent answer")]);
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("snapshot".into(), serde_json::json!(true));
        obj.insert("trimmed".into(), serde_json::json!(true));
    }
    respond(a, None, "get_messages", true, Some(snapshot), None);

    // Intermediate frame: partial tail is visible while we wait for full history.
    let mid = chat_text(a);
    assert!(
        mid.contains("recent answer") && mid.contains("LIVE_AFTER_TRIMMED"),
        "trimmed snapshot must still render its tail + live tokens:\n{mid}"
    );
    assert!(
        !mid.contains("oldest question"),
        "trimmed snapshot must not invent omitted older history:\n{mid}"
    );

    // Solicited attach-backfill: full durable history (older + recent).
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[
            ("oldest question", "oldest answer"),
            ("recent question", "recent answer"),
        ])),
        None,
    );

    let frame = chat_text(a);
    assert!(
        frame.contains("oldest question") && frame.contains("oldest answer"),
        "complete attach-backfill must restore history omitted by trimmed snapshot:\n{frame}"
    );
    assert_eq!(
        frame.matches("recent answer").count(),
        1,
        "full backfill must replace the partial prefix, not duplicate the tail:\n{frame}"
    );
    assert!(
        frame.contains("LIVE_AFTER_TRIMMED"),
        "trimmed→full path must preserve live tokens:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "trimmed→full path must reconcile, not wholesale-replace:\n{frame}"
    );
    let oldest = frame.find("oldest answer").expect("oldest present");
    let recent = frame.find("recent answer").expect("recent present");
    let live = frame.find("LIVE_AFTER_TRIMMED").expect("live present");
    assert!(
        oldest < recent && recent < live,
        "full history must stay ordered above live content:\n{frame}"
    );
}

#[tokio::test]
async fn attach_backfill_into_idle_master_renders_full_history_in_order() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("first question", "first answer")])),
        None,
    );

    let frame = chat_text(a);
    let q = frame
        .find("first question")
        .expect("history question present");
    let apos = frame.find("first answer").expect("history answer present");
    assert!(
        q < apos,
        "idle attach backfill must render history in order:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "attach backfill must reconcile, not the resume replace path:\n{frame}"
    );
}

#[tokio::test]
async fn resume_get_messages_still_replaces_chat_when_not_attach_backfill() {
    // Non-attach get_messages (resume / rewind-refresh) must keep the wholesale
    // replace path and "Session resumed" status (#1050 must not break #resume).
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::Token {
        token: "SHOULD_BE_CLEARED".into(),
    });
    respond(
        a,
        Some("resume-messages"),
        "get_messages",
        true,
        Some(attach_backfill_data(&[(
            "resumed user",
            "resumed assistant",
        )])),
        None,
    );
    let frame = chat_text(a);
    assert!(
        frame.contains("Session resumed"),
        "resume path must still replace chat:\n{frame}"
    );
    assert!(
        frame.contains("resumed user"),
        "resume path must show resumed messages:\n{frame}"
    );
    assert!(
        !frame.contains("SHOULD_BE_CLEARED"),
        "resume replace must clear prior live content:\n{frame}"
    );
}

#[tokio::test]
async fn rewind_open_get_messages_still_opens_selector_over_attach_path() {
    // A rewind-pending get_messages id must open the rewind selector, never
    // attach-backfill reconcile, even if history is also pending.
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_open_id = Some("rewind-open-1".into());
    respond(
        a,
        Some("rewind-open-1"),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("turn one", "reply one")])),
        None,
    );
    assert!(
        a.rewind.selector.is_some(),
        "rewind pending id must open the rewind selector"
    );
    assert!(
        a.rewind.pending_open_id.is_none(),
        "rewind open id must be cleared after handling"
    );
    let frame = chat_text(a);
    assert!(
        !frame.contains("turn one"),
        "rewind open must not inject history into chat:\n{frame}"
    );
}
