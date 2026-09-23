//! #2114: a finished child's long report reaches the parent whole.
//!
//! The child's history page carries a very large message as a collapsed
//! preview, and `get_report` cuts its preview at 8 KiB with a `get_message`
//! recovery reference. `get_message` is a client (TUI) command, not an agent
//! one, so `agent_cmd` follows it here, in code: it reads the text from the
//! child in ranges, up to just past the final-report budget, and hands the
//! agent the text — never a recovery step.
//!
//! A read through an ancestor that is itself mid-turn is not served on the
//! ancestor's busy path, so it can time out; the report is then marked
//! incomplete (not acknowledged) and the agent is told to read it again.
use super::super::agent_cmd_report::{FINAL_REPORT_BUDGET_BYTES, FINAL_REPORT_NOTICE};
use super::*;

/// Bound on ranged reads for one message.
const MAX_RANGE_READS: usize = 64;
/// Bound on the whole read of one message, however many ranges it takes.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

/// Shown when the rest of a report could not be read from the child.
pub(crate) const READ_FAILED_NOTICE: &str = "Only a preview of this report could be read: the child did not answer the read in time. Call agent_cmd get_messages again later, or get_report with export_raw true to write the full history to an artifact.";

impl AgentCmdTool {
    /// The first `cap + 1` bytes (at most) of `message_id`'s text, read
    /// from the child in ranges: `(text, full content length)`. `None` when
    /// a read fails, a range is not the one asked for, or the whole read
    /// passes [`READ_DEADLINE`].
    async fn read_message_text(
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        message_id: &str,
        cap: usize,
    ) -> Option<(String, usize)> {
        let read = async {
            let mut text = String::new();
            for _ in 0..MAX_RANGE_READS {
                let offset = text.len();
                let mut cmd = serde_json::json!({
                    "type": "get_message", "messageId": message_id,
                    "offset": offset, "limit": cap + 1 - offset
                });
                // Only a grandchild read through its ancestor names a target;
                // a direct child is the socket's own session.
                if let Some(target_id) = routed_target_id {
                    cmd["agent_id"] = serde_json::json!(target_id);
                }
                let line = send_uds_command_with_timeout(
                    socket_path,
                    &cmd.to_string(),
                    super::super::subagent_registry::INSPECTOR_RESPONSE_TIMEOUT,
                )
                .await
                .ok()?;
                let reply: serde_json::Value = serde_json::from_str(&line).ok()?;
                if reply.get("success").and_then(|v| v.as_bool()) != Some(true) {
                    return None;
                }
                let data = reply.get("data")?;
                // The range must be the one asked for, and say what follows.
                if data.get("offset").and_then(|v| v.as_u64()) != Some(offset as u64) {
                    return None;
                }
                let chunk = data.get("content").and_then(|v| v.as_str())?;
                let length = data.get("contentLength").and_then(|v| v.as_u64())? as usize;
                let more = data.get("hasMoreContent").and_then(|v| v.as_bool())?;
                text.push_str(chunk);
                if text.len() > length {
                    return None;
                }
                if !more || text.len() > cap {
                    return Some((text, length));
                }
                if chunk.is_empty() {
                    return None;
                }
            }
            None
        };
        tokio::time::timeout(READ_DEADLINE, read)
            .await
            .ok()
            .flatten()
    }

    /// For a default `get_messages` response: when the unread final
    /// assistant message (the one the report planner delivers whole) is a
    /// collapsed preview, replace it with its text. A failed read marks the
    /// response incomplete, so the report is not acknowledged and a later
    /// read tries again.
    pub(super) async fn expand_collapsed_final_report(
        &self,
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        response: String,
        agent_id: &str,
    ) -> String {
        let delivered = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            super::super::subagent_registry::resolve_registry_key(&entries, agent_id)
                .ok()
                .and_then(|key| entries.get(&key).and_then(|e| e.delivered_message_ordinal))
                .unwrap_or(0)
        };
        let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(&response) else {
            return response;
        };
        let Some(messages) = envelope
            .pointer_mut("/data/messages")
            .and_then(|v| v.as_array_mut())
        else {
            return response;
        };
        let unread = |m: &serde_json::Value| {
            m.get("ordinal")
                .and_then(|v| v.as_u64())
                .is_none_or(|ordinal| ordinal > delivered)
        };
        let Some(index) = messages.iter().rposition(|m| {
            unread(m) && super::super::agent_cmd_report::is_substantive_assistant(m)
        }) else {
            return response;
        };
        if messages[index].get("truncated").and_then(|v| v.as_bool()) != Some(true) {
            return response;
        }
        let Some(message_id) = messages[index]
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            return response;
        };
        let read = Self::read_message_text(
            socket_path,
            routed_target_id,
            &message_id,
            FINAL_REPORT_BUDGET_BYTES,
        )
        .await;
        let message = &mut messages[index];
        let failed = read.is_none();
        match read {
            Some((text, length)) => {
                let whole = text.len() == length;
                message["content"] = serde_json::json!(text);
                message["contentLength"] = serde_json::json!(length);
                if let Some(obj) = message.as_object_mut() {
                    obj.remove("collapsed");
                    obj.remove("contentRecovery");
                    if whole {
                        obj.remove("truncated");
                    }
                }
                if !whole {
                    message["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
                }
            }
            None => {
                message["contentNotice"] = serde_json::json!(READ_FAILED_NOTICE);
            }
        }
        if failed && let Some(data) = envelope.get_mut("data") {
            data["reportIncomplete"] = serde_json::json!(true);
        }
        envelope.to_string()
    }

    /// `get_report`: replace a truncated preview and its `get_message`
    /// recovery reference with the report's text, serialized within the
    /// final-report budget.
    pub(super) async fn expand_report(
        &self,
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        response: String,
    ) -> String {
        let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(&response) else {
            return response;
        };
        let Some(data) = envelope.get_mut("data") else {
            return response;
        };
        let Some(recovery) = data.get("recovery").filter(|v| !v.is_null()).cloned() else {
            return response;
        };
        if let Some(obj) = data.as_object_mut() {
            obj.remove("recovery");
        }
        if !data.get("report").is_some_and(serde_json::Value::is_object) {
            return envelope.to_string();
        }
        let message_id = recovery
            .get("messageId")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let read = if message_id.is_empty() {
            None
        } else {
            Self::read_message_text(
                socket_path,
                routed_target_id,
                message_id,
                FINAL_REPORT_BUDGET_BYTES,
            )
            .await
        };
        let report = &mut data["report"];
        match read {
            Some((text, length)) => {
                let kept = serialized_prefix(&text, FINAL_REPORT_BUDGET_BYTES);
                let whole = kept.len() == length;
                report["content"] = serde_json::json!(kept);
                report["contentTruncated"] = serde_json::json!(!whole);
                if !whole {
                    report["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
                }
            }
            None => {
                report["contentNotice"] = serde_json::json!(READ_FAILED_NOTICE);
            }
        }
        envelope.to_string()
    }
}

/// The longest prefix of `text` (on a character boundary) whose JSON string
/// encoding fits in `budget` bytes.
pub(crate) fn serialized_prefix(text: &str, budget: usize) -> &str {
    let encoded = |s: &str| {
        serde_json::to_string(s)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
    };
    if encoded(text) <= budget {
        return text;
    }
    let (mut low, mut high) = (0usize, text.len());
    while low < high {
        let mut mid = (low + high).div_ceil(2);
        while !text.is_char_boundary(mid) {
            mid -= 1;
        }
        if mid <= low {
            break;
        }
        if encoded(&text[..mid]) <= budget {
            low = mid;
        } else {
            high = mid - 1;
            while !text.is_char_boundary(high) {
                high -= 1;
            }
        }
    }
    &text[..low]
}
