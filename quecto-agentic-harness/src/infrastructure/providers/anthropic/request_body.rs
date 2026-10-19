use super::{
    AnthropicProvider, ChatRequest, Message, Role, ToolChoice, anthropic_user_msg, claude_code,
    normalize, sanitize_surrogates, to_claude_code_name,
};

impl AnthropicProvider {
    pub(super) fn build_request_body(
        request: &ChatRequest<'_>,
        is_oauth: bool,
    ) -> (Option<String>, serde_json::Value) {
        let (system_prompt, mut api_messages) = Self::build_messages(request.messages, is_oauth);
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": [],
        });

        let adaptive_model = Self::model_uses_adaptive_thinking(request.model);
        Self::apply_thinking_config(&mut body, request, adaptive_model);
        Self::apply_system_prompt(&mut body, &system_prompt, is_oauth);
        Self::apply_cache_control_to_last_user_message(&mut api_messages);

        body["messages"] = serde_json::Value::Array(api_messages);

        if !request.tools.is_empty() {
            body["tools"] =
                serde_json::Value::Array(Self::build_tool_defs(request.tools, is_oauth));
        }

        // tool_choice
        if let Some(ref tc) = request.tool_choice {
            body["tool_choice"] = match tc {
                ToolChoice::Auto => serde_json::json!({"type": "auto"}),
                ToolChoice::Any => serde_json::json!({"type": "any"}),
                ToolChoice::Specific(name) => {
                    let tool_name = if is_oauth {
                        to_claude_code_name(name).to_string()
                    } else {
                        name.clone()
                    };
                    serde_json::json!({"type": "tool", "name": tool_name})
                }
            };
        }

        // metadata
        if let Some(ref meta) = request.metadata {
            if let Some(ref user_id) = meta.user_id {
                body["metadata"] = serde_json::json!({"user_id": user_id});
            }
        }

        (system_prompt, body)
    }

    /// The conversation as the Messages API takes it. Which images a model
    /// is sent is the application's decision (#2421): every image here is
    /// sent.
    pub(super) fn build_messages(
        messages: &[Message],
        is_oauth: bool,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        let normalized = Self::normalize_messages(messages);
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<serde_json::Value> = Vec::new();
        let mut i = 0;

        while i < normalized.len() {
            let m: &Message = &normalized[i];
            match m.role {
                Role::System => {
                    // Concatenate rather than overwrite (#1338). Anthropic has
                    // no in-conversation system role, so every system message
                    // folds into the single top-level `system` field; assigning
                    // meant the LAST one silently discarded the real system
                    // prompt — e.g. the pinned spill manifest
                    // (`context_pruning::update_spill_manifest`) left the agent
                    // running with its entire prompt replaced by one line of
                    // recall guidance. Codex does the same join
                    // (`codex/mod.rs`); OpenAI demotes later system messages to
                    // `user` in place.
                    match &mut system_prompt {
                        Some(existing) => {
                            existing.push_str("\n\n");
                            existing.push_str(&m.content);
                        }
                        None => system_prompt = Some(m.content.clone()),
                    }
                    i += 1;
                }
                Role::User => {
                    if let Some(content) = anthropic_user_msg::build_user_content(m) {
                        api_messages.push(serde_json::json!({"role": "user", "content": content}));
                    }
                    i += 1;
                }
                Role::Assistant => {
                    api_messages.extend(Self::build_assistant_message(m, is_oauth));
                    i += 1;
                }
                Role::Tool => {
                    let mut tool_results: Vec<serde_json::Value> = Vec::new();
                    while i < normalized.len() && normalized[i].role == Role::Tool {
                        tool_results.push(Self::build_tool_result_block(&normalized[i]));
                        i += 1;
                    }
                    api_messages.push(serde_json::json!({
                        "role": "user",
                        "content": tool_results,
                    }));
                }
            }
        }

        normalize::inject_orphaned_tool_results(&mut api_messages);
        (system_prompt, api_messages)
    }

    pub(super) fn build_tool_result_block(m: &Message) -> serde_json::Value {
        let content_value = if m.image_blocks.is_empty() {
            serde_json::Value::String(sanitize_surrogates(&m.content).into_owned())
        } else {
            // An image-only result (an extension may send no text, #2423)
            // has no text block: the API refuses an empty one.
            let mut result_content: Vec<serde_json::Value> = match m.content.is_empty() {
                true => Vec::new(),
                false => vec![
                    serde_json::json!({"type": "text", "text": sanitize_surrogates(&m.content)}),
                ],
            };
            for img in &m.image_blocks {
                result_content.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": img.mime_type(),
                        "data": img.data(),
                    }
                }));
            }
            serde_json::Value::Array(result_content)
        };
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
            "content": content_value,
            "is_error": m.is_error,
        })
    }

    pub(super) fn build_assistant_message(
        m: &Message,
        is_oauth: bool,
    ) -> Option<serde_json::Value> {
        claude_code::build_assistant_message(m, is_oauth)
    }

    #[cfg(test)]
    pub(super) fn build_tool_result_message(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }

    pub(super) fn build_tool_defs(
        tools: &[crate::domain::tool_policy::value_objects::tool::ToolDefinition],
        is_oauth: bool,
    ) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                let input_schema: serde_json::Value =
                    serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                let name = if is_oauth {
                    to_claude_code_name(&t.name).to_string()
                } else {
                    t.name.to_string()
                };
                serde_json::json!({
                    "name": name,
                    "description": t.description,
                    "input_schema": input_schema,
                })
            })
            .collect()
    }
}
