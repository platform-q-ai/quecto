use super::{ChatRequest, Message, OpenAiProvider, Role, openai_images};

impl OpenAiProvider {
    /// Build the JSON request body for OpenAI chat completions.
    pub(super) fn build_request_body(&self, request: &ChatRequest<'_>) -> serde_json::Value {
        let messages = request.messages;
        let tools = request.tools;
        let model = request.model;
        let max_tokens = request.max_tokens;
        // #938: strict OpenAI-compatible endpoints (Fireworks qwen3p7-plus)
        // reject any non-leading `system` message. Preserve only the *first*
        // `system` message regardless of its position; demote every later one to
        // `user`, prefixing the content with "[system] " to keep the framing.
        // Tracking `seen_system` (rather than `idx > 0`) avoids an implicit
        // "system is always first" invariant: a non-system message preceding the
        // system prompt no longer causes the real system message to be demoted.
        let mut seen_system = false;
        // #2349 review M1: chat completions rejects a call without its result
        // and a result without its call (400); leave both out, as the
        // Responses route does (`codex_input`).
        let (paired, orphans) =
            crate::domain::sessions::entities::session::filter_orphan_tool_pairs(messages);
        if orphans.has_orphans() {
            tracing::warn!(
                orphaned_calls = ?orphans.orphaned_calls,
                orphaned_results = ?orphans.orphaned_results,
                "chat completions: orphaned tool calls/results left out of the request"
            );
        }
        let is_paired = |id: &str| paired.contains(id);
        let sent: Vec<(&Message, serde_json::Value)> = messages
            .iter()
            .filter(|m| match (&m.role, m.tool_call_id.as_deref()) {
                (Role::Tool, Some(id)) => is_paired(id),
                // An assistant turn left with no text (whitespace is none)
                // and no paired call is an empty turn: left out, as
                // `codex_input` does (#2434). Its reasoning is never replayed.
                (Role::Assistant, _) => {
                    m.tool_calls.iter().any(|tc| is_paired(&tc.id)) || !m.content.trim().is_empty()
                }
                _ => true,
            })
            .map(|m| {
                let is_system = matches!(m.role, Role::System);
                let demote_system = is_system && seen_system;
                if is_system {
                    seen_system = true;
                }
                let role = if demote_system {
                    "user"
                } else {
                    m.role.as_str()
                };
                let content = if demote_system {
                    format!("[system] {}", m.content)
                } else {
                    m.content.clone()
                };
                let mut obj = serde_json::json!({
                    "role": role,
                    "content": content,
                });
                // #2421: a user message's images are content parts.
                if let (Role::User, Some(parts)) = (&m.role, openai_images::user_content(m)) {
                    obj["content"] = parts;
                }
                if m.tool_calls.iter().any(|tc| is_paired(&tc.id)) {
                    let tcs: Vec<serde_json::Value> = m
                        .tool_calls
                        .iter()
                        .filter(|tc| is_paired(&tc.id))
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.wire_arguments(),
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = serde_json::Value::Array(tcs);
                }
                if let Some(ref id) = m.tool_call_id {
                    obj["tool_call_id"] = serde_json::Value::String(id.clone());
                }
                (m, obj)
            })
            .collect();
        // #2421: a tool message carries text only; a batch's images follow it.
        let msgs = openai_images::with_tool_images(sent);

        let mut body = serde_json::json!({
            "model": model,
            "messages": msgs,
            "max_completion_tokens": max_tokens,
        });

        // #1996: a selected effort is transmitted verbatim as the
        // chat-completions `reasoning_effort` parameter (Fireworks, xAI and
        // compatible endpoints document the same name). The application
        // admits a level only from the model's own catalogue vocabulary, so
        // this adapter never decides support and never sends a reasoning
        // option to a model that declared none. Fireworks' alternative
        // `thinking` parameter is mutually exclusive with this one and is
        // never emitted here. OpenAI's own Chat Completions endpoint is the
        // one exception: it rejects `reasoning_effort` with function tools
        // (its reasoning ids are routed to the Responses API, and the OAuth
        // fallback without an account id lands here), so the `openai`
        // adapter never transmits one.
        if let Some(effort) = request.effort.filter(|_| self.transmits_reasoning_effort()) {
            body["reasoning_effort"] = serde_json::Value::String(effort.as_str().to_string());
        }

        if !tools.is_empty() {
            let tool_defs: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    let params: serde_json::Value =
                        serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": params,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tool_defs);
        }

        body
    }
}
