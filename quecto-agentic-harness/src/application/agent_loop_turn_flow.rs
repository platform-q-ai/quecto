use super::*;
use crate::application::agent_loop_stream::ends_turn_empty;
use crate::application::providers::ports::ChatRequest;
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;

/// Retries for a reply cut off at the output limit with nothing visible.
pub(super) const MAX_CUT_OFF_RETRIES: u32 = 1;

/// What the loop does after a provider response or failure.
pub(super) enum AfterResponse {
    /// Continue the turn with this response.
    Proceed(LlmResponse),
    /// Feedback was added: ask the provider again.
    Retry,
    /// The turn ends with this result.
    Finish(Result<AgentResult, DomainError>),
}

impl AgentLoopImpl {
    /// Send a chat request using incremental streaming.
    ///
    /// Emits `AgentProgressEvent::Token` for each text delta so the UDS layer
    /// can forward them as `{"type":"token"}` events.  Falls back gracefully
    /// for providers whose `chat_stream_incremental()` wraps `chat()` (emitting
    /// only a single `Done`). A completed reply with nothing in it is an
    /// empty stream unless it ends the turn as `requirement` allows (#2434).
    pub(super) async fn stream_chat_once(
        &self,
        request: ChatRequest<'_>,
        requirement: ReplyRequirement,
    ) -> Result<LlmResponse, StreamProviderError> {
        let mut emitted_event = false;
        let trace = request.trace.clone();
        let mut rx = self.provider.chat_stream_incremental(request).await;
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::TextDelta(t) => {
                    // Whitespace alone is no output (#2434 review): a blank
                    // reply is an empty stream, asked again as before.
                    emitted_event |= t.chars().any(|c| !c.is_whitespace());
                    self.notify(|| AgentProgressEvent::Token(t));
                }
                StreamEvent::ThinkingDelta(t) => {
                    emitted_event = true;
                    self.notify(|| AgentProgressEvent::ThinkingDelta(t));
                }
                StreamEvent::Done(response) => {
                    let empty = is_empty_streamed_response(&response);
                    return match (empty, ends_turn_empty(&response, requirement)) {
                        // Output, or nothing where nothing ends the turn.
                        (false, _) | (true, true) => Ok(response),
                        (true, false) => {
                            // The empty reply's tokens were spent all the
                            // same: its attempt reports them (#2436 review).
                            // Only here: a reply that ends the turn reports
                            // its usage as an `ok` reply.
                            if let (Some(trace), Some(usage)) = (&trace, response.usage.clone()) {
                                trace.record_unfinished_usage(usage);
                            }
                            Err(StreamProviderError {
                                error: DomainError::Provider(empty_stream_error_message(&response)),
                                emitted_event,
                            })
                        }
                    };
                }
                StreamEvent::Error(e) => {
                    return Err(StreamProviderError {
                        error: DomainError::Provider(e),
                        emitted_event,
                    });
                }
                // Tool call streaming events are handled by the provider's
                // accumulator — they assemble into LlmResponse.tool_calls
                // and are delivered via StreamEvent::Done.
                _ => {
                    emitted_event = true;
                }
            }
        }
        // Channel closed without Done — shouldn't happen but handle gracefully.
        Err(StreamProviderError {
            error: DomainError::Provider("streaming channel closed without completion".to_string()),
            emitted_event,
        })
    }

    pub(super) async fn request_provider_response(
        &self,
        mut request: ChatRequest<'_>,
        (turn, estimate): (u32, usize),
        requirement: ReplyRequirement,
    ) -> Result<LlmResponse, StreamProviderError> {
        self.flush_request_accounting()
            .await
            .map_err(StreamProviderError::before_output)?;
        let prefix = super::super::request_observation::prefix(&request);
        let unchanged = self
            .request_prefix
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(prefix.0.clone())
            .map(|previous| previous == prefix.0);
        let trace = Arc::new(
            crate::domain::inference::events::request_observation::RequestTrace::default(),
        );
        // #2398: the provider compares the input with the session's last.
        trace.attach_input_baseline(self.input_baseline.clone());
        // #2210: every attempt is capped at its output limit's worth of bytes.
        trace.set_output_cap(
            crate::domain::inference::events::request_progress::output_cap_bytes(
                self.model_max_tokens,
                request.max_tokens,
            ),
        );
        // #2436: each attempt is counted and announced as it ends.
        trace.on_attempt_end(self.request_completion_sink(request.model));
        request.trace = Some(trace.clone());
        let mut observation = super::super::request_observation::ObservationGuard::new(
            super::super::request_observation::ObservationSinks {
                log: &self.request_observations,
                outbox: self
                    .request_accounting
                    .as_ref()
                    .map(|_| &self.accounting_outbox),
                in_flight: &self.in_flight_request,
                interrupted: &self.interrupted_requests,
                turn,
            },
            &request,
            self.provider.name(),
            estimate,
            super::super::request_observation::PrefixObservation {
                sha256: prefix.0,
                bytes: prefix.1,
                unchanged,
            },
            trace,
        );
        let outcome = super::super::request_observation::MarkDropping::new(
            observation.trace(),
            self.request_provider_response_inner(request, requirement),
        )
        .await;
        // Whether the failed reply had shown output travels beside the
        // error, which is observed and accounted as before (#2155).
        let (result, emitted_event) = match outcome {
            Ok(response) => (Ok(response), false),
            Err(failure) => (Err(failure.error), failure.emitted_event),
        };
        {
            let mut unreported = self
                .unreported_usage
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Tokens attempts reported before they were cut short were
            // spent all the same (#2249 review): counted first, so the
            // reply's own usage sets the context occupancy last.
            for usage in observation.trace().unfinished_usage() {
                unreported.record(&usage);
            }
            if let Ok(response) = &result {
                if let Some(usage) = &response.usage {
                    unreported.record(usage);
                }
            }
        }
        // #2434: an empty reply that ends the turn stays visible.
        if let Ok(response) = &result {
            if ends_turn_empty(response, requirement) {
                observation.note_ended_empty_after_tools();
            }
        }
        let record = observation.finish(&result);
        self.audit(
            turn,
            AuditEvent::RequestObserved {
                observation: Box::new(record.clone()),
            },
        )
        .await;
        // A flush failure after the reply keeps whether it had shown output.
        self.flush_request_accounting()
            .await
            .map_err(|error| StreamProviderError {
                error,
                emitted_event,
            })?;
        result.map_err(|error| StreamProviderError {
            error,
            emitted_event,
        })
    }

    async fn request_provider_response_inner(
        &self,
        request: ChatRequest<'_>,
        requirement: ReplyRequirement,
    ) -> Result<LlmResponse, StreamProviderError> {
        if let Some(admission) = &self.request_admission {
            admission
                .check(crate::domain::inference::value_objects::provider::RequestAttempt::First)
                .await
                .map_err(StreamProviderError::before_output)?;
        }
        // Transient-error retry is owned by the `RetryingProvider` decorator, so
        // the non-streaming path makes a single call and passes the error
        // through (only enhancing it); re-retrying here would double the budget.
        if !self.streaming {
            if let Some(trace) = &request.trace {
                trace.start();
            }
            return self.provider.chat(request).await.map_err(|error| {
                StreamProviderError::before_output(enhance_provider_error(error))
            });
        }

        // Streaming initiation *is* retried here: the decorator forwards
        // `chat_stream` without retry, so this loop owns stream re-initiation.
        let mut capped =
            crate::domain::inference::services::provider_error::CappedFailures::default();
        for attempt in 1..=MAX_PROVIDER_ATTEMPTS {
            // The logical request was admitted above; only re-initiations
            // re-check, as reattempts (#2339), so streaming never pays a
            // second first-attempt check.
            if attempt > 1 {
                if let Some(admission) = &self.request_admission {
                    admission
                        .check(crate::domain::inference::value_objects::provider::RequestAttempt::Reattempt)
                        .await
                        .map_err(StreamProviderError::before_output)?;
                }
            }
            if let Some(trace) = &request.trace {
                if attempt == 1 {
                    trace.start();
                } else {
                    trace.retry();
                }
            }
            let result = match self.stream_chat_once(request.clone(), requirement).await {
                Ok(response) => Ok(response),
                Err(stream_error) if stream_error.emitted_event => {
                    // Replaying after emitted content would corrupt output;
                    // the turn is told output was shown (#2155).
                    return Err(StreamProviderError {
                        error: enhance_provider_error(stream_error.error),
                        emitted_event: true,
                    });
                }
                Err(stream_error) => Err(stream_error.error),
            };

            match result {
                Ok(response) => return Ok(response),
                Err(err) => {
                    // The attempt ended here, before any back-off (#2436).
                    if let Some(trace) = &request.trace {
                        trace.end_attempt_failed();
                    }
                    let class = classify_provider_error(&err);
                    if attempt == MAX_PROVIDER_ATTEMPTS
                        || !class.is_retryable()
                        || !capped.allows_another(&class)
                    {
                        return Err(StreamProviderError::before_output(enhance_provider_error(
                            err,
                        )));
                    }
                    tracing::warn!(
                        target: "provider_retry",
                        attempt,
                        max_attempts = MAX_PROVIDER_ATTEMPTS,
                        error_class = %class,
                        "retrying stream initiation after transient failure"
                    );
                    let Some(delay) =
                        crate::domain::inference::services::provider_retry::bounded_delay(
                            &err,
                            std::time::Duration::from_millis(
                                PROVIDER_RETRY_BACKOFF_MS * attempt as u64,
                            ),
                            std::time::Duration::from_secs(30),
                        )
                    else {
                        return Err(StreamProviderError::before_output(enhance_provider_error(
                            err,
                        )));
                    };
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(StreamProviderError::before_output(DomainError::Provider(
            "provider request failed without an error".to_string(),
        )))
    }

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

    /// What follows a provider failure: re-prompt a model-malformed request
    /// that showed no output while retries remain, otherwise fail the turn.
    pub(super) async fn after_provider_failure(
        &mut self,
        messages: &mut Vec<Message>,
        (error, output): (DomainError, Output),
        current_turn: u32,
        (malformed_retries, appended_messages): (&mut u32, &mut Vec<Message>),
    ) -> AfterResponse {
        let transition = classify_provider_failure(
            &error,
            output,
            *malformed_retries,
            MAX_MALFORMED_REQUEST_RETRIES,
        );
        let _state = state_for_provider_failure_transition(&transition);
        match transition {
            ProviderFailureTransition::RecoverMalformedRequest => {
                let _state = TurnState::RecoverMalformedResponse;
                self.recover_malformed_response(
                    messages,
                    &error,
                    current_turn,
                    malformed_retries,
                    appended_messages,
                )
                .await;
                AfterResponse::Retry
            }
            ProviderFailureTransition::Terminal(_class) => {
                let _state = TurnState::FailProviderRequest;
                self.drain_tool_policy_mutations_at_boundary();
                AfterResponse::Finish(self.fail_provider_request(current_turn, error).await)
            }
        }
    }

    /// A reply cut off at the output limit with nothing visible (#2124): the
    /// model is asked again, concisely, and the reply is not recorded; once
    /// the retries are spent the turn fails instead of ending empty.
    pub(super) async fn after_cut_off_answer(
        &mut self,
        messages: &mut Vec<Message>,
        response: &LlmResponse,
        current_turn: u32,
        (retries, appended_messages, feedback): (&mut u32, &mut Vec<Message>, String),
    ) -> AfterResponse {
        if *retries < MAX_CUT_OFF_RETRIES {
            *retries += 1;
            tracing::warn!(
                target: "provider_retry",
                attempt = *retries,
                max = MAX_CUT_OFF_RETRIES,
                "reply hit the output limit with nothing visible — asking again, concisely"
            );
            append_feedback(messages, feedback, current_turn);
            record_feedback(messages, appended_messages);
            // The retry may use the model's cap, not repeat the same budget.
            self.output_boost
                .store(true, std::sync::atomic::Ordering::Relaxed);
            AfterResponse::Retry
        } else {
            self.drain_tool_policy_mutations_at_boundary();
            let error = DomainError::Provider(empty_stream_error_message(response));
            AfterResponse::Finish(self.fail_provider_request(current_turn, error).await)
        }
    }

    pub(super) async fn recover_malformed_response(
        &self,
        messages: &mut Vec<Message>,
        error: &DomainError,
        current_turn: u32,
        malformed_retries: &mut u32,
        appended_messages: &mut Vec<Message>,
    ) {
        *malformed_retries += 1;
        tracing::warn!(
            target: "provider_retry",
            attempt = *malformed_retries,
            max = MAX_MALFORMED_REQUEST_RETRIES,
            error = %error,
            "provider rejected request as malformed — re-prompting with addressable feedback"
        );
        append_malformed_feedback(messages, error, current_turn);
        // Feedback the run added belongs in the ledger (#1072 review), once.
        record_feedback(messages, appended_messages);
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

// #2348: the cached share of the input on `llm_turn_end`.
#[cfg(test)]
#[path = "agent_loop_cached_tokens_tests.rs"]
mod cached_tokens_tests;
