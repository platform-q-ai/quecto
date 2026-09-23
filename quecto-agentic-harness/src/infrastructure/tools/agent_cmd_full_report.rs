//! #2114: a finished child's long report reaches the parent whole.
//!
//! The child's history page carries a very large message as a collapsed
//! preview, and `get_report` cuts its preview at 8 KiB with a `get_message`
//! recovery reference. `get_message` is a client (TUI) command, not an agent
//! one, so `agent_cmd` follows it here, in code: it reads the text from the
//! child in ranges, up to just past the final-report budget, and hands the
//! agent the text — never a recovery step.
//!
//! A read the child could not be reached for is retried: the report stays
//! incomplete (not acknowledged). A read the child refused, or answered
//! outside the protocol, would fail the same way again, so the preview is
//! delivered with a notice pointing at `export_raw`.
use super::super::agent_cmd_report::{FINAL_REPORT_BUDGET_BYTES, FINAL_REPORT_NOTICE};
use super::*;

/// Bound on ranged reads for one message.
const MAX_RANGE_READS: usize = 64;
/// Bound on the whole read of one message, however many ranges it takes.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

/// Shown when the child could not be reached to read the rest (retried).
pub(crate) const READ_FAILED_NOTICE: &str = "Only a preview of this report could be read: the child did not answer the read in time. Call agent_cmd get_messages again later, or get_report with export_raw true to write the full history to an artifact.";

/// Shown when the child refused the read (it will not succeed on retry).
pub(crate) const READ_REFUSED_NOTICE: &str = "Only a preview of this report is available: the child could not serve the rest. Call agent_cmd get_report with export_raw true to write the full history to an artifact you can read.";

/// Why the rest of a message could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadFailure {
    /// No answer (transport error or deadline): worth reading again.
    Unreachable,
    /// The child answered, but refused or broke the ranged-read protocol.
    Refused,
}

impl ReadFailure {
    fn notice(self) -> &'static str {
        match self {
            Self::Unreachable => READ_FAILED_NOTICE,
            Self::Refused => READ_REFUSED_NOTICE,
        }
    }
}

impl AgentCmdTool {
    /// The first `cap + 1` bytes (at most) of `message_id`'s text, read
    /// from the child in ranges: `(text, full content length)`.
    async fn read_message_text(
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        message_id: &str,
        cap: usize,
    ) -> Result<(String, usize), ReadFailure> {
        use ReadFailure::{Refused, Unreachable};
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
                .map_err(|_| Unreachable)?;
                let reply: serde_json::Value = serde_json::from_str(&line).map_err(|_| Refused)?;
                if reply.get("success").and_then(|v| v.as_bool()) != Some(true) {
                    return Err(Refused);
                }
                let data = reply.get("data").ok_or(Refused)?;
                // The range must be the one asked for, and say what follows.
                if data.get("offset").and_then(|v| v.as_u64()) != Some(offset as u64) {
                    return Err(Refused);
                }
                let chunk = data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or(Refused)?;
                let length = data
                    .get("contentLength")
                    .and_then(|v| v.as_u64())
                    .ok_or(Refused)? as usize;
                let more = data
                    .get("hasMoreContent")
                    .and_then(|v| v.as_bool())
                    .ok_or(Refused)?;
                text.push_str(chunk);
                if text.len() > length {
                    return Err(Refused);
                }
                if !more || text.len() > cap {
                    return Ok((text, length));
                }
                if chunk.is_empty() {
                    return Err(Refused);
                }
            }
            Err(Refused)
        };
        tokio::time::timeout(READ_DEADLINE, read)
            .await
            .unwrap_or(Err(Unreachable))
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
        let unreachable = read == Err(ReadFailure::Unreachable);
        match read {
            Ok((text, length)) => {
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
            Err(failure) => {
                message["contentNotice"] = serde_json::json!(failure.notice());
            }
        }
        // Only an unreachable child is worth reading again: keep the report
        // unacknowledged. A refusal would repeat, so the preview is final.
        if unreachable && let Some(data) = envelope.get_mut("data") {
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
            Err(ReadFailure::Refused)
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
            Ok((text, length)) => {
                let kept = serialized_prefix(&text, FINAL_REPORT_BUDGET_BYTES);
                let whole = kept.len() == length;
                report["content"] = serde_json::json!(kept);
                report["contentTruncated"] = serde_json::json!(!whole);
                if !whole {
                    report["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
                }
            }
            Err(failure) => {
                report["contentNotice"] = serde_json::json!(failure.notice());
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
