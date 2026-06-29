//! Message normalization for the Anthropic provider.
//!
//! Handles tool call ID sanitization, filtering of error/aborted assistant
//! turns, orphaned tool result injection, and clone-on-write message
//! forwarding (#184, #182, #374).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::domain::message::{Message, Role, StopReason};

/// Check whether a tool call ID is already valid (only `[a-zA-Z0-9_-]`, ≤64 chars).
fn is_valid_tool_call_id(id: &str) -> bool {
    id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Sanitize a tool call ID to Anthropic's allowed character set:
/// `[a-zA-Z0-9_-]`, max 64 chars.  Invalid characters are replaced with `_`.
///
/// Returns `None` if the ID is already valid (avoids allocation).
pub(super) fn normalize_tool_call_id(id: &str) -> Option<String> {
    if is_valid_tool_call_id(id) {
        return None;
    }
    Some(
        id.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect(),
    )
}

/// Normalize messages: strip invalid tool call IDs, filter error/aborted
/// assistant turns and their orphaned tool_result counterparts (#184, #182).
///
/// Returns `Cow::Borrowed` for messages that need no modification (the
/// common case — user/system messages and tool results with already-valid
/// IDs).  Only messages whose tool call IDs actually change are cloned.
pub(super) fn normalize_messages(messages: &[Message]) -> Vec<Cow<'_, Message>> {
    // Collect IDs from dropped assistant turns (error/aborted) so we can
    // also drop their orphaned tool_result counterparts.
    let is_incomplete = |m: &&Message| {
        m.role == Role::Assistant
            && matches!(
                m.stop_reason,
                Some(StopReason::Error) | Some(StopReason::Aborted)
            )
    };
    let dropped_tool_ids: HashSet<&str> = messages
        .iter()
        .filter(is_incomplete)
        .flat_map(|m| m.tool_calls.iter().map(|tc| tc.id.as_str()))
        .collect();

    // Build a map from original tool call ID → normalised ID, but only
    // for IDs that actually change.  normalize_tool_call_id returns None
    // for already-valid IDs (no allocation).
    let id_map: HashMap<&str, String> = messages
        .iter()
        .flat_map(|m| m.tool_calls.iter())
        .filter_map(|tc| normalize_tool_call_id(&tc.id).map(|norm| (tc.id.as_str(), norm)))
        .collect();

    messages
        .iter()
        .filter(|m| {
            // Drop incomplete assistant turns (error or aborted).
            if m.role == Role::Assistant
                && matches!(
                    m.stop_reason,
                    Some(StopReason::Error) | Some(StopReason::Aborted)
                )
            {
                return false;
            }
            // Drop tool results whose tool call was dropped above.
            if m.role == Role::Tool {
                if let Some(id) = &m.tool_call_id {
                    if dropped_tool_ids.contains(id.as_str()) {
                        return false;
                    }
                }
            }
            true
        })
        .map(|m| {
            // Check if any IDs in this message need normalization.
            let tc_needs_norm = m
                .tool_calls
                .iter()
                .any(|tc| id_map.contains_key(tc.id.as_str()));
            let tcid_needs_norm = m.role == Role::Tool
                && m.tool_call_id
                    .as_ref()
                    .is_some_and(|id| id_map.contains_key(id.as_str()));

            if !tc_needs_norm && !tcid_needs_norm {
                return Cow::Borrowed(m);
            }

            let mut out = m.clone();
            // Normalise IDs in tool_use blocks (assistant messages).
            for tc in &mut out.tool_calls {
                if let Some(norm) = id_map.get(tc.id.as_str()) {
                    tc.id.clone_from(norm);
                }
            }
            // Normalise IDs in tool_result blocks (tool messages).
            if m.role == Role::Tool {
                if let Some(orig) = &m.tool_call_id {
                    if let Some(norm) = id_map.get(orig.as_str()) {
                        out.tool_call_id = Some(norm.clone());
                    }
                }
            }
            Cow::Owned(out)
        })
        .collect()
}

/// Collect all tool_use IDs from assistant messages in the API payload.
pub(super) fn collect_tool_use_ids(api_messages: &[serde_json::Value]) -> Vec<String> {
    api_messages
        .iter()
        .filter(|m| m["role"] == "assistant")
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_use")
        .filter_map(|b| b["id"].as_str().map(str::to_string))
        .collect()
}

/// Collect all tool_result IDs from the API payload.
pub(super) fn collect_tool_result_ids(api_messages: &[serde_json::Value]) -> HashSet<String> {
    api_messages
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_result")
        .filter_map(|b| b["tool_use_id"].as_str().map(str::to_string))
        .collect()
}

/// Synthetic `tool_result` for an orphaned tool call (no non-standard fields).
pub(super) fn synthetic_tool_result(tool_use_id: String) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": "No result provided",
        "is_error": true,
    })
}

/// Detect orphaned tool calls in `api_messages` (tool_use blocks without a
/// matching tool_result) and inject synthetic error results.
///
/// A tool call is orphaned when an interrupted session has an assistant
/// message with tool_use blocks but no subsequent tool result messages.
/// Sending such a payload to Anthropic causes an API error.
pub(super) fn inject_orphaned_tool_results(api_messages: &mut Vec<serde_json::Value>) {
    let pending = collect_tool_use_ids(api_messages);
    let satisfied = collect_tool_result_ids(api_messages);

    let mut synthetic_blocks: Vec<serde_json::Value> = pending
        .into_iter()
        .filter(|id| !satisfied.contains(id))
        .map(synthetic_tool_result)
        .collect();

    if synthetic_blocks.is_empty() {
        return;
    }

    // Append into the last user message only if it already contains
    // tool_result blocks (not a plain text user message — mixing them
    // would produce an invalid payload).
    if let Some(last) = api_messages.last_mut() {
        if last["role"] == "user" {
            let has_tool_results = last["content"]
                .as_array()
                .map(|arr| arr.iter().any(|b| b["type"] == "tool_result"))
                .unwrap_or(false);
            if has_tool_results {
                if let Some(arr) = last["content"].as_array_mut() {
                    arr.append(&mut synthetic_blocks);
                    return;
                }
            }
        }
    }
    api_messages.push(serde_json::json!({
        "role": "user",
        "content": synthetic_blocks,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{Message, Role, StopReason, ToolCall};

    // --- is_valid_tool_call_id / normalize_tool_call_id ---

    #[test]
    fn valid_id_returns_none() {
        assert!(normalize_tool_call_id("abc123_-XYZ").is_none());
    }

    #[test]
    fn invalid_chars_replaced() {
        let norm = normalize_tool_call_id("call.123!@#").unwrap();
        assert!(
            norm.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        );
    }

    #[test]
    fn long_id_truncated() {
        let long = "a".repeat(100);
        let norm = normalize_tool_call_id(&long).unwrap();
        assert!(norm.len() <= 64);
    }

    #[test]
    fn empty_id_is_valid() {
        assert!(normalize_tool_call_id("").is_none());
    }

    // --- normalize_messages ---

    #[test]
    fn normal_messages_borrowed() {
        let msgs = vec![
            Message::user("hello".to_string()),
            Message::assistant("hi".to_string(), vec![]),
        ];
        let normalized = normalize_messages(&msgs);
        assert_eq!(normalized.len(), 2);
        assert!(matches!(normalized[0], std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn error_assistant_turn_dropped() {
        let mut assistant = Message::assistant("partial".to_string(), vec![]);
        assistant.stop_reason = Some(StopReason::Error);
        let msgs = vec![Message::user("hello".to_string()), assistant];
        let normalized = normalize_messages(&msgs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].role, Role::User);
    }

    #[test]
    fn aborted_assistant_turn_dropped() {
        let mut assistant = Message::assistant("partial".to_string(), vec![]);
        assistant.stop_reason = Some(StopReason::Aborted);
        let tc = ToolCall {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            arguments: "{}".to_string(),
        };
        assistant.tool_calls = vec![tc];
        let mut tool_result = Message::tool("call-1", "result");
        tool_result.tool_call_id = Some("call-1".to_string());
        let msgs = vec![Message::user("hello".to_string()), assistant, tool_result];
        let normalized = normalize_messages(&msgs);
        // Both the aborted assistant and its orphaned tool result should be dropped
        assert_eq!(normalized.len(), 1);
    }

    #[test]
    fn invalid_tool_call_id_normalized() {
        let mut assistant = Message::assistant("calling".to_string(), vec![]);
        assistant.tool_calls = vec![ToolCall {
            id: "call.123!@#".to_string(),
            name: "bash".to_string(),
            arguments: "{}".to_string(),
        }];
        let msgs = vec![assistant];
        let normalized = normalize_messages(&msgs);
        assert_eq!(normalized.len(), 1);
        let tc_id = &normalized[0].tool_calls[0].id;
        assert!(
            tc_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        );
    }

    // --- collect_tool_use_ids / collect_tool_result_ids ---

    #[test]
    fn collect_tool_use_ids_finds_all() {
        let msgs = vec![serde_json::json!({
            "role": "assistant",
            "content": [
                {"type": "tool_use", "id": "call-1", "name": "bash", "input": {}},
                {"type": "tool_use", "id": "call-2", "name": "read", "input": {}},
            ]
        })];
        let ids = collect_tool_use_ids(&msgs);
        assert_eq!(ids, vec!["call-1", "call-2"]);
    }

    #[test]
    fn collect_tool_use_ids_empty() {
        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        assert!(collect_tool_use_ids(&msgs).is_empty());
    }

    #[test]
    fn collect_tool_result_ids_finds_all() {
        let msgs = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": "call-1", "content": "ok"},
                {"type": "tool_result", "tool_use_id": "call-2", "content": "ok"},
            ]
        })];
        let ids = collect_tool_result_ids(&msgs);
        assert!(ids.contains("call-1"));
        assert!(ids.contains("call-2"));
    }

    // --- synthetic_tool_result ---

    #[test]
    fn synthetic_result_has_required_fields() {
        let result = synthetic_tool_result("call-42".to_string());
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], "call-42");
        assert_eq!(result["is_error"], true);
    }

    // --- inject_orphaned_tool_results ---

    #[test]
    fn inject_no_orphans_is_noop() {
        let mut msgs = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "call-1", "name": "bash", "input": {}}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "call-1", "content": "ok"}]
            }),
        ];
        let len_before = msgs.len();
        inject_orphaned_tool_results(&mut msgs);
        assert_eq!(msgs.len(), len_before);
    }

    #[test]
    fn inject_orphaned_creates_synthetic() {
        let mut msgs = vec![serde_json::json!({
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "orphan-1", "name": "bash", "input": {}}]
        })];
        inject_orphaned_tool_results(&mut msgs);
        // Should have added a user message with synthetic tool_result
        assert_eq!(msgs.len(), 2);
        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content[0]["tool_use_id"], "orphan-1");
        assert_eq!(content[0]["is_error"], true);
    }

    #[test]
    fn inject_orphaned_appends_to_existing_user_tool_results() {
        let mut msgs = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "call-1", "name": "a", "input": {}},
                    {"type": "tool_use", "id": "call-2", "name": "b", "input": {}},
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "call-1", "content": "ok"}]
            }),
        ];
        inject_orphaned_tool_results(&mut msgs);
        // Should have appended synthetic result to existing user message
        assert_eq!(msgs.len(), 2);
        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2); // original + synthetic
        assert_eq!(content[1]["tool_use_id"], "call-2");
    }
}
