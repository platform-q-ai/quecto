//! What the member's event log never keeps, and the order it keeps the
//! rest in (#2304 swarm review): a tool call's arguments, text from the
//! stream that looks like a secret, an unknown event's type; results
//! paired on their calls' full ids; and a close racing a fold records the
//! fold first and the member's end last.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::test_rig::*;
use crate::application::external_agent::dto::SessionRecord;
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InitEvent, ToolResultEvent,
};
use crate::domain::external_agent::telemetry::ExternalAgentTool;

const SECRET: &str = "sk-ant-api03-LEAKLEAKLEAKLEAKLEAKLEAK";

fn tool_use(id: &str, name: &str, input: serde_json::Value) -> ExternalAgentEvent {
    ExternalAgentEvent::AssistantBlock {
        message_id: "m1".into(),
        block: AssistantContent::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        },
    }
}

fn tool_result(id: &str, content: &str, is_error: bool) -> ExternalAgentEvent {
    ExternalAgentEvent::ToolResult(ToolResultEvent {
        tool_use_id: Some(id.into()),
        content: serde_json::json!(content),
        is_error,
        permission_denied: false,
    })
}

fn tools(rig: &Rig) -> Vec<ExternalAgentTool> {
    rig.records
        .all()
        .into_iter()
        .filter_map(|record| match record {
            SessionRecord::ToolFinished(tool) => Some(*tool),
            _ => None,
        })
        .collect()
}

/// Every record, as the log would hold it.
fn logged(rig: &Rig) -> String {
    format!("{:?}", rig.records.all())
}

/// A tool call's record holds its name and sizes: never its arguments
/// (a command, a path, a message's text or a claim token), so it needs no
/// redaction.
#[tokio::test]
async fn a_tool_call_records_its_name_and_sizes_never_its_arguments() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let calls = [
        (
            "t1",
            "Bash",
            serde_json::json!({"command": format!("curl -u me:{SECRET} https://x")}),
        ),
        (
            "t2",
            "Write",
            serde_json::json!({"file_path": "/home/me/secret-plans.md", "content": "words"}),
        ),
        (
            "t3",
            "mcp__quecto__board_submit",
            serde_json::json!({"task_id": "T1", "token": "06f61158", "evidence": "evidence words"}),
        ),
    ];
    for (id, name, input) in &calls {
        rig.feed(tool_use(id, name, input.clone())).await;
        rig.feed(tool_result(id, "out", false)).await;
    }
    let recorded = tools(&rig);
    assert_eq!(recorded.len(), calls.len());
    for (tool, (id, name, input)) in recorded.iter().zip(&calls) {
        assert_eq!(
            (tool.tool_use_id.as_str(), tool.tool.as_str()),
            (*id, *name)
        );
        assert_eq!(tool.argument_bytes, input.to_string().len());
        assert_eq!(tool.result_bytes, 3);
        let json = serde_json::to_value(tool).unwrap();
        assert!(json.get("summary").is_none(), "no summary: {json}");
    }
    assert_eq!(recorded[2].task_id.as_deref(), Some("T1"));
    let log = logged(&rig);
    for argument in [
        "curl",
        "LEAKLEAK",
        "secret-plans",
        "06f61158",
        "evidence words",
    ] {
        assert!(!log.contains(argument), "{argument} in {log}");
    }
}

/// Text from the stream that looks like a secret is never kept: an
/// init's ids, a tool's name and id, a turn's reason and an unknown
/// event's type (kept only as a fixed-size fingerprint and a count).
#[tokio::test]
async fn stream_text_that_looks_like_a_secret_is_never_kept() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(ExternalAgentEvent::Init(InitEvent {
        session_id: Some(SECRET.into()),
        model: Some(format!("token={SECRET}")),
        cli_version: Some(SECRET.into()),
        ..InitEvent::default()
    }))
    .await;
    rig.feed(ExternalAgentEvent::Unknown {
        kind: SECRET.into(),
    })
    .await;
    rig.feed(ExternalAgentEvent::Unknown {
        kind: "system/brand_new".into(),
    })
    .await;
    rig.feed(tool_use(SECRET, SECRET, serde_json::json!({})))
        .await;
    rig.feed(tool_result(SECRET, "out", false)).await;
    rig.feed(result(true, SECRET, None)).await;
    let log = logged(&rig);
    assert!(!log.contains("LEAKLEAK"), "{log}");
    let [tool] = tools(&rig).try_into().expect("one call, answered");
    assert_eq!(tool.outcome, "ok", "paired on its id all the same");
    let names: Vec<String> = rig
        .records
        .all()
        .into_iter()
        .filter_map(|record| match record {
            SessionRecord::StreamDiagnostic(d) if d.kind == "unknown_event" => Some(d.name),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    for name in &names {
        assert!(name.starts_with("sha256:"), "{name}");
        assert_eq!(name.len(), "sha256:".len() + 16, "{name}");
    }
    assert!(
        !log.contains("brand_new"),
        "an unknown type is only a print"
    );
}

/// A result is paired with its call on the call's full id: two ids that
/// share their first 64 bytes, or differ only in a character the log
/// cannot keep, are two calls.
#[tokio::test]
async fn results_pair_with_their_calls_on_the_full_id() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let long_a = format!("{}A", "x".repeat(64));
    let long_b = format!("{}B", "x".repeat(64));
    rig.feed(tool_use(&long_a, "Bash", serde_json::json!({})))
        .await;
    rig.feed(tool_use(&long_b, "Read", serde_json::json!({})))
        .await;
    rig.feed(tool_use("t/1", "Grep", serde_json::json!({})))
        .await;
    rig.feed(tool_use("t 1", "Glob", serde_json::json!({})))
        .await;
    rig.feed(tool_result(&long_b, "bb", false)).await;
    rig.feed(tool_result("t 1", "dddd", false)).await;
    rig.feed(tool_result(&long_a, "a", true)).await;
    rig.feed(tool_result("t/1", "ccc", false)).await;
    let finished: Vec<(String, String, usize)> = tools(&rig)
        .into_iter()
        .map(|tool| (tool.tool, tool.outcome, tool.result_bytes))
        .collect();
    assert_eq!(
        finished,
        [
            ("Read".into(), "ok".into(), 2),
            ("Glob".into(), "ok".into(), 4),
            ("Bash".into(), "error".into(), 1),
            ("Grep".into(), "ok".into(), 3),
        ]
    );
}

/// A close racing a fold on another thread waits for the fold's records:
/// they come before `closed`, and `ended` is the last record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_close_racing_a_fold_records_the_fold_first_and_the_end_last() {
    let rig = started().await;
    let session = Arc::downgrade(&rig.session);
    let records = Arc::downgrade(&rig.records);
    let closer: Arc<Mutex<Option<std::thread::JoinHandle<()>>>> = Arc::default();
    let closing = closer.clone();
    rig.records.hook(Arc::new(move |record| {
        let (SessionRecord::StreamDiagnostic(_), Some(session), Some(records)) =
            (record, session.upgrade(), records.upgrade())
        else {
            return;
        };
        let mut closing = closing.lock().unwrap();
        if closing.is_some() {
            return;
        }
        *closing = Some(std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async { session.close().await.expect("the member is closed") });
        }));
        drop(closing);
        // Every chance for the close to run past this record: one not
        // held back by the fold records `closed` meanwhile.
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline
            && !records
                .all()
                .iter()
                .any(|record| matches!(record, SessionRecord::Closed { .. }))
        {
            std::thread::sleep(Duration::from_millis(5));
        }
    }));
    rig.wire.emit(ExternalAgentEvent::Unknown {
        kind: "system/brand_new".into(),
    });
    // Folded, and recorded, on this thread while the close runs on another.
    let _ = rig.step().await;
    let closer = closer.lock().unwrap().take().expect("the close ran");
    closer.join().expect("the close ended");
    rig.end_recorded().await;
    let kinds = rig.records.kinds();
    let at = |kind| kinds.iter().position(|k| *k == kind).expect(kind);
    assert!(at("stream_diagnostic") < at("closed"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"ended"), "{kinds:?}");
}

/// An event read before a close is not folded once the close is done, and
/// nothing is recorded after the member's end: a prompt the close refused
/// while it waited to be written leaves `ended` last.
#[tokio::test]
async fn nothing_is_recorded_after_the_end_of_a_closed_member() {
    let rig = named_started().await;
    rig.wire.hold_queue(true);
    let session = rig.session.clone();
    let prompt = tokio::spawn(async move { session.prompt("two", None).await });
    // The prompt holds the write gate, waiting to be queued.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    rig.wire.emit(ExternalAgentEvent::Unknown {
        kind: "system/brand_new".into(),
    });
    let session = rig.session.clone();
    let reader = tokio::spawn(async move { session.next_step().await });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    rig.session.close().await.expect("the member is closed");
    rig.wire.hold_queue(false);
    assert!(prompt.await.unwrap().is_err(), "the close refused it");
    assert_eq!(reader.await.unwrap(), None);
    rig.end_recorded().await;
    let kinds = rig.records.kinds();
    assert!(!kinds.contains(&"stream_diagnostic"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"ended"), "{kinds:?}");
}
