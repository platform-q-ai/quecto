use super::agent_loop_turn_flow::MAX_TOOL_CALL_CUT_OFF_RETRIES;
use super::*;
use crate::domain::inference::services::output_limit::cut_off_tool_call;

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
        // Count of model-malformed requests turned into addressable feedback.
        let mut malformed_retries: u32 = 0;
        let mut cut_off_retries: u32 = 0;
        // #2593: consecutive replies cut off inside a tool call.
        let mut tool_cut_off_retries: u32 = 0;

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
            let after = match response {
                // #2124: nothing visible before the output limit is no answer.
                Ok(response)
                    if is_cut_off_without_answer(&response, self.last_request_max_tokens()) =>
                {
                    let feedback = output_limit_feedback(self.last_request_max_tokens());
                    let recovery = (&mut cut_off_retries, &mut appended_messages, feedback);
                    self.after_cut_off_answer(messages, &response, current_turn, recovery)
                        .await
                }
                // #2593: a call cut off by the output limit is corrected,
                // while the budget lasts; then answered as invalid (#2123).
                Ok(response)
                    if tool_cut_off_retries < MAX_TOOL_CALL_CUT_OFF_RETRIES
                        && cut_off_tool_call(&response).is_some() =>
                {
                    let recovery = (&mut tool_cut_off_retries, &mut appended_messages);
                    self.after_cut_off_tool_call(messages, &response, current_turn, recovery)
                }
                Ok(response) => AfterResponse::Proceed(response),
                Err(error) => {
                    let recovery = (&mut malformed_retries, &mut appended_messages);
                    self.after_provider_failure(messages, (error, output), current_turn, recovery)
                        .await
                }
            };
            let response = match after {
                AfterResponse::Proceed(response) => {
                    cut_off_retries = 0;
                    // A reply that cut no call off restores the budget; one
                    // past the budget keeps it spent.
                    match cut_off_tool_call(&response) {
                        Some(_) => {}
                        None => tool_cut_off_retries = 0,
                    }
                    response
                }
                AfterResponse::Retry => {
                    current_turn += 1;
                    continue;
                }
                AfterResponse::Finish(result) => return result,
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
            let cut_off = cut_off_tool_call(&response).map(|cut| cut.id.to_owned());
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
