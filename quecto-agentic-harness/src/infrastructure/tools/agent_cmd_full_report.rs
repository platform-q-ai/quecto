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
use super::super::agent_cmd_report::{
    FINAL_REPORT_BUDGET_BYTES, FINAL_REPORT_NOTICE, NamedReport, holds_whole,
    is_substantive_assistant,
};
use super::*;

/// Bound on ranged reads for one message.
const MAX_RANGE_READS: usize = 64;
/// Bound on the whole read of one message, however many ranges it takes.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

/// Shown when the child could not be reached to read the rest (retried).
pub(crate) const READ_FAILED_NOTICE: &str = "Only a preview of this report could be read: the child did not answer the read in time. Call agent_cmd get_messages again later, or get_report with export_raw true to write the full history to an artifact.";

/// Shown when the child refused the read (it will not succeed on retry).
pub(crate) const READ_REFUSED_NOTICE: &str = "Only a preview of this report is available: the child could not serve the rest. Call agent_cmd get_report with export_raw true to write the full history to an artifact you can read.";

/// Errors a child answers that will not change on retry (the message is
/// gone, or cannot be framed). Any other refusal — a busy ancestor's
/// "capacity exhausted", say — is treated as a moment's unavailability.
const PERMANENT_READ_ERRORS: [&str; 2] = ["message not found", "exceeds the protocol frame limit"];

/// The unread ordinals `from..=to` a later read skipped, as ranges around
/// the report it delivered by id (`report`), which is not skipped.
fn skipped_ranges(from: u64, to: u64, report: Option<u64>) -> Vec<serde_json::Value> {
    let range = |from: u64, to: u64| {
        (from <= to).then(|| serde_json::json!({"fromOrdinal": from, "toOrdinal": to}))
    };
    match report {
        Some(ordinal) if (from..=to).contains(&ordinal) => {
            [range(from, ordinal - 1), range(ordinal + 1, to)]
                .into_iter()
                .flatten()
                .collect()
        }
        _ => range(from, to).into_iter().collect(),
    }
}

/// How a page holds a message's text (#2226 review 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// All of it.
    Whole,
    /// A preview of a message too large for the page: the start of its
    /// text, marked `truncated`.
    Preview,
    /// A context-collapsed recall stub (`collapsed` alone): none of it.
    Stub,
}

fn held(message: &serde_json::Value) -> Held {
    let flag = |name: &str| message.get(name).and_then(|v| v.as_bool());
    match (flag("truncated"), flag("collapsed")) {
        (Some(true), _) => Held::Preview,
        (None | Some(false), Some(true)) => Held::Stub,
        (None | Some(false), None | Some(false)) => Held::Whole,
    }
}

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

/// What the default read's backfill cap decides on (#2226).
pub(crate) struct CapRead<'a> {
    /// The supervisor's watermark: 0 on a first read.
    pub delivered: u64,
    /// The oldest page's cursor, from which skipped history can be paged.
    pub before: Option<serde_json::Value>,
    /// The report the child named, unread and off its newest page.
    pub named: Option<&'a NamedReport>,
    /// A named report a first read failed to read: still owed.
    pub owed: Option<&'a NamedReport>,
}

/// What the cap decided: whether the read is complete (acknowledgeable),
/// whether a first read found a report to deliver, and the unread history
/// the read skipped.
pub(crate) struct CapOutcome {
    pub complete: bool,
    pub report_found: bool,
    pub skipped: Option<serde_json::Value>,
}

impl AgentCmdTool {
    /// The report the child named, read whole by id (never a stand-in).
    pub(super) async fn read_named_report(
        &self,
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        report: &NamedReport,
    ) -> Result<serde_json::Value, ReadFailure> {
        Self::read_message_text(
            socket_path,
            routed_target_id,
            &report.id,
            FINAL_REPORT_BUDGET_BYTES,
        )
        .await
        .map(|(text, length)| report.with_text(text, length))
    }

    /// The default read reached its backfill cap with `messages` (#2226):
    /// - a named report a first read failed to read is still owed, so the
    ///   read is incomplete and acknowledges nothing (the next read retries);
    /// - a first read still looking for an answer reports the latest reply
    ///   it holds; holding none (a child that never replied, #2246), it
    ///   delivers the newest window it holds, says no report was found, and
    ///   names the unread history it skipped, so it is acknowledged and the
    ///   next read moves on instead of paging back for ever;
    /// - a later read delivers the newest window it holds and the named
    ///   report (read by id when not held whole), naming the unread history
    ///   it skipped; a named report it cannot read leaves it incomplete.
    pub(super) async fn at_backfill_cap(
        &self,
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        messages: &mut Vec<serde_json::Value>,
        read: CapRead<'_>,
    ) -> CapOutcome {
        let incomplete = CapOutcome {
            complete: false,
            report_found: true,
            skipped: None,
        };
        if read.owed.is_some() {
            return incomplete;
        }
        let oldest = messages
            .iter()
            .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
            .min();
        let skipped = |delivered_report: Option<u64>| {
            let to = oldest.map_or(read.delivered, |ordinal| ordinal.saturating_sub(1));
            let ranges = skipped_ranges(read.delivered + 1, to, delivered_report);
            Some(serde_json::json!({"ranges": ranges, "before": read.before}))
        };
        if read.delivered == 0 {
            let report_found = messages.iter().any(is_substantive_assistant);
            return CapOutcome {
                complete: true,
                report_found,
                skipped: if report_found { None } else { skipped(None) },
            };
        }
        let mut delivered_report = None;
        if let Some(report) = read.named.filter(|r| !holds_whole(messages, &r.id)) {
            match self
                .read_named_report(socket_path, routed_target_id, report)
                .await
            {
                Ok(whole) => {
                    delivered_report = whole.get("ordinal").and_then(|v| v.as_u64());
                    report.place(messages, whole);
                }
                Err(_) => return incomplete,
            }
        }
        CapOutcome {
            complete: true,
            report_found: true,
            skipped: skipped(delivered_report),
        }
    }

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
                    let error = reply.get("error").and_then(|v| v.as_str()).unwrap_or("");
                    let permanent = PERMANENT_READ_ERRORS
                        .iter()
                        .any(|known| error.contains(known));
                    return Err(if permanent { Refused } else { Unreachable });
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

    /// For a default `get_messages` response: when the unread report (the
    /// message the report planner delivers whole) is not held whole — a page
    /// preview or a context-collapsed stub — replace it with its text read
    /// by id. A failed read of a stub withholds it and marks the response
    /// incomplete (#2226 review 4); of a preview, only an unreachable child
    /// does, so the report is not acknowledged and a later read tries again.
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
        // The message the report planner delivers whole: the report among
        // the unread messages (#2226), by the planner's own rule.
        let unread_indices: Vec<usize> = (0..messages.len())
            .filter(|&i| unread(&messages[i]))
            .collect();
        let unread_messages: Vec<serde_json::Value> = unread_indices
            .iter()
            .map(|&i| messages[i].clone())
            .collect();
        let Some(index) = super::super::agent_cmd_report::report_position(&unread_messages)
            .map(|position| unread_indices[position])
        else {
            return response;
        };
        let Some(message_id) = messages[index]
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            return response;
        };
        // A report held whole is delivered as it is. Otherwise it is a page
        // preview (`truncated`: the start of its text) or a context-collapsed
        // recall stub (`collapsed` alone: not its text at all, #2226 review
        // 4), and its text is read from the child by id.
        let holding = held(&messages[index]);
        if holding == Held::Whole {
            return response;
        }
        let read = Self::read_message_text(
            socket_path,
            routed_target_id,
            &message_id,
            FINAL_REPORT_BUDGET_BYTES,
        )
        .await;
        let message = &mut messages[index];
        let unreachable = read == Err(ReadFailure::Unreachable);
        let read_failed = read.is_err();
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
                debug_assert!(!whole || held(message) == Held::Whole);
                if !whole {
                    message["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
                }
            }
            Err(failure) => {
                message["contentNotice"] = serde_json::json!(failure.notice());
            }
        }
        // A stub is never delivered as the report: an unread one leaves the
        // read incomplete (unacknowledged), and is withheld. Of a preview
        // only an unreachable child is worth reading again; a refusal would
        // repeat, so the preview is final.
        let failed_stub = holding == Held::Stub && read_failed;
        if failed_stub {
            messages.remove(index);
        }
        if (unreachable || failed_stub)
            && let Some(data) = envelope.get_mut("data")
        {
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
