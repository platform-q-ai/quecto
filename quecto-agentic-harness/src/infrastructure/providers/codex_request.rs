use super::{ChatRequest, CodexProvider, ResponsesAuth};

impl CodexProvider {
    /// Build the Responses API tool definitions.
    pub(super) fn build_tools(
        tools: &[crate::domain::tool::ToolDefinition],
    ) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                let params: serde_json::Value =
                    serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                serde_json::json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": params,
                    "strict": false,
                })
            })
            .collect()
    }

    /// Build the full request body.
    ///
    /// `max_output_tokens` is a standard Responses API parameter that the
    /// ChatGPT Codex backend rejects outright with HTTP 400 `{"detail":
    /// "Unsupported parameter: max_output_tokens"}` (#1233 regression), so
    /// it is emitted only on the API-key path.
    pub(super) fn build_request_body(
        request: &ChatRequest<'_>,
        auth: &ResponsesAuth,
        origin: &str,
    ) -> serde_json::Value {
        let (instructions, input) = Self::build_input_for(request.messages, origin);

        let mut body = serde_json::json!({
            "model": request.model,
            "input": input,
            "store": false,
            "stream": true,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": {
                "summary": "auto",
            },
            "include": ["reasoning.encrypted_content"],
        });

        if matches!(auth, ResponsesAuth::ApiKey) {
            body["max_output_tokens"] = serde_json::json!(request.max_tokens);
        }

        // #1066: transmit a configured effort, clamped onto OpenAI's
        // documented scale; when none is configured, omit the field so
        // OpenAI's server default applies — the kernel must not invent a
        // fallback. The same rule applies to `text.verbosity`: only derive it
        // from a configured effort, never hardcode a client-side default.
        if let Some(effort) = request.effort {
            body["reasoning"]["effort"] =
                serde_json::Value::String(Self::reasoning_effort_str(effort).to_string());
            body["text"] = serde_json::json!({ "verbosity": Self::verbosity_str(effort) });
        }

        if let Some(inst) = instructions {
            body["instructions"] = serde_json::Value::String(inst);
        }

        if let Some(session_id) = request.session_id {
            body["prompt_cache_key"] =
                serde_json::Value::String(Self::sanitize_cache_key(session_id));
        }

        let tools = Self::build_tools(request.tools);
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools);
        }

        body
    }

    /// Map an effort level onto OpenAI's documented `reasoning.effort` scale
    /// (`none`/`low`/`medium`/`high`/`xhigh`): levels outside that scale
    /// clamp to the nearest documented value (#1066). `max` is
    /// Anthropic-only, so it clamps to `xhigh` here.
    pub(super) fn reasoning_effort_str(
        effort: crate::domain::inference::value_objects::provider::EffortLevel,
    ) -> &'static str {
        use crate::domain::inference::value_objects::provider::EffortLevel;
        match effort {
            EffortLevel::Max => "xhigh",
            other => other.as_str(),
        }
    }

    /// Map an effort level onto the Responses API `text.verbosity` scale,
    /// which only accepts `low`/`medium`/`high`: levels outside that scale
    /// clamp to the nearest documented value.
    pub(super) fn verbosity_str(
        effort: crate::domain::inference::value_objects::provider::EffortLevel,
    ) -> &'static str {
        use crate::domain::inference::value_objects::provider::EffortLevel;
        match effort {
            EffortLevel::None | EffortLevel::Low => "low",
            EffortLevel::Medium => "medium",
            EffortLevel::High | EffortLevel::XHigh | EffortLevel::Max => "high",
        }
    }
}
