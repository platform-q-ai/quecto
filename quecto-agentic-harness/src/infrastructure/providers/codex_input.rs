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

    /// The reasoning items `msg` carries for `origin` that led to
    /// `leads_to` (a call, or `None` for the reply's text), as input items:
    /// only the origin that produced one can decrypt it (#2162).
    fn replayed_reasoning(
        msg: &Message,
        origin: &str,
        leads_to: Option<&str>,
    ) -> Vec<serde_json::Value> {
        msg.thinking_blocks
            .iter()
            .filter_map(|block| match block {
                ThinkingBlock::EncryptedReasoning {
                    origin: produced_by,
                    leads_to: led_to,
                    item,
                } if !origin.is_empty()
                    && produced_by == origin
                    && led_to.as_deref() == leads_to =>
                {
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

    /// The `phase` of the assistant message at `idx` (#2397). It depends
    /// only on the message and the one right after it, never on how many
    /// turns follow, so an earlier answer is sent the same way every time and
    /// the prompt cache keeps it: text that ended its turn (nothing after
    /// it, or a user message) is a `final_answer`; text the same turn went
    /// on from (another assistant message next), or text sent with tool
    /// calls, is `commentary`.
    fn phase(messages: &[Message], idx: usize) -> &'static str {
        let msg = &messages[idx];
        debug_assert!(
            matches!(msg.role, Role::Assistant),
            "only an assistant message has a phase"
        );
        let continued = messages
            .get(idx + 1)
            .is_some_and(|next| matches!(next.role, Role::Assistant | Role::Tool));
        match (msg.tool_calls.is_empty(), continued) {
            (true, false) => "final_answer",
            (true, true) | (false, _) => "commentary",
        }
    }

    pub(super) fn build_input_for(
        messages: &[Message],
        origin: &str,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        let (valid_pairs, diag) = crate::domain::session::filter_orphan_tool_pairs(messages);
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
                    let phase = Self::phase(messages, idx);
                    // Each reasoning item goes just before the item it led
                    // to, and only with it (#2162).
                    if !msg.tool_calls.is_empty() {
                        // Emit only the valid (matched) tool calls.
                        let mut emitted = 0usize;
                        for tc in &msg.tool_calls {
                            if valid_pairs.contains(&tc.id) {
                                input.extend(Self::replayed_reasoning(msg, origin, Some(&tc.id)));
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
                            input.extend(Self::replayed_reasoning(msg, origin, None));
                            input.push(serde_json::json!({
                                "role": "assistant",
                                "phase": phase,
                                "content": msg.content,
                            }));
                        }
                    } else {
                        input.extend(Self::replayed_reasoning(msg, origin, None));
                        input.push(serde_json::json!({
                            "role": "assistant",
                            "phase": phase,
                            "content": msg.content,
                        }));
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
