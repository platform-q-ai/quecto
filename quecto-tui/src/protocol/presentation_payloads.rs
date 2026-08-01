//! Total, accessor-based mappings from raw presentation payloads.
//!
//! Views and feature controllers consume these typed projections instead of
//! interpreting wire JSON themselves (#1220, #1257).

use serde_json::Value;

/// Parse a recovered transcript message with the same lenient per-field
/// semantics as ledger synchronization.
pub fn recovered_message(value: &Value) -> crate::protocol::agent_ledger_payloads::LedgerMessage {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// Parse a `get_subagents` response at the protocol boundary.
pub fn subagents(value: &Value) -> Vec<crate::protocol::client::SubagentInfoEvent> {
    value
        .get("subagents")
        .cloned()
        .and_then(|items| serde_json::from_value(items).ok())
        .unwrap_or_default()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TurnEndPayload {
    pub message_refs: Vec<String>,
    pub content_length: Option<u64>,
    pub usage_total: u64,
    pub context_tokens: Option<u64>,
    pub max_context_tokens: Option<usize>,
}

pub fn parse_turn_end(value: &Value) -> TurnEndPayload {
    TurnEndPayload {
        message_refs: message_refs(value),
        content_length: u64_field(value, "contentLength"),
        usage_total: value
            .pointer("/usage/total")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        context_tokens: u64_field(value, "contextTokens"),
        max_context_tokens: u64_field(value, "maxContextTokens").map(|v| v as usize),
    }
}

pub fn message_refs(value: &Value) -> Vec<String> {
    [value.get("messageRefs"), value.get("message_refs")]
        .into_iter()
        .flatten()
        .find_map(|candidate| {
            let refs = candidate
                .as_array()?
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .or_else(|| item.get("id")?.as_str().map(str::to_owned))
                })
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>();
            (!refs.is_empty()).then_some(refs)
        })
        .unwrap_or_default()
}

pub fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

pub fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

pub fn has_array_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_array).is_some()
}

pub fn spawn_is_read_only(value: &Value) -> bool {
    bool_field(value, "read_only").unwrap_or(false)
        || value
            .get("disable_tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                let has = |name| tools.iter().any(|tool| tool.as_str() == Some(name));
                has("write") && has("edit")
            })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryPageFacts {
    pub before: Option<String>,
    pub has_more_before: bool,
    pub trimmed: bool,
}

pub fn history_page_facts(value: &Value) -> HistoryPageFacts {
    HistoryPageFacts {
        before: string_field(value, "before"),
        has_more_before: bool_field(value, "hasMoreBefore").unwrap_or(false),
        trimmed: bool_field(value, "trimmed").unwrap_or(false),
    }
}

pub fn is_history_page(value: &Value) -> bool {
    has_array_field(value, "messages")
        && (bool_field(value, "hasMoreBefore").is_some() || value.get("before").is_some())
}

pub fn response_identity(value: &Value) -> (Option<String>, Option<String>) {
    (string_field(value, "id"), string_field(value, "role"))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ToolDisplayArgs<'a> {
    pub command: Option<&'a str>,
    pub path: Option<&'a str>,
    pub content: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub task: Option<&'a str>,
    pub action: Option<&'a str>,
    pub step: Option<u64>,
    pub template: Option<&'a str>,
    pub issue_number: Option<u64>,
    pub query: Option<&'a str>,
    pub url: Option<&'a str>,
    pub old_text: Option<&'a str>,
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

pub fn tool_display_args(value: Option<&Value>) -> ToolDisplayArgs<'_> {
    let Some(v) = value else {
        return ToolDisplayArgs::default();
    };
    ToolDisplayArgs {
        command: str_field(v, "command"),
        path: str_field(v, "path").or_else(|| str_field(v, "file_path")),
        content: str_field(v, "content"),
        agent_id: str_field(v, "agent_id"),
        task: str_field(v, "task"),
        action: str_field(v, "action"),
        step: u64_field(v, "step"),
        template: str_field(v, "template"),
        issue_number: u64_field(v, "issueNumber"),
        query: str_field(v, "query"),
        url: str_field(v, "url"),
        old_text: str_field(v, "oldText"),
    }
}

/// Whether a `user`-role history message is a harness-injected sub-agent note
/// rather than something the operator typed (#1338).
///
/// These notes are delivered as real user turns so the model actually answers
/// them (as system messages they were folded into the provider's `system`
/// field and never reached the conversation). For display they are still
/// operator-facing status, not user input: the live path renders them as a
/// one-line chat status via `handle_subagent_notification`, so replayed
/// history must skip them instead of drawing them as user messages.
/// Also matches a ladder-collapsed note: pruning rewrites the content as
/// `[user: "<preview>" (N tokens) — recall("id")]`, and the 60-char preview
/// keeps the opening wrapper tag. Notes are now user-role, so unlike before
/// they are ordinary conversation for the collapse ladder.
pub fn is_subagent_note(content: &str) -> bool {
    let content = content.trim_start();
    content.starts_with(SUBAGENT_NOTE_TAG)
        || content
            .strip_prefix("[user: \"")
            .is_some_and(|preview| preview.starts_with(SUBAGENT_NOTE_TAG))
}

const SUBAGENT_NOTE_TAG: &str = "<subagent_notification";

#[cfg(test)]
mod subagent_note_tests {
    use super::is_subagent_note;

    #[test]
    fn detects_notes_verbatim_and_collapsed() {
        assert!(is_subagent_note(
            "<subagent_notification source=\"spawn_tool\" agent_id=\"poet\" sequence=\"1\">\nidle\n</subagent_notification>"
        ));
        // Ladder-collapsed form (context_pruning::message_collapse_stub).
        assert!(is_subagent_note(
            "[user: \"<subagent_notification source=\"spawn_tool\" agent_id=\"po\" (31 tokens) — recall(\"turn3:msg:user\")]"
        ));
        assert!(!is_subagent_note("write me a poem"));
        assert!(!is_subagent_note(
            "[user: \"write me a poem\" (4 tokens) — recall(\"turn1:msg:user\")]"
        ));
    }
}
