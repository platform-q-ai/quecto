//! Protocol event-line capping (#1047).
//!
//! A single oversized protocol message is dropped unread by every reader in
//! the workspace (they all bound reads at [`EVENT_LINE_CAP_BYTES`]), which
//! makes the session look frozen/disconnected. These helpers shrink an
//! over-budget event by tailing its growth-prone payload — the OLDEST content
//! is dropped, the most recent kept — so the line always stays receivable.
//!
//! Lives in the infrastructure layer (not `interface::cli::protocol`) because
//! the sub-agent monitor — which MUST NOT import `crate::interface`
//! (architecture rule) — needs to re-cap forwarded child lines that grow past
//! the cap when re-stamped with the child's identity. The protocol module
//! re-exports [`EVENT_LINE_CAP_BYTES`] for interface-side callers.

/// Hard cap on a single emitted event line, INCLUDING the trailing newline.
///
/// Derived from the shared framing crate's protocol bound so the emitter and
/// every reader (`quecto-tui`'s compatibility cap, the UDS and sub-agent read
/// bounds, `quecto-api`'s client bound) agree by construction (#1047).
pub const EVENT_LINE_CAP_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;

/// Serialized-JSON budget: the cap minus the trailing newline added on emit.
pub const EVENT_LINE_JSON_BUDGET: usize = EVENT_LINE_CAP_BYTES - 1;

/// Cap a serialized event line to [`EVENT_LINE_JSON_BUDGET`] when it exceeds
/// it; lines already within budget pass through byte-for-byte unmodified.
/// Event shapes with nothing safe to shrink are returned as-is (the caller's
/// reader will drop them — but nothing here can help).
pub fn cap_line(line: String) -> String {
    if line.len() <= EVENT_LINE_JSON_BUDGET {
        return line;
    }
    cap_event_json_line(&line, EVENT_LINE_JSON_BUDGET).unwrap_or(line)
}

/// Cap an over-budget serialized event by tailing its growth-prone payload.
/// Returns `None` for event shapes with nothing safe to shrink.
pub(crate) fn cap_event_json_line(line: &str, budget: usize) -> Option<String> {
    let mut v: serde_json::Value = serde_json::from_str(line).ok()?;
    loop {
        let s = serde_json::to_string(&v).ok()?;
        if s.len() <= budget {
            return Some(s);
        }
        if !shrink_event_payload(&mut v, s.len() - budget) {
            return None;
        }
    }
}

/// Shrink the event's payload by roughly `excess` bytes of content, keeping
/// the most recent output. JSON escaping means the byte accounting is
/// approximate, so the caller re-serializes and loops. Returns `false` when
/// nothing further can be removed.
fn shrink_event_payload(v: &mut serde_json::Value, excess: usize) -> bool {
    match v.get("type").and_then(serde_json::Value::as_str) {
        Some("turn_end") => tail_string_field(v.pointer_mut("/message/content"), excess),
        Some("token") => tail_string_field(v.pointer_mut("/token"), excess),
        Some("agent_end") | Some("subagent_messages_appended") => {
            shrink_messages_array(v.get_mut("messages"), excess)
        }
        // A live `response` carrying the whole conversation (an uncounted
        // `get_messages` near a full context window) grows without bound just
        // like `agent_end`; tail its messages array the same way instead of
        // emitting a line the client drops unread (#1047 review).
        Some("response") => shrink_messages_array(v.pointer_mut("/data/messages"), excess),
        _ => false,
    }
}

/// Drop roughly `excess` bytes of the OLDEST messages from a conversation
/// array (keeping at least one), or tail/drop a lone remaining message.
///
/// The prefix to drop is sized in ONE pass using each dropped message's own
/// serialized length — not one full-event re-serialization per dropped
/// message, which was quadratic in conversation size (#1051 review).
fn shrink_messages_array(field: Option<&mut serde_json::Value>, excess: usize) -> bool {
    let Some(serde_json::Value::Array(messages)) = field else {
        return false;
    };
    if messages.len() > 1 {
        let mut removed = 0usize;
        let mut cut = 0usize;
        while cut < messages.len() - 1 && removed < excess {
            // Element serialization is exactly its bytes in the array, so
            // this accounting is exact (+1 for the separator).
            removed += serde_json::to_string(&messages[cut]).map_or(1, |s| s.len() + 1);
            cut += 1;
        }
        messages.drain(..cut);
        return true;
    }
    let Some(last) = messages.first_mut() else {
        return false;
    };
    if tail_string_field(last.pointer_mut("/content"), excess) {
        return true;
    }
    // Last resort: a lone untailable message (e.g. structured
    // content blocks) is dropped whole so the event stays receivable.
    messages.clear();
    true
}

/// Keep the TAIL of the string (the most recent output), dropping at least
/// `excess` bytes from the front on a char boundary.
fn tail_string_field(field: Option<&mut serde_json::Value>, excess: usize) -> bool {
    let Some(serde_json::Value::String(s)) = field else {
        return false;
    };
    if s.is_empty() {
        return false;
    }
    let mut cut = excess.min(s.len());
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    *s = s.split_off(cut);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(content: &str) -> serde_json::Value {
        serde_json::json!({ "role": "user", "content": content })
    }

    /// #1051 review: a `response` carrying a conversation-scale `data.messages`
    /// array (live uncounted `get_messages`) must be tailed under the cap, not
    /// emitted oversized for the client to drop unread.
    #[test]
    fn response_messages_are_tailed_under_the_cap() {
        let chunk = 64 * 1024;
        let count = EVENT_LINE_JSON_BUDGET / chunk + 8;
        let messages: Vec<_> = (0..count)
            .map(|i| msg(&format!("{i}-{}", "x".repeat(chunk))))
            .collect();
        let line = serde_json::to_string(&serde_json::json!({
            "type": "response",
            "command": "get_messages",
            "success": true,
            "data": { "messages": messages },
        }))
        .unwrap();
        assert!(line.len() > EVENT_LINE_JSON_BUDGET);
        let capped = cap_line(line);
        assert!(capped.len() <= EVENT_LINE_JSON_BUDGET);
        let v: serde_json::Value = serde_json::from_str(&capped).unwrap();
        let kept = v["data"]["messages"].as_array().unwrap();
        assert!(!kept.is_empty(), "the most recent messages must survive");
        let last = kept.last().unwrap()["content"].as_str().unwrap();
        assert!(
            last.starts_with(&format!("{}-", count - 1)),
            "the newest message must be kept"
        );
    }

    /// #1051 review: capping a many-message payload must size the dropped
    /// prefix in one pass (exact element accounting), so a single shrink call
    /// lands under budget instead of one full re-serialization per message.
    #[test]
    fn oversized_messages_array_is_bulk_dropped_in_one_shrink_call() {
        let chunk = 1024;
        let count = EVENT_LINE_JSON_BUDGET / chunk + 100;
        let messages: Vec<_> = (0..count)
            .map(|i| msg(&format!("{i}-{}", "y".repeat(chunk))))
            .collect();
        let mut v = serde_json::json!({ "type": "agent_end", "messages": messages });
        let len = serde_json::to_string(&v).unwrap().len();
        assert!(len > EVENT_LINE_JSON_BUDGET);
        assert!(shrink_event_payload(&mut v, len - EVENT_LINE_JSON_BUDGET));
        let after = serde_json::to_string(&v).unwrap();
        assert!(
            after.len() <= EVENT_LINE_JSON_BUDGET,
            "one shrink call must remove the whole excess; got {} bytes",
            after.len()
        );
        let kept = v["messages"].as_array().unwrap();
        assert!(!kept.is_empty());
        assert!(
            kept.last().unwrap()["content"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{}-", count - 1)),
            "the newest messages must be kept"
        );
    }

    /// A lone over-budget message still falls back to content tailing.
    #[test]
    fn single_message_falls_back_to_content_tailing() {
        let mut v = serde_json::json!({
            "type": "agent_end",
            "messages": [msg(&"z".repeat(4096))],
        });
        assert!(shrink_event_payload(&mut v, 4000));
        let content = v["messages"][0]["content"].as_str().unwrap();
        assert!(
            content.len() <= 96,
            "content must be tailed: {}",
            content.len()
        );
    }

    /// Events without a shrinkable shape are left alone (and reported so).
    #[test]
    fn unknown_event_shapes_are_not_shrunk() {
        let mut v = serde_json::json!({ "type": "state_changed", "blob": "x" });
        assert!(!shrink_event_payload(&mut v, 10));
    }
}
