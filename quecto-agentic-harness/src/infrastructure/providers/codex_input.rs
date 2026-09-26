//! The Responses API `input` a conversation becomes, including the
//! encrypted reasoning a turn carries back to its model (#2162).
use super::CodexProvider;
use crate::domain::message::{Message, Role, ThinkingBlock};

impl CodexProvider {
    /// `build_input` replaying no reasoning (tests of the rest of it).
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn build_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_input_for(messages, "")
    }

    /// The reasoning items `msg` carries for `model`, as input items: only
    /// the model that produced one can decrypt it (#2162).
    fn replayed_reasoning(msg: &Message, model: &str) -> Vec<serde_json::Value> {
        msg.thinking_blocks
            .iter()
            .filter_map(|block| match block {
                ThinkingBlock::EncryptedReasoning {
                    model: produced_by,
                    item,
                } if !model.is_empty() && produced_by == model => {
                    match serde_json::from_str::<serde_json::Value>(item) {
                        Ok(value @ serde_json::Value::Object(_)) => Some(value),
                        Ok(_) | Err(_) => {
                            tracing::warn!(
                                "Codex: a stored reasoning item is not an object; not replayed"
                            );
                            None
                        }
                    }
                }
                ThinkingBlock::EncryptedReasoning { .. }
                | ThinkingBlock::Normal { .. }
                | ThinkingBlock::Redacted { .. } => None,
            })
            .collect()
    }

    /// Convert our domain messages into Responses API `input` array.
    ///
    /// Calls [`crate::domain::session::filter_orphan_tool_pairs`] to exclude
    /// mismatched function_call/function_call_output pairs (which would cause
    /// HTTP 400). Logs any orphaned pairs with Codex-specific context.
    pub(super) fn build_input_for(
        messages: &[Message],
        model: &str,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        let (valid_pairs, diag) = crate::domain::session::filter_orphan_tool_pairs(messages);
        let last_non_tool_assistant_idx = messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, m)| matches!(m.role, Role::Assistant) && m.tool_calls.is_empty())
            .map(|(i, _)| i);
        if diag.has_orphans() {
            tracing::warn!(
                orphaned_calls = ?diag.orphaned_calls,
                orphaned_outputs = ?diag.orphaned_results,
                "Codex: orphaned function_call/output pairs removed \
                 (session corrupted mid-turn or by context pruning). \
                 OpenAI and Anthropic have the same pairing constraint."
            );
        }
        let mut instructions: Option<String> = None;
        let mut input = Vec::new();

        for (idx, msg) in messages.iter().enumerate() {
            match msg.role {
                Role::System => match &mut instructions {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(&msg.content);
                    }
                    None => instructions = Some(msg.content.clone()),
                },
                Role::User => {
                    input.push(serde_json::json!({ "role": "user", "content": msg.content }));
                }
                Role::Assistant => {
                    let phase = if Some(idx) == last_non_tool_assistant_idx {
                        "final_answer"
                    } else {
                        "commentary"
                    };
                    // The turn's own items, then (below) its reasoning in
                    // front of them: a reasoning item is only sent with the
                    // item that followed it (#2162).
                    let turn_start = input.len();
                    if !msg.tool_calls.is_empty() {
                        // Emit only the valid (matched) tool calls.
                        let mut emitted = 0usize;
                        for tc in &msg.tool_calls {
                            if valid_pairs.contains(&tc.id) {
                                input.push(serde_json::json!({
                                    "type": "function_call",
                                    "call_id": tc.id,
                                    "name": tc.name,
                                    "arguments": tc.wire_arguments(),
                                }));
                                emitted += 1;
                            }
                        }
                        // If every tool call was orphaned and dropped, fall back to
                        // emitting the assistant text content (if any) so narrative
                        // context is not silently lost.
                        if emitted == 0 && !msg.content.is_empty() {
                            input.push(serde_json::json!({
                                "role": "assistant",
                                "phase": phase,
                                "content": msg.content,
                            }));
                        }
                    } else {
                        input.push(serde_json::json!({
                            "role": "assistant",
                            "phase": phase,
                            "content": msg.content,
                        }));
                    }
                    if input.len() > turn_start {
                        let reasoning = Self::replayed_reasoning(msg, model);
                        input.splice(turn_start..turn_start, reasoning);
                    }
                }
                Role::Tool => {
                    if let Some(ref call_id) = msg.tool_call_id {
                        if valid_pairs.contains(call_id) {
                            input.push(serde_json::json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": msg.content,
                            }));
                        }
                    }
                }
            }
        }

        (instructions, input)
    }
}
