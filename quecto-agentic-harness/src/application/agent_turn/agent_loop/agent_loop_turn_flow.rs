use super::*;
use crate::application::providers::ports::ChatRequest;
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;
use crate::domain::inference::services::provider_error::{
    classify_provider_error, provider_http_status,
};

impl AgentLoopImpl {
    pub(super) fn build_chat_request<'a>(
        &'a self,
        messages: &'a [Message],
        tool_defs: &'a [crate::domain::tool_policy::value_objects::tool::ToolDefinition],
    ) -> ChatRequest<'a> {
        // Pass session_key as session_id so providers that support prompt
        // caching (e.g. Codex prompt_cache_key) can use it.
        let session_id = if self.session_key.is_empty() {
            None
        } else {
            Some(self.session_key.as_str())
        };
        ChatRequest {
            trace: None,
            admission: self.request_admission.clone(),
            messages,
            tools: tool_defs,
            model: &self.model,
            max_tokens: self.effective_max_tokens(),
            temperature: self.temperature,
            session_id,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: self.effort,
        }
    }

    pub(super) fn prepare_provider_request_transition<'a>(
        &'a self,
        messages: &'a [Message],
        tool_defs: &'a [crate::domain::tool_policy::value_objects::tool::ToolDefinition],
        estimated_context_tokens: usize,
    ) -> ChatRequest<'a> {
        let display_context_tokens = self.reconcile_context_gauge(estimated_context_tokens);

        // Emit Thinking before every LLM call so the REPL spinner activates
        // immediately, including during multi-turn tool loops. The gauge value
        // is provider-truth when known, calibrated across estimate-only
        // pruning/collapse changes; pruning uses the estimate at the
        // provider-observed scale (#2212).
        self.notify(|| AgentProgressEvent::Thinking {
            context_tokens: display_context_tokens,
            max_context_tokens: self.effective_max_context_tokens(),
            provider: self.provider.name().to_string(),
            model: self.model.clone(),
        });

        let mut request = self.build_chat_request(messages, tool_defs);
        request.max_tokens = self.request_max_tokens(estimated_context_tokens);
        request
    }

    pub(super) async fn audit_provider_request_start(
        &self,
        current_turn: u32,
        estimated_context_tokens: usize,
        message_count: usize,
    ) {
        if self.audit_log.is_some() {
            self.audit(
                current_turn,
                AuditEvent::LlmTurnStart {
                    input_tokens_estimate: estimated_context_tokens,
                    message_count,
                },
            )
            .await;
        }
    }

    pub(super) async fn audit_provider_response_end(
        &self,
        current_turn: u32,
        response: &LlmResponse,
        estimated_context_tokens: usize,
        duration_ms: u64,
    ) {
        if self.audit_log.is_some() {
            let (input_toks, output_toks) = response
                .usage
                .as_ref()
                .map(|u| (u.context_input_tokens() as _, u.completion_tokens as _))
                .unwrap_or((estimated_context_tokens, 0));
            let stop = response
                .stop_reason
                .as_ref()
                .map_or_else(|| "unknown".to_string(), |s| s.to_string());
            self.audit(
                current_turn,
                AuditEvent::LlmTurnEnd {
                    usage_source: Some(
                        if response.usage.is_some() {
                            "provider_usage_object"
                        } else {
                            "context_estimate"
                        }
                        .into(),
                    ),
                    input_tokens: input_toks,
                    output_tokens: output_toks,
                    stop_reason: stop,
                    duration_ms,
                    // #2348: the share the provider's prompt cache served.
                    cached_input_tokens: response
                        .usage
                        .as_ref()
                        .and_then(|u| u.cache_read_tokens)
                        .map(|n| n as usize),
                    cache_write_tokens: response
                        .usage
                        .as_ref()
                        .and_then(|u| u.cache_write_tokens)
                        .map(|n| n as usize),
                },
            )
            .await;
        }
    }

    /// Audits a provider response and counts its usage, whether the loop then
    /// proceeds with it or drops it (#2124): its tokens were spent either way.
    pub(super) async fn account_response(
        &self,
        response: &Result<LlmResponse, DomainError>,
        (current_turn, context_tokens, duration_ms): (u32, usize, u64),
        usage_totals: &mut UsageTotals,
    ) {
        if let Ok(response) = response {
            self.audit_provider_response_end(current_turn, response, context_tokens, duration_ms)
                .await;
            if let Some(ref usage) = response.usage {
                usage_totals.record(usage);
                // #2212: every reported prompt size calibrates the estimate
                // the next pruning pass decides with, not only the last one
                // of a run; `context_tokens` is the estimate that was sent.
                let reported = usage.context_input_tokens() as usize;
                if reported > 0 {
                    self.observe_provider_context_gauge(reported, context_tokens);
                }
            }
        }
    }

    pub(super) async fn fail_provider_request(
        &self,
        current_turn: u32,
        error: DomainError,
    ) -> Result<AgentResult, DomainError> {
        // Audit: persist the FULL provider error body (redacted) once per
        // terminal failure, never per retry, so it survives TUI line-truncation
        // (#937). `provider` is the harness adapter name (e.g. `openai`), not
        // the upstream endpoint (#939 review).
        let ev = provider_failure_audit_event(self.provider.name(), &error);
        self.audit(current_turn, ev).await;
        self.notify(|| AgentProgressEvent::Done);
        Err(error)
    }

    pub(super) async fn finalize_turn_response(
        &self,
        messages: &mut Vec<Message>,
        response: LlmResponse,
        end: TurnEnd,
        appended_messages: &mut Vec<Message>,
    ) -> AgentResult {
        // Emit Done before finalising so the REPL can clear the spinner line
        // before the final response is printed to stdout.
        self.notify(|| AgentProgressEvent::Done);
        let mut result = self.finalize_text_response(messages, response, end).await;
        self.notify(|| AgentProgressEvent::ConversationChanged {
            messages: messages.clone().into(),
        });
        if let Some(final_message) = messages.last() {
            appended_messages.push(final_message.clone());
        }
        result.appended_messages = std::mem::take(appended_messages);
        result
    }

    pub(super) async fn finalize_text_response(
        &self,
        messages: &mut Vec<Message>,
        response: LlmResponse,
        end: TurnEnd,
    ) -> AgentResult {
        let text = response.content.unwrap_or_default();
        let mut assistant_message = Message::assistant(text.clone(), vec![]);
        assistant_message.thinking_blocks = response.thinking_blocks;
        // Stamp + spill at creation: the loop returns right after this, so no
        // later pruning pass could file the final reply (#1046).
        assistant_message.turn = Some(end.current_turn);
        self.spill_conversation_message(&mut assistant_message)
            .await;
        let estimate_context_tokens = end
            .pre_response_context_tokens
            .saturating_add(context_pruning::estimate_message_tokens(&assistant_message));
        messages.push(assistant_message);
        let usage = end.usage;
        let context_tokens = if usage.context_input_tokens > 0 {
            usage.context_input_tokens as usize
        } else {
            estimate_context_tokens
        };
        // #2212: `account_response` paired each report with its own estimate;
        // the run's last usage may be an earlier call's, so never pair it here.
        if usage.context_input_tokens == 0 {
            self.observe_estimated_context_gauge(estimate_context_tokens);
        }
        AgentResult {
            response: text,
            tool_iterations: end.iterations,
            iteration_limit_reached: false,
            input_tokens: usage.context_input_tokens,
            context_tokens,
            output_tokens: usage.output_tokens,
            billed_input_tokens: usage.billed_input_tokens,
            billed_output_tokens: usage.billed_output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            cost_micro_usd: usage.cost_micro_usd,
            appended_messages: Vec::new(),
        }
    }

    pub(super) fn tool_iteration_limit_result(
        &self,
        messages: &[Message],
        iterations: u32,
        usage_totals: UsageTotals,
        appended_messages: Vec<Message>,
    ) -> AgentResult {
        let response = format!(
            "Tool iteration limit ({}) reached. Stopping.",
            self.max_tool_iterations
        );
        let mut result =
            self.result_without_reply(messages, (iterations, usage_totals), appended_messages);
        result.response = response;
        result.iteration_limit_reached = true;
        result
    }

    /// The model answered tool results with nothing (#2434): the turn ends
    /// as a final answer with no text, and nothing empty is recorded as its
    /// reply (an empty assistant message is one some providers refuse to be
    /// sent again), so the conversation ends on the tool results, as at the
    /// tool iteration limit.
    pub(super) fn empty_reply_result(
        &self,
        messages: &[Message],
        iterations: u32,
        usage_totals: UsageTotals,
        appended_messages: Vec<Message>,
    ) -> AgentResult {
        debug_assert_eq!(
            ReplyRequirement::for_conversation(messages),
            ReplyRequirement::MayBeEmpty,
            "only a reply to tool results ends a turn empty"
        );
        tracing::info!(
            target: "agent_loop",
            iterations,
            "ended_empty_after_tools: the model answered tool results with nothing; the turn ends"
        );
        self.result_without_reply(messages, (iterations, usage_totals), appended_messages)
    }

    /// A turn that ends with no reply recorded: no text, its usage and the
    /// messages it appended.
    fn result_without_reply(
        &self,
        messages: &[Message],
        (iterations, usage_totals): (u32, UsageTotals),
        appended_messages: Vec<Message>,
    ) -> AgentResult {
        // Emit Done so the spinner is cleared before the turn ends.
        self.notify(|| AgentProgressEvent::Done);
        // With the tool definitions, as every other estimate path (#2212).
        let estimated_context_tokens = context_pruning::estimate_total_tokens(messages)
            .saturating_add(self.tool_definition_tokens());
        // `context_input_tokens` is the latest call's provider-reported
        // occupancy (assigned, not accumulated, by UsageTotals::record), so
        // report it directly; estimate-only providers observe the estimate.
        let context_tokens = if usage_totals.context_input_tokens > 0 {
            usage_totals.context_input_tokens as usize
        } else {
            self.observe_estimated_context_gauge(estimated_context_tokens);
            estimated_context_tokens
        };
        AgentResult {
            response: String::new(),
            tool_iterations: iterations,
            iteration_limit_reached: false,
            input_tokens: usage_totals.context_input_tokens,
            context_tokens,
            output_tokens: usage_totals.output_tokens,
            billed_input_tokens: usage_totals.billed_input_tokens,
            billed_output_tokens: usage_totals.billed_output_tokens,
            cache_read_tokens: usage_totals.cache_read_tokens,
            cache_write_tokens: usage_totals.cache_write_tokens,
            cost_micro_usd: usage_totals.cost_micro_usd,
            appended_messages,
        }
    }
}

/// Build the audit event that persists a terminal provider failure (#937).
///
/// Returns the rich [`AuditEvent::ProviderError`] carrying the *full*,
/// untruncated error body (with provider name, classified class, and recovered
/// HTTP status). The body is redacted by [`AuditEvent::provider_error`] before
/// it is persisted, so a failed turn is retrievable from `~/.quecto/audit`
/// instead of evaporating with the truncated TUI line.
///
/// Only this single event is emitted. An earlier version also emitted a generic
/// [`AuditEvent::Error`] carrying the same `err.to_string()` "for back-compat",
/// but that copy was *not* redacted, so any secret echoed in the body landed on
/// disk in cleartext on the Error line right next to the scrubbed ProviderError
/// (PR #939 security review). No consumer required the duplicate, so it is
/// dropped: the redacted ProviderError is the sole, complete record. Emitted
/// once per terminal failure (the caller invokes this only after
/// retries/malformed re-prompts are exhausted), never per retry.
fn provider_failure_audit_event(provider: &str, err: &DomainError) -> AuditEvent {
    AuditEvent::provider_error(
        provider,
        &classify_provider_error(err),
        provider_http_status(err),
        &err.to_string(),
    )
}

// #2348: the cached share of the input on `llm_turn_end`.
#[cfg(test)]
#[path = "agent_loop_cached_tokens_tests.rs"]
mod cached_tokens_tests;
// #2593: a reply cut off at the output limit inside a tool call.
#[cfg(test)]
#[path = "agent_loop_2593_tests.rs"]
mod tool_call_cut_off_tests;
