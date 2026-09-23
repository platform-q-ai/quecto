//! #2114: a parent reads a finished child's report in full through
//! `agent_cmd`, and the TUI-only `get_message` command is not an agent tool.
use crate::application::tools::ports::Tool;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::sync::{Arc, Mutex};

/// A fake direct child: answers `get_messages` with a collapsed preview of
/// `full` (as the real history page does for an oversized message) and
/// `get_message` with `chunk`-byte ranges of it. Records every command.
fn fake_child(
    sock_path: std::path::PathBuf,
    full: String,
    chunk: usize,
    seen: Arc<Mutex<Vec<serde_json::Value>>>,
) {
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let full = full.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let (read_half, mut write_half) = tokio::io::split(stream);
                let mut reader = tokio::io::BufReader::new(read_half);
                let Ok(Some(payload)) = quecto_line_io::read_frame(
                    &mut reader,
                    quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
                )
                .await
                else {
                    return;
                };
                let cmd: serde_json::Value = serde_json::from_slice(&payload).unwrap();
                seen.lock().unwrap().push(cmd.clone());
                let data = match cmd["type"].as_str() {
                    Some("get_messages") => serde_json::json!({
                        "messages": [{
                            "id": "m-final", "role": "assistant", "ordinal": 1,
                            "content": full.chars().take(2048).collect::<String>(),
                            "collapsed": true, "truncated": true,
                            "contentLength": full.len(), "toolCalls": []
                        }],
                        "before": null, "hasMoreBefore": false
                    }),
                    Some("get_message") => {
                        let offset = cmd["offset"].as_u64().unwrap_or(0) as usize;
                        let end = (offset + chunk).min(full.len());
                        serde_json::json!({
                            "id": "m-final", "role": "assistant",
                            "content": &full[offset..end],
                            "offset": offset, "nextOffset": end,
                            "contentLength": full.len(), "hasMoreContent": end < full.len()
                        })
                    }
                    Some("get_report") => serde_json::json!({
                        "report": {
                            "messageId": "m-final",
                            "content": full.chars().take(8192).collect::<String>(),
                            "contentTruncated": true, "fullLengthBytes": full.len()
                        },
                        "recovery": {"command": "get_message", "messageId": "m-final", "offset": 8192},
                        "snapshot": true
                    }),
                    other => panic!("unexpected command {other:?}"),
                };
                let reply = serde_json::json!({
                    "type": "response", "id": cmd["id"], "success": true,
                    "command": cmd["type"], "data": data
                })
                .to_string();
                quecto_line_io::write_frame(
                    &mut write_half,
                    reply.as_bytes(),
                    quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
                )
                .await
                .unwrap();
            });
        }
    });
}

fn live_child(sock_path: std::path::PathBuf) -> SubagentEntry {
    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.persisted_liveness = crate::domain::session::SubagentLiveness::Live;
    entry
}

/// ASCII report text of `len` bytes that is not a repeat of its preview.
fn report_of(len: usize) -> String {
    (0..len)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect::<String>()
}

#[tokio::test]
async fn a_direct_childs_final_report_arrives_in_full() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let full = report_of(10_000);
    let seen = Arc::new(Mutex::new(Vec::new()));
    fake_child(sock.clone(), full.clone(), 4_000, seen.clone());
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child-uuid".to_string(), live_child(sock));
    let tool = AgentCmdTool::new(registry);

    let result = tool
        .execute(r#"{"agent_id":"child-uuid","command":"get_messages"}"#)
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.content);
    assert!(
        !result.content.contains("contentRecovery") && !result.content.contains("get_message\""),
        "no recovery step is offered to the agent: {}",
        result.content
    );
    let response: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let message = &response["data"]["messages"][0];
    assert_eq!(message["content"].as_str().unwrap(), full);
    assert_ne!(message["truncated"], true);
    // The full text is fetched from the child itself, never addressed to a
    // sub-agent of the child.
    let seen = seen.lock().unwrap();
    assert!(seen.iter().any(|cmd| cmd["type"] == "get_message"));
    assert!(
        seen.iter().all(|cmd| cmd.get("agent_id").is_none()),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_report_beyond_the_final_report_cap_is_cut_with_a_plain_notice() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let full = report_of(super::super::agent_cmd_report::FINAL_REPORT_BUDGET_BYTES + 200_000);
    let seen = Arc::new(Mutex::new(Vec::new()));
    fake_child(sock.clone(), full.clone(), 16_000, seen.clone());
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child-uuid".to_string(), live_child(sock));
    let tool = AgentCmdTool::new(registry);

    let result = tool
        .execute(r#"{"agent_id":"child-uuid","command":"get_messages"}"#)
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.content);
    assert!(
        !result.content.contains("contentRecovery"),
        "{}",
        result.content
    );
    let response: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let message = &response["data"]["messages"][0];
    let content = message["content"].as_str().unwrap();
    assert!(
        full.starts_with(content),
        "the kept part is the report's start"
    );
    assert!(content.len() > super::super::agent_cmd_report::FINAL_REPORT_BUDGET_BYTES / 2);
    assert_eq!(message["truncated"], true);
    assert_eq!(message["contentLength"], full.len());
    assert!(
        message["contentNotice"]
            .as_str()
            .is_some_and(|notice| notice.contains("export_raw")),
        "the agent is told how to get the rest: {message}"
    );
    // Reading stops just past the budget, not at the end of the report.
    let reads = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|cmd| cmd["type"] == "get_message")
        .count();
    assert!(
        reads <= super::super::agent_cmd_report::FINAL_REPORT_BUDGET_BYTES / 16_000 + 2,
        "{reads} reads"
    );
}

#[tokio::test]
async fn get_message_is_not_an_agent_cmd_command() {
    let tool = AgentCmdTool::new(new_registry());
    let result = tool
        .execute(r#"{"agent_id":"child-uuid","command":"get_message","messageId":"m1"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("unknown") || result.content.contains("unsupported"),
        "{}",
        result.content
    );
    let schema = tool.definition().parameters_schema;
    assert!(!schema.contains("\"get_message\""), "{schema}");
    assert!(!schema.contains("messageId"), "{schema}");
}

#[tokio::test]
async fn a_grandchilds_final_report_is_read_through_its_ancestor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("ancestor.sock");
    let full = report_of(9_000);
    let seen = Arc::new(Mutex::new(Vec::new()));
    fake_child(sock.clone(), full.clone(), 4_000, seen.clone());
    let registry = new_registry();
    let mut child = SubagentEntry::new(std::path::PathBuf::new(), 0);
    child.parent_id = Some("parent".to_string());
    child.persisted_liveness = crate::domain::session::SubagentLiveness::Live;
    let mut grandchild = SubagentEntry::new(std::path::PathBuf::new(), 0);
    grandchild.parent_id = Some("child".to_string());
    grandchild.persisted_liveness = crate::domain::session::SubagentLiveness::Live;
    {
        let mut entries = registry.lock().unwrap();
        entries.insert("parent".to_string(), live_child(sock));
        entries.insert("child".to_string(), child);
        entries.insert("grandchild".to_string(), grandchild);
    }
    let tool = AgentCmdTool::new(registry);

    let result = tool
        .execute(r#"{"agent_id":"grandchild","command":"get_messages"}"#)
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(
        response["data"]["messages"][0]["content"].as_str().unwrap(),
        full
    );
    let seen = seen.lock().unwrap();
    let reads: Vec<_> = seen
        .iter()
        .filter(|cmd| cmd["type"] == "get_message")
        .collect();
    assert!(!reads.is_empty());
    assert!(
        reads.iter().all(|cmd| cmd["agent_id"] == "grandchild"),
        "{reads:?}"
    );
}

#[tokio::test]
async fn get_report_hands_over_the_full_report_instead_of_a_recovery_reference() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sock = tmp.path().join("child.sock");
    let full = report_of(20_000);
    fake_child(
        sock.clone(),
        full.clone(),
        6_000,
        Arc::new(Mutex::new(Vec::new())),
    );
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child-uuid".to_string(), live_child(sock));
    let tool = AgentCmdTool::new(registry);

    let result = tool
        .execute(r#"{"agent_id":"child-uuid","command":"get_report"}"#)
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.content);
    assert!(
        !result.content.contains("get_message"),
        "{}",
        result.content
    );
    let response: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(
        response["data"]["report"]["content"].as_str().unwrap(),
        full
    );
    assert_eq!(response["data"]["report"]["contentTruncated"], false);
    assert!(response["data"].get("recovery").is_none());
}
