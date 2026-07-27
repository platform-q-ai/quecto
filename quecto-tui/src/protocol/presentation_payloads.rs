//! Total, accessor-based mappings from raw presentation payloads.
//!
//! Views and feature controllers consume these typed projections instead of
//! interpreting wire JSON themselves (#1220, #1257).

use serde_json::Value;

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
pub struct ToolDisplayArgs {
    pub command: Option<String>,
    pub path: Option<String>,
    pub content: Option<String>,
    pub agent_id: Option<String>,
    pub task: Option<String>,
    pub action: Option<String>,
    pub step: Option<u64>,
    pub template: Option<String>,
    pub issue_number: Option<u64>,
    pub query: Option<String>,
    pub url: Option<String>,
    pub old_text: Option<String>,
}

pub fn tool_display_args(value: Option<&Value>) -> ToolDisplayArgs {
    let Some(v) = value else {
        return ToolDisplayArgs::default();
    };
    ToolDisplayArgs {
        command: string_field(v, "command"),
        path: string_field(v, "path").or_else(|| string_field(v, "file_path")),
        content: string_field(v, "content"),
        agent_id: string_field(v, "agent_id"),
        task: string_field(v, "task"),
        action: string_field(v, "action"),
        step: u64_field(v, "step"),
        template: string_field(v, "template"),
        issue_number: u64_field(v, "issueNumber"),
        query: string_field(v, "query"),
        url: string_field(v, "url"),
        old_text: string_field(v, "oldText"),
    }
}
