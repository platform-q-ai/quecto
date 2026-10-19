use super::*;
use crate::domain::inference::services::inference_recovery::{
    Correction, InferenceRecovery, MAX_CUT_OFF_RETRIES, MAX_MALFORMED_REQUEST_RETRIES,
    MAX_TOOL_CALL_CUT_OFF_RETRIES, Recovery,
};

impl AgentLoopImpl {
    /// Run the LLM-tool loop.
    pub(super) async fn run_loop(
        &mut self,
        messages: &mut Vec<Message>,
    ) -> Result<AgentResult, DomainError> {
        self.mark_turn_in_flight();
        let mut tool_defs = self.current_tool_definitions();
        let mut iterations: u32 = 0;
        let mut current_turn: u32 = 1;
        // True when the manifest needs a rebuild; starts true for prior spills.
        let mut spills_dirty = true;
        let mut usage_totals = UsageTotals::default();
        // Per-run append ledger (#1072). Entries are full clones taken at
        // append time: a later ladder pass may demote any of them IN PLACE
        // (`collapse_message` mutates the `&mut Message` in the conversation
        // vector), so a shared representation (`Arc<Message>`) or clone-on-emit
        // cannot preserve as-appended content.
        //
        // Cost (#1073 review): one clone per appended message, held until run
        // end. Accepted deliberately: the ledger drops when `AgentResult`
        // is consumed at run end, and emission is independently capped at the
        // shared 8 MiB protocol frame cap (#1062); over-cap aggregates reject
        // at emission. If run-lifetime ledger growth becomes a problem, use
        // clone-on-demote (move the original into the ledger only when a
        // prune pass is about to mutate it), not Arc sharing.
        let mut appended_messages = Vec::new();
        // #1072: durable-prefix dirtiness is latched by `apply_context_pruning`
        // itself (any mutating ladder/collapse outcome) rather than diffing a
        // pre-run id snapshot — in-place stub demotion changes content while
        // every message id stays the same, which a snapshot comparison misses.
        // #1908: the run's corrections, their budgets and their feedback.
        let mut recovery = InferenceRecovery::for_run();

        loop {
            if iterations > 0 {
                self.drain_tool_policy_mutations_at_internal_boundary();
                tool_defs = self.current_tool_definitions();
            }
            let _state = TurnState::PrepareProviderRequest;
            let estimated_context_tokens = self
                .apply_context_pruning(messages, current_turn, spills_dirty)
                .await;
            self.notify(|| AgentProgressEvent::ConversationChanged {
                messages: messages.clone().into(),
            });

            // #2421: the conversation as the active model is sent it.
            let sent = self.conversation_for_model(messages);
            // #2434: whether its reply may be empty (tool results) or must
            // have output (a prompt, a steer, a follow-up, feedback).
            let requirement = ReplyRequirement::for_conversation(sent.messages());
            let request = self.prepare_provider_request_transition(
                sent.messages(),
                &tool_defs,
                estimated_context_tokens,
            );
            let _state = TurnState::AwaitProviderResponse;
            let message_count = sent.messages().len();
            self.audit_provider_request_start(
                current_turn,
                estimated_context_tokens,
                message_count,
            )
            .await;

            let llm_start = std::time::Instant::now();
            // Streaming (UDS mode) forwards token events in real time; REPL/
            // one-shot use the non-streaming path.
            let (response, output) = match self
                .request_provider_response(
                    request,
                    (current_turn, estimated_context_tokens),
                    requirement,
                )
                .await
            {
                Ok(response) => (Ok(response), Output::NotShown),
                Err(failure) => (
                    Err(failure.error),
                    Output::from_emitted(failure.emitted_event),
                ),
            };
            drop(sent);

            let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
            // Every response is audited and counted, including one dropped below.
            let timing = (current_turn, estimated_context_tokens, llm_duration_ms);
            self.account_response(&response, timing, &mut usage_totals)
                .await;
            let (reply, recovered) = match response {
                Ok(response) => {
                    let next = recovery.after_reply(&response, self.last_request_max_tokens());
                    (Some(response), next)
                }
                Err(error) => (None, recovery.after_failure(error, output)),
            };
            let response = match (recovered, reply) {
                (Recovery::Proceed, Some(response)) => response,
                (Recovery::Proceed, None) => unreachable!("a failed request never proceeds"),
                (
                    Recovery::Correct {
                        feedback,
                        boost_output,
                        correction,
                    },
                    _,
                ) => {
                    log_correction(&correction);
                    recovery.correct(messages, &mut appended_messages, feedback, current_turn);
                    // The retry may use the model's cap, not repeat the budget.
                    if boost_output {
                        self.output_boost
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    current_turn += 1;
                    continue;
                }
                (Recovery::Fail(error), _) => {
                    // #2132: no unsettled feedback outlives the failed turn.
                    recovery.withdraw(messages, &mut appended_messages);
                    self.drain_tool_policy_mutations_at_boundary();
                    return self.fail_provider_request(current_turn, error).await;
                }
            };

            match next_state_after_provider_response(&response, requirement) {
                TurnState::FinalizeAssistantResponse => {
                    let end = TurnEnd {
                        iterations,
                        usage: usage_totals,
                        pre_response_context_tokens: estimated_context_tokens,
                        current_turn,
                    };
                    let result = self
                        .finalize_turn_response(messages, response, end, &mut appended_messages)
                        .await;
                    self.drain_tool_policy_mutations_at_boundary();
                    return Ok(result);
                }
                TurnState::EndTurnWithoutReply => {
                    let result = self.empty_reply_result(
                        messages,
                        iterations,
                        usage_totals,
                        appended_messages,
                    );
                    self.drain_tool_policy_mutations_at_boundary();
                    return Ok(result);
                }
                TurnState::ExecuteToolCalls => {}
                _ => unreachable!("provider response classification returned non-response state"),
            }

            // #1072: this turn's appended messages are recorded in the run
            // ledger AT APPEND TIME inside `execute_tool_calls_for_response`
            // — never recovered from a positional slice of `messages`, which
            // pruning can shrink or demote in place.
            let ledger_from = appended_messages.len();
            // #2593: past the correction budget the cut-off call is
            // answered as invalid (#2123), whatever its argument text.
            let cut_off =
                crate::domain::inference::services::output_limit::cut_off_tool_call(&response)
                    .map(|cut| cut.id.to_owned());
            self.execute_tool_calls_for_response(
                messages,
                current_turn,
                response,
                (&mut appended_messages, cut_off.as_deref()),
            )
            .await;
            // Stream this turn's output (assistant message + tool results) over
            // the live progress path so a parent/inspector sees it turn-by-turn,
            // not only at completion (#797). The clone is only paid when a
            // progress callback is registered (via `notify`'s guard), and the
            // Arc<[Message]> payload makes further event clones refcount bumps (#993).
            self.notify(|| AgentProgressEvent::TurnCompleted {
                messages: appended_messages[ledger_from..].into(),
            });
            // Tool calls were executed and spilled — mark dirty for next iteration
            spills_dirty = self.retains_context;
            iterations += 1;
            current_turn += 1;

            if iterations >= self.max_tool_iterations {
                let _state = TurnState::StopAtToolIterationLimit;
                let result = self.tool_iteration_limit_result(
                    messages,
                    iterations,
                    usage_totals,
                    appended_messages,
                );
                self.drain_tool_policy_mutations_at_boundary();
                return Ok(result);
            }
        }
    }
}

/// The operator log of a correction the run makes (#1908): one warning
/// per correction, with its attempt within its budget.
fn log_correction(correction: &Correction) {
    match correction {
        Correction::MalformedRequest { attempt, error } => tracing::warn!(
            target: "provider_retry",
            attempt,
            max = MAX_MALFORMED_REQUEST_RETRIES,
            error = %error,
            "provider rejected request as malformed — re-prompting with addressable feedback"
        ),
        Correction::CutOffAnswer { attempt } => tracing::warn!(
            target: "provider_retry",
            attempt,
            max = MAX_CUT_OFF_RETRIES,
            "reply hit the output limit with nothing visible — asking again, concisely"
        ),
        Correction::CutOffCall { attempt, tool } => tracing::warn!(
            target: "provider_retry",
            attempt,
            max = MAX_TOOL_CALL_CUT_OFF_RETRIES,
            tool = tool.as_str(),
            "reply hit the output limit inside a tool call — asking again with smaller calls"
        ),
    }
}

// #2132: feedback whose correction failed is withdrawn with the turn.
#[cfg(test)]
#[path = "agent_loop_2132_tests.rs"]
mod failed_turn_feedback_tests;
