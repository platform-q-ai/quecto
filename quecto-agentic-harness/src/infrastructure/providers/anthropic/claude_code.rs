//! Tool name remapping and identity constants for Anthropic OAuth (#437-4).
//!
//! When using Anthropic OAuth tokens (`sk-ant-oat-*`), the provider uses
//! canonical tool naming required by the `claude-code-20250219` beta.
//! This module provides the mappings.

/// Canonical tool names for OAuth identity (case-sensitive).
const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// Version string used in `user-agent` header for OAuth identity.
pub(super) const CLAUDE_CODE_VERSION: &str = "2.1.75";

/// Convert a tool name to canonical casing for OAuth (case-insensitive match).
/// Returns the original name if no match is found.
///
/// Uses `eq_ignore_ascii_case` to avoid String allocations (#437 perf review).
pub(super) fn to_claude_code_name(name: &str) -> &str {
    for cc_name in CLAUDE_CODE_TOOLS {
        if cc_name.eq_ignore_ascii_case(name) {
            return cc_name;
        }
    }
    name
}

/// Convert a tool name received from the API back to the original tool name
/// used in the tool registry (reverse of `to_claude_code_name`).
///
/// Used in production to reverse-map API-returned canonical tool names
/// (e.g. `"Read"`) back to the registered tool names (e.g. `"read"`).
pub(super) fn from_claude_code_name(
    name: &str,
    tool_defs: &[crate::domain::tool::ToolDefinition],
) -> String {
    for def in tool_defs {
        if def.name.eq_ignore_ascii_case(name) {
            return def.name.to_string();
        }
    }
    name.to_string()
}

/// Build an Anthropic assistant message with thinking blocks (#437-5)
/// and tool name remapping (#437-4).
pub(super) fn build_assistant_message(
    m: &crate::domain::message::Message,
    is_oauth: bool,
) -> serde_json::Value {
    use crate::domain::message::ThinkingBlock;

    // If there are no thinking blocks and no tool calls, use simple format.
    if m.thinking_blocks.is_empty() && m.tool_calls.is_empty() {
        return serde_json::json!({"role": "assistant", "content": sanitize_surrogates(&m.content)});
    }

    let mut content_blocks: Vec<serde_json::Value> = Vec::new();

    // Emit thinking blocks first (they precede text/tool_use in the API).
    for tb in &m.thinking_blocks {
        match tb {
            ThinkingBlock::Normal {
                thinking,
                signature,
            } => {
                if thinking.trim().is_empty() {
                    continue;
                }
                if signature.is_empty() {
                    // Missing signature → convert to plain text.
                    content_blocks.push(serde_json::json!({
                        "type": "text",
                        "text": sanitize_surrogates(thinking),
                    }));
                } else {
                    content_blocks.push(serde_json::json!({
                        "type": "thinking",
                        "thinking": sanitize_surrogates(thinking),
                        "signature": signature,
                    }));
                }
            }
            ThinkingBlock::Redacted { data } => {
                content_blocks.push(serde_json::json!({
                    "type": "redacted_thinking",
                    "data": data,
                }));
            }
        }
    }

    if !m.content.is_empty() {
        content_blocks
            .push(serde_json::json!({"type": "text", "text": sanitize_surrogates(&m.content)}));
    }

    for tc in &m.tool_calls {
        let input: serde_json::Value = serde_json::from_str(&tc.arguments)
            .ok()
            .filter(|v: &serde_json::Value| v.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        let name = if is_oauth {
            to_claude_code_name(&tc.name).to_string()
        } else {
            tc.name.clone()
        };
        content_blocks.push(serde_json::json!({
            "type": "tool_use",
            "id": tc.id,
            "name": name,
            "input": input,
        }));
    }

    if content_blocks.is_empty() {
        return serde_json::json!({"role": "assistant", "content": ""});
    }

    serde_json::json!({"role": "assistant", "content": content_blocks})
}

/// Defence-in-depth surrogate sanitization stub.
///
/// Rust `String` is guaranteed valid UTF-8 and cannot contain unpaired
/// surrogates, so this is a no-op that avoids allocation by returning
/// `Cow::Borrowed`. Kept as a named function so call sites document
/// their intent and the function can be made non-trivial if Quecto
/// ever processes WTF-8 or lossy-converted input.
pub(super) fn sanitize_surrogates(s: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(s)
}
