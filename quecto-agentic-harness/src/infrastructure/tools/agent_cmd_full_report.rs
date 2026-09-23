//! #2114: a finished child's long report reaches the parent whole.
//!
//! The child's history page carries an oversized message as a short
//! collapsed preview, and `get_report` cuts its preview at 8 KiB with a
//! `get_message` recovery reference. `get_message` is a client (TUI)
//! command, not an agent one, so `agent_cmd` follows it here, in code:
//! it reads the full text from the child in ranges, up to the final-report
//! budget, and hands the agent the text — never a recovery step.
use super::super::agent_cmd_report::{FINAL_REPORT_BUDGET_BYTES, FINAL_REPORT_NOTICE};
use super::*;

/// Upper bound on ranged reads for one message (each range is a full
/// protocol frame, so this is far above the budget).
const MAX_RANGE_READS: usize = 256;

impl AgentCmdTool {
    /// The text of `message_id` read from the child in ranges, stopping once
    /// more than `cap` bytes are held: `(text, full content length)`. `None`
    /// when a read fails or makes no progress.
    async fn read_message_text(
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        message_id: &str,
        cap: usize,
    ) -> Option<(String, usize)> {
        let mut text = String::new();
        let mut offset = 0usize;
        for _ in 0..MAX_RANGE_READS {
            let mut cmd = serde_json::json!({
                "type": "get_message", "messageId": message_id, "offset": offset
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
            text.push_str(data.get("content").and_then(|v| v.as_str())?);
            let length = data.get("contentLength").and_then(|v| v.as_u64())? as usize;
            let next = data.get("nextOffset").and_then(|v| v.as_u64())? as usize;
            let more = data.get("hasMoreContent").and_then(|v| v.as_bool()) == Some(true);
            if !more || text.len() > cap {
                return Some((text, length));
            }
            if next <= offset {
                return None;
            }
            offset = next;
        }
        None
    }

    /// Replace the newest collapsed final assistant message of a default
    /// `get_messages` response with its full text (the planner then applies
    /// the final-report budget). A failed read leaves the preview, marked
    /// with the notice.
    pub(super) async fn expand_collapsed_final_report(
        &self,
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        response: String,
    ) -> String {
        let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(&response) else {
            return response;
        };
        let Some(messages) = envelope
            .pointer_mut("/data/messages")
            .and_then(|v| v.as_array_mut())
        else {
            return response;
        };
        let collapsed_final = messages.iter().rposition(|m| {
            m.get("role").and_then(|v| v.as_str()) == Some("assistant")
                && m.get("truncated").and_then(|v| v.as_bool()) == Some(true)
                && m.get("toolCalls")
                    .and_then(|v| v.as_array())
                    .is_none_or(Vec::is_empty)
                && m.get("id").and_then(|v| v.as_str()).is_some()
        });
        let Some(index) = collapsed_final else {
            return response;
        };
        let message_id = messages[index]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let read = Self::read_message_text(
            socket_path,
            routed_target_id,
            &message_id,
            FINAL_REPORT_BUDGET_BYTES,
        )
        .await;
        let message = &mut messages[index];
        match read {
            Some((text, length)) => {
                let whole = text.len() >= length;
                message["content"] = serde_json::json!(text);
                message["contentLength"] = serde_json::json!(length);
                if let Some(obj) = message.as_object_mut() {
                    obj.remove("collapsed");
                    obj.remove("contentRecovery");
                    if whole {
                        obj.remove("truncated");
                    }
                }
            }
            None => {
                message["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
            }
        }
        envelope.to_string()
    }

    /// `get_report`: replace a truncated preview and its `get_message`
    /// recovery reference with the report's full text (up to the budget).
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
                let mut end = text.len().min(FINAL_REPORT_BUDGET_BYTES);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                let whole = end >= length;
                report["content"] = serde_json::json!(&text[..end]);
                report["contentTruncated"] = serde_json::json!(!whole);
                if !whole {
                    report["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
                }
            }
            None => {
                report["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
            }
        }
        envelope.to_string()
    }
}
