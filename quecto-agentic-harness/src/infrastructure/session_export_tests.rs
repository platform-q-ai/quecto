use super::*;
use crate::domain::message::{Message, ThinkingBlock, ToolCall};
use crate::domain::session::SpillEntry;

fn manifest() -> ExportManifest {
    ExportManifest {
        epoch: 3,
        revision: 7,
        record_count: 2,
        spill_count: 1,
    }
}

fn rich_message() -> Message {
    let mut message = Message::assistant(
        "thinking done",
        vec![ToolCall {
            id: "call-1".into(),
            name: "bash".into(),
            arguments: "{\"command\":\"ls\"}".into(),
        }],
    );
    message.ordinal = Some(9);
    message.thinking_blocks = vec![
        ThinkingBlock::Normal {
            thinking: "hmm".into(),
            signature: "sig".into(),
        },
        ThinkingBlock::Redacted {
            data: "opaque".into(),
        },
    ];
    message
}

#[tokio::test]
async fn an_export_writes_records_and_manifest_under_the_root_and_returns_the_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("artifacts/session-exports");
    let exporter = FileSessionExport::new(root.clone());
    let message = rich_message();
    let receipt = exporter
        .write_export(
            vec![
                ExportRecord::Message(Box::new(message.clone())),
                ExportRecord::Spill(SpillEntry {
                    id: "spill-1".into(),
                    tool: "grep".into(),
                    input_preview: "needle".into(),
                    tokens: 5,
                    content: "haystack".into(),
                }),
            ],
            manifest(),
        )
        .await
        .unwrap();
    assert!(
        receipt
            .records_path
            .starts_with(root.canonicalize().unwrap())
    );
    assert_eq!(
        receipt.manifest_path.parent(),
        receipt.records_path.parent()
    );
    let records = std::fs::read_to_string(&receipt.records_path).unwrap();
    let lines: Vec<&str> = records.lines().collect();
    assert_eq!(lines.len(), 2);
    let expected_message = serde_json::json!({"kind":"message","message":{
        "id": message.id().to_string(), "ordinal": 9, "role": "assistant",
        "content": "thinking done",
        "toolCalls": [{"id":"call-1","name":"bash","arguments":"{\"command\":\"ls\"}"}],
        "toolCallId": null, "toolName": null, "isError": false, "collapsed": false,
        "thinking": [{"kind":"text","text":"hmm"},{"kind":"redacted"}]
    }});
    assert_eq!(
        lines[0],
        expected_message.to_string(),
        "format 1 message record"
    );
    assert_eq!(
        lines[1],
        r#"{"content":"haystack","id":"spill-1","input_preview":"needle","kind":"spill","tokens":5,"tool":"grep"}"#
    );
    let mut hash = Sha256::new();
    hash.update(records.as_bytes());
    assert_eq!(receipt.sha256, format!("{:x}", hash.finalize()));
    assert_eq!(receipt.bytes, records.len() as u64);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["format"], 1);
    assert_eq!(manifest["epoch"], 3);
    assert_eq!(manifest["revision"], 7);
    assert_eq!(manifest["recordCount"], 2);
    assert_eq!(manifest["spillCount"], 1);
    assert_eq!(manifest["scope"], ExportManifest::SCOPE);
    assert_eq!(
        manifest["spillConsistency"],
        ExportManifest::SPILL_CONSISTENCY
    );
    assert_eq!(manifest["sha256"], receipt.sha256);
    assert_eq!(manifest["bytes"], receipt.bytes);
    assert!(manifest["runtime"]["packageVersion"].is_string() || manifest["runtime"].is_object());
}

#[tokio::test]
async fn a_plain_message_record_omits_thinking_and_keeps_tool_result_linkage() {
    let directory = tempfile::tempdir().unwrap();
    let exporter = FileSessionExport::new(directory.path().to_path_buf());
    let mut result = Message::user("ignored");
    result.role = crate::domain::message::Role::Tool;
    result.content = "out".into();
    result.tool_call_id = Some("call-1".into());
    result.tool_name = Some("bash".into());
    result.is_error = true;
    result.is_collapsed = true;
    let receipt = exporter
        .write_export(vec![ExportRecord::Message(Box::new(result))], manifest())
        .await
        .unwrap();
    let line = std::fs::read_to_string(&receipt.records_path).unwrap();
    let record: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(record["message"]["role"], "tool");
    assert_eq!(record["message"]["toolCallId"], "call-1");
    assert_eq!(record["message"]["toolName"], "bash");
    assert_eq!(record["message"]["isError"], true);
    assert_eq!(record["message"]["collapsed"], true);
    assert!(record["message"].get("thinking").is_none());
    assert_eq!(record["message"]["toolCalls"], serde_json::json!([]));
}

#[tokio::test]
async fn an_unwritable_root_fails_as_a_session_export_error_without_an_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("export");
    std::fs::write(&root, "occupied").unwrap();
    let error = FileSessionExport::new(root)
        .write_export(vec![], manifest())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("tool error: session export: "),
        "{error}"
    );
}

#[tokio::test]
async fn a_panicking_writer_task_is_reported_by_the_bare_runtime_text() {
    let error = tokio::task::spawn_blocking(|| panic!("writer exploded"))
        .await
        .unwrap_err();
    let expected = error.to_string();
    assert!(expected.contains("writer exploded"), "{expected}");
    let mapped = join_failure(error);
    assert_eq!(mapped.to_string(), expected, "no `tool error:` prefix");
    assert!(matches!(mapped, DomainError::Other(_)));
}

#[test]
fn an_export_over_the_size_bound_is_refused_and_its_directory_removed() {
    let directory = tempfile::tempdir().unwrap();
    // The bound is injected: the check is identical at any value and the
    // production one would mean serialising a 256 MiB message here.
    assert_eq!(MAX_EXPORT_BYTES, 256 * 1024 * 1024);
    let max_bytes = 2 * 1024 * 1024;
    let huge = Message::user("x".repeat(max_bytes as usize + 1));
    let error = write_bounded(
        directory.path(),
        &[ExportRecord::Message(Box::new(huge))],
        &manifest(),
        max_bytes,
    )
    .unwrap_err();
    assert!(error.to_string().contains("exceeds 2 MiB"), "{error}");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
