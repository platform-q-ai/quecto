//! Claude Code stealth mode: tool name remapping and identity constants (#437-4).
//!
//! When using Anthropic OAuth tokens (`sk-ant-oat-*`), Pi mimics Claude Code's
//! canonical tool naming and identity. This module provides the mappings.

/// Claude Code 2.x canonical tool names (case-sensitive).
/// Source: https://cchistory.mariozechner.at/data/prompts-2.1.11.md
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

/// Version string used in `user-agent` header for OAuth (Claude Code identity).
pub(super) const CLAUDE_CODE_VERSION: &str = "2.1.75";

/// Convert a tool name to Claude Code canonical casing (case-insensitive match).
/// Returns the original name if no match is found.
pub(super) fn to_claude_code_name(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    for cc_name in CLAUDE_CODE_TOOLS {
        if cc_name.to_ascii_lowercase() == lower {
            return cc_name;
        }
    }
    name
}

/// Convert a tool name received from the API back to the original tool name
/// used in the tool registry (reverse of `to_claude_code_name`).
#[cfg(any(test, feature = "test-support"))]
pub(super) fn from_claude_code_name(
    name: &str,
    tool_defs: &[crate::domain::tool::ToolDefinition],
) -> String {
    let lower = name.to_ascii_lowercase();
    for def in tool_defs {
        if def.name.to_ascii_lowercase() == lower {
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
                    // Missing signature → convert to plain text (Pi does the same).
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

/// Remove unpaired Unicode surrogates that are invalid in JSON strings.
///
/// Valid Rust `String`s cannot contain surrogates, but content pasted from
/// external sources may have WTF-8 artefacts after lossy conversion.
/// This function exists for defence-in-depth parity with pi-mono's
/// `sanitizeSurrogates`.
pub(super) fn sanitize_surrogates(s: &str) -> String {
    s.to_string()
}
