//! The Responses API `input` a conversation becomes, including the
//! encrypted reasoning a turn carries back to its model (#2162).
use super::CodexProvider;
use crate::domain::message::{Message, Role, ThinkingBlock};
use crate::infrastructure::providers::provider_images::{DETAIL, data_url, images};

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

    /// Whether an assistant message has text to send (#2434): one with no
    /// text (whitespace is none) and no call sends no item — neither an
    /// empty answer nor reasoning that leads to nothing.
    fn has_text(msg: &Message) -> bool {
        !msg.content.trim().is_empty()
    }

    /// The `phase` of an assistant message (#2397): text-only is a
    /// `final_answer`, because it ended its turn; text sent with tool calls
    /// (the orphan fallback) is `commentary`. It depends on the message alone,
    /// never on what comes after it, so an earlier item is sent the same way
    /// on every later request and the prompt cache keeps it.
    fn phase(msg: &Message) -> &'static str {
        debug_assert!(
            matches!(msg.role, Role::Assistant),
            "only an assistant message has a phase"
        );
        match msg.tool_calls.is_empty() {
            true => "final_answer",
            false => "commentary",
        }
    }

    /// What a user message's `content`, or a tool result's `output`,
    /// becomes (#2421): the text alone, as a string, when the message
    /// carries no image, which is every message's bytes before images
    /// were sent; with images, an array of its text (when it has any) and
    /// each image as an `input_image` data URL at `"detail": "high"`.
    fn sent_content(msg: &Message) -> serde_json::Value {
        let images = images(msg);
        let text = match msg.content.is_empty() {
            true => None,
            false => Some(serde_json::json!({"type": "input_text", "text": msg.content})),
        };
        match images.is_empty() {
            true => serde_json::Value::String(msg.content.clone()),
            false => serde_json::Value::Array(
                text.into_iter()
                    .chain(images.iter().map(|(mime, data)| {
                        serde_json::json!({
                            "type": "input_image",
                            "image_url": data_url(mime, data),
                            "detail": DETAIL,
                        })
                    }))
                    .collect(),
            ),
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

        for msg in messages {
            match msg.role {
                Role::System => match &mut instructions {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(&msg.content);
                    }
                    None => instructions = Some(msg.content.clone()),
                },
                Role::User => {
                    input.push(
                        serde_json::json!({ "role": "user", "content": Self::sent_content(msg) }),
                    );
                }
                Role::Assistant => {
                    let phase = Self::phase(msg);
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
                        if emitted == 0 && Self::has_text(msg) {
                            input.extend(Self::replayed_reasoning(msg, origin, None));
                            input.push(serde_json::json!({
                                "role": "assistant",
                                "phase": phase,
                                "content": msg.content,
                            }));
                        }
                    } else if Self::has_text(msg) {
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
                                "output": Self::sent_content(msg),
                            }));
                        }
                    }
                }
            }
        }

        (instructions, input)
    }
}
