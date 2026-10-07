//! #2192 review (PRRT_kwDORUxnPM6mfx7-, PRRT_kwDORUxnPM6mfxxC): an ended
//! child's default `get_messages` keeps the unread-report contract a live
//! child's has (#2226) — the report is delivered however much followed it,
//! and once acknowledged it is not replayed — and its final report, or any
//! one message, is read whole up to the final-report budget (#2114).
use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::application::tools::ports::Tool;
use crate::domain::agents::child_end::ChildOrigin;
use crate::domain::ids::AgentUuid;
use crate::domain::message::ToolCall;
use crate::domain::sessions::entities::session::Session;
use crate::domain::sessions::entities::session::SubagentLiveness;
use crate::domain::turn_origin::TurnOrigin;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::agent_cmd_ended::EndedChildSlot;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};

const UUID: &str = "65268567-be4a-471f-a805-1238dcf08b68";

struct Ended {
    _base: tempfile::TempDir,
    tool: AgentCmdTool,
}

/// An ended child this harness launched, whose persisted transcript is
/// `messages`, behind an `agent_cmd` tool that can read what it left.
async fn ended_child(messages: Vec<Message>) -> Ended {
    let base = tempfile::tempdir().unwrap();
    let store = crate::composition::sessions::build_file_session_store(base.path());
    let mut session =
        Session::new(InspectEndedChild::child_session(&AgentUuid::new(UUID)).unwrap());
    session.messages = messages;
    store.save(&session).await.unwrap();
    let registry = new_registry();
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(UUID),
        "worker".into(),
        "/tmp/worker.sock".into(),
        7,
    );
    entry.origin = ChildOrigin::Launched;
    entry.persisted_liveness = SubagentLiveness::Dead;
    registry.lock().unwrap().insert(UUID.into(), entry);
    let slot = EndedChildSlot::default();
    assert!(slot.install(
        crate::composition::subagent_lifecycle::build_ended_child_inspection(base.path())
    ));
    Ended {
        _base: base,
        tool: AgentCmdTool::new(registry).with_ended_child_slot(slot),
    }
}

/// Run `arguments` and deliver the result, as the agent loop does.
async fn read(ended: &Ended, arguments: &str) -> ToolResult {
    let result = ended.tool.execute(arguments).await.unwrap();
    ended.tool.result_delivered(arguments, &result);
    result
}

const DEFAULT_READ: &str = r#"{"command":"get_messages","agent_id":"worker"}"#;

fn instruction(text: &str) -> Message {
    let mut message = Message::user(text);
    message.turn_origin = TurnOrigin::Instruction;
    message
}

fn answer(text: &str) -> Message {
    let mut message = Message::assistant(text, vec![]);
    message.turn_origin = TurnOrigin::Instruction;
    message
}

/// `count` tool-call rounds: an assistant call and its result each.
fn tool_rounds(count: usize) -> Vec<Message> {
    (0..count)
        .flat_map(|i| {
            let call = ToolCall {
                id: format!("call-{i}"),
                name: "read".into(),
                arguments: "{}".into(),
            };
            [
                Message::assistant("", vec![call]),
                Message::tool(format!("call-{i}"), format!("output {i}")),
            ]
        })
        .collect()
}

fn data(result: &ToolResult) -> serde_json::Value {
    assert!(!result.is_error, "{}", result.content);
    serde_json::from_str::<serde_json::Value>(&result.content).unwrap()["data"].clone()
}

#[tokio::test]
async fn a_report_followed_by_many_messages_is_still_the_first_default_read() {
    let mut transcript = vec![
        instruction("do the cases"),
        answer("FINAL REPORT: all passed"),
    ];
    transcript.extend(tool_rounds(30));
    let ended = ended_child(transcript).await;
    let first = read(&ended, DEFAULT_READ).await;
    assert!(
        first.delivery_metadata.is_some(),
        "a receipt to acknowledge: {}",
        first.content
    );
    let first = data(&first);
    assert_eq!(first["ended"], true, "{first}");
    assert!(
        first.to_string().contains("FINAL REPORT: all passed"),
        "the report is delivered: {first}"
    );
    // Acknowledged: the next default read brings only what followed it.
    let second = data(&read(&ended, DEFAULT_READ).await);
    assert!(!second.to_string().contains("FINAL REPORT"), "{second}");
    // And once that is delivered too, nothing is replayed.
    let third = data(&read(&ended, DEFAULT_READ).await);
    assert_eq!(third["unchanged"], true, "{third}");
    assert!(!third.to_string().contains("FINAL REPORT"), "{third}");
}

#[tokio::test]
async fn an_explicit_page_neither_consults_nor_moves_the_watermark() {
    let ended = ended_child(vec![instruction("go"), answer("THE REPORT")]).await;
    let page = data(
        &read(
            &ended,
            r#"{"command":"get_messages","agent_id":"worker","count":5}"#,
        )
        .await,
    );
    assert!(page.to_string().contains("THE REPORT"), "{page}");
    // The default read still delivers the report: the page acknowledged nothing.
    let first = data(&read(&ended, DEFAULT_READ).await);
    assert!(first.to_string().contains("THE REPORT"), "{first}");
}

/// A 40 KiB final report of a child that then crashed is read whole.
#[tokio::test]
async fn a_long_final_report_is_read_whole_by_get_report_and_by_one_message_page() {
    let long = format!("REPORT {}", "r".repeat(40 * 1024));
    let mut transcript = vec![instruction("write it up"), answer(&long)];
    transcript.extend(tool_rounds(2));
    let ended = ended_child(transcript).await;
    let report = data(&read(&ended, r#"{"command":"get_report","agent_id":"worker"}"#).await);
    assert_eq!(report["ended"], true, "{report}");
    assert_eq!(report["report"]["content"], long.as_str());
    assert_eq!(report["report"]["contentTruncated"], false);
    let ordinal = report["report"]["ordinal"].as_u64().unwrap();
    let before = (ordinal + 1).to_string();
    let page = data(
        &read(
            &ended,
            &serde_json::json!({
                "command": "get_messages", "agent_id": "worker", "count": 1, "before": before
            })
            .to_string(),
        )
        .await,
    );
    let message = &page["messages"][0];
    assert_eq!(message["content"], long.as_str(), "whole, not a 16 KiB cut");
    assert!(message.get("contentLength").is_none());
}

#[tokio::test]
async fn a_report_past_the_budget_is_cut_with_a_notice_and_none_is_said_plainly() {
    let huge = "h".repeat(100 * 1024);
    let ended = ended_child(vec![instruction("go"), answer(&huge)]).await;
    let report = data(&read(&ended, r#"{"command":"get_report","agent_id":"worker"}"#).await);
    let content = report["report"]["content"].as_str().unwrap();
    assert!(content.len() < 64 * 1024, "{}", content.len());
    assert_eq!(report["report"]["contentTruncated"], true);
    assert!(report["report"]["contentNotice"].is_string(), "{report}");

    let ended = ended_child(vec![instruction("go")]).await;
    let none = data(&read(&ended, r#"{"command":"get_report","agent_id":"worker"}"#).await);
    assert_eq!(none["reportFound"], false, "{none}");
}

/// #2404 review M2: an ended child's messages name their user kind, as a
/// live child's page does, so a supervisor tells a watermark cut's archive
/// stub from a prompt; an unmarked message names none.
#[tokio::test]
async fn an_ended_childs_messages_name_their_user_kind() {
    let brief = crate::domain::turn_origin::prompt("the brief".into());
    let stub = crate::domain::conversation::watermark_cut::archive_stub(9, Some("archive"));
    assert_eq!(wire_message(&brief)["userKind"], "prompt");
    assert_eq!(wire_message(&stub)["userKind"], "archiveStub");
    let unmarked = wire_message(&instruction("go"));
    assert!(unmarked.get("userKind").is_none(), "{unmarked}");
    let ended = ended_child(vec![brief, stub, answer("THE REPORT")]).await;
    let page = data(
        &read(
            &ended,
            r#"{"command":"get_messages","agent_id":"worker","count":5}"#,
        )
        .await,
    );
    let kinds: Vec<Option<&str>> = page["messages"]
        .as_array()
        .expect("a page")
        .iter()
        .map(|m| m.get("userKind").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(kinds, [Some("prompt"), Some("archiveStub"), None], "{page}");
}
