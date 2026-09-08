use super::*;

impl AgentLoopImpl {
    pub(super) async fn request_provider_response(
        &self,
        mut request: ChatRequest<'_>,
        turn: u32,
        estimate: usize,
    ) -> Result<LlmResponse, DomainError> {
        self.flush_request_accounting().await?;
        let prefix = super::super::request_observation::prefix(&request);
        let unchanged = self
            .request_prefix
            .lock()
            .unwrap()
            .replace(prefix.0.clone())
            .map(|previous| previous == prefix.0);
        let trace = Arc::new(crate::domain::request_observation::RequestTrace::default());
        request.trace = Some(trace.clone());
        let mut observation = super::super::request_observation::ObservationGuard::new(
            super::super::request_observation::ObservationSinks {
                log: &self.request_observations,
                outbox: self
                    .request_accounting
                    .as_ref()
                    .map(|_| &self.accounting_outbox),
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
        let result = self.request_provider_response_inner(request).await;
        if let Ok(response) = &result {
            if let Some(usage) = &response.usage {
                self.unreported_usage.lock().unwrap().record(usage);
            }
        }
        let record = observation.finish(&result);
        self.audit(
            turn,
            AuditEvent::RequestObserved {
                observation: record.clone(),
            },
        )
        .await;
        self.flush_request_accounting().await?;
        result
    }

    async fn request_provider_response_inner(
        &self,
        request: ChatRequest<'_>,
    ) -> Result<LlmResponse, DomainError> {
        if let Some(admission) = &self.request_admission {
            admission.check().await?;
        }
        // Transient-error retry is owned by the `RetryingProvider` decorator, so
        // the non-streaming path makes a single call and passes the error
        // through (only enhancing it); re-retrying here would double the budget.
        if !self.streaming {
            if let Some(trace) = &request.trace {
                trace.start();
            }
            return self
                .provider
                .chat(request)
                .await
                .map_err(enhance_provider_error);
        }

        // Streaming initiation *is* retried here: the decorator forwards
        // `chat_stream` without retry, so this loop owns stream re-initiation.
        for attempt in 1..=MAX_PROVIDER_ATTEMPTS {
            if let Some(admission) = &self.request_admission {
                admission.check().await?;
            }
            if let Some(trace) = &request.trace {
                if attempt == 1 {
                    trace.start();
                } else {
                    trace.retry();
                }
            }
            let result = match self.stream_chat_once(request.clone()).await {
                Ok(response) => Ok(response),
                Err(stream_error) if stream_error.emitted_event => {
                    // Replaying after emitted content would corrupt output.
                    return Err(enhance_provider_error(stream_error.error));
                }
                Err(stream_error) => Err(stream_error.error),
            };

            match result {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let class = classify_provider_error(&err);
                    if attempt == MAX_PROVIDER_ATTEMPTS || !class.is_retryable() {
                        return Err(enhance_provider_error(err));
                    }
                    tracing::warn!(
                        target: "provider_retry",
                        attempt,
                        max_attempts = MAX_PROVIDER_ATTEMPTS,
                        error_class = %class,
                        "retrying stream initiation after transient failure"
                    );
                    let Some(delay) = crate::domain::provider_retry::bounded_delay(
                        &err,
                        std::time::Duration::from_millis(
                            PROVIDER_RETRY_BACKOFF_MS * attempt as u64,
                        ),
                        std::time::Duration::from_secs(30),
                    ) else {
                        return Err(enhance_provider_error(err));
                    };
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(DomainError::Provider(
            "provider request failed without an error".to_string(),
        ))
    }

    pub(super) fn prepare_provider_request_transition<'a>(
        &'a self,
        messages: &'a Vec<Message>,
        tool_defs: &'a [crate::domain::tool::ToolDefinition],
        estimated_context_tokens: usize,
    ) -> ChatRequest<'a> {
        let display_context_tokens = self.reconcile_context_gauge(estimated_context_tokens);

        // Emit Thinking before every LLM call so the REPL spinner activates
        // immediately, including during multi-turn tool loops. The gauge value
        // is provider-truth when known, calibrated across estimate-only
        // pruning/collapse changes; pruning itself still uses the estimate.
        self.notify(|| AgentProgressEvent::Thinking {
            context_tokens: display_context_tokens,
            max_context_tokens: self.effective_max_context_tokens(),
            provider: self.provider.name().to_string(),
            model: self.model.clone(),
        });

        self.build_chat_request(messages, tool_defs)
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
                },
            )
            .await;
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
        // The feedback user message was appended by this run, so it belongs in
        // the ledger (#1072 review).
        if let Some(feedback) = messages.last() {
            appended_messages.push(feedback.clone());
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

    pub(super) fn tool_iteration_limit_result(
        &self,
        messages: &[Message],
        iterations: u32,
        usage_totals: UsageTotals,
        appended_messages: Vec<Message>,
    ) -> AgentResult {
        // Emit Done so the spinner is cleared before the limit message.
        self.notify(|| AgentProgressEvent::Done);
        let estimated_context_tokens = context_pruning::estimate_total_tokens(messages);
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
            response: format!(
                "Tool iteration limit ({}) reached. Stopping.",
                self.max_tool_iterations
            ),
            tool_iterations: iterations,
            iteration_limit_reached: true,
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
