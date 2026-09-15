//! Format 1 of a raw session export (#1859): how each record and the
//! manifest are encoded on disk. A message record carries the message in
//! the shape UDS clients read it (stable id, ordinal, role, content, tool
//! calls, tool result linkage, error and collapsed flags, display-safe
//! thinking); a spill record carries one retention entry with its content.
//! Values are built as JSON objects so keys are emitted in sorted order,
//! the same bytes whichever record built them.
use crate::application::sessions::dto::{ExportManifest, ExportRecord};
use crate::domain::message::{Message, ThinkingBlock};
use crate::domain::request_observation::RuntimeIdentity;
use crate::domain::session::SpillEntry;
use serde_json::{Value, json};

/// One record as one JSON object.
pub(super) fn record_json(record: &ExportRecord) -> Value {
    match record {
        ExportRecord::Message(message) => json!({"kind":"message","message":message_json(message)}),
        ExportRecord::Spill(spill) => spill_json(spill),
    }
}

fn message_json(message: &Message) -> Value {
    let mut value = json!({
        "id": message.id().to_string(),
        "ordinal": message.ordinal,
        "role": message.role.as_str(),
        "content": message.content,
        "toolCalls": message
            .tool_calls
            .iter()
            .map(|call| json!({"id":call.id,"name":call.name,"arguments":call.arguments}))
            .collect::<Vec<_>>(),
        "toolCallId": message.tool_call_id,
        "toolName": message.tool_name,
        "isError": message.is_error,
        "collapsed": message.is_collapsed,
    });
    if !message.thinking_blocks.is_empty() {
        value["thinking"] =
            Value::Array(message.thinking_blocks.iter().map(thinking_json).collect());
    }
    value
}

fn thinking_json(block: &ThinkingBlock) -> Value {
    match block {
        ThinkingBlock::Normal { thinking, .. } => json!({"kind":"text","text":thinking}),
        ThinkingBlock::Redacted { .. } => json!({"kind":"redacted"}),
    }
}

fn spill_json(spill: &SpillEntry) -> Value {
    json!({"kind":"spill","id":spill.id,"tool":spill.tool,"input_preview":spill.input_preview,
        "tokens":spill.tokens,"content":spill.content})
}

/// The manifest as one JSON object: what the use case stated plus the
/// checksum, size and writing runtime.
pub(super) fn manifest_json(
    manifest: &ExportManifest,
    sha256: &str,
    bytes: u64,
    runtime: &RuntimeIdentity,
) -> Value {
    json!({
        "format": ExportManifest::FORMAT,
        "epoch": manifest.epoch,
        "revision": manifest.revision,
        "recordCount": manifest.record_count,
        "spillCount": manifest.spill_count,
        "scope": ExportManifest::SCOPE,
        "spillConsistency": ExportManifest::SPILL_CONSISTENCY,
        "sha256": sha256,
        "bytes": bytes,
        "runtime": runtime,
    })
}
