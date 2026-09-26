//! Tool-call execution for the agent loop: appends the assistant tool-call
//! message and each tool result to the conversation, recording every append
//! in the per-run ledger (#1072).

use super::*;

/// An overlapping batch in flight: its calls and each call's result so far.
/// Dropped at the batch's end or by a cancellation, it gives the response
/// its calls back and appends every finished result in call order, so a
/// cancelled turn keeps what completed (#2175 review).
struct OverlappingBatch<'a> {
    agent: &'a AgentLoopImpl,
    messages: &'a mut Vec<Message>,
    run_ledger: &'a mut Vec<Message>,
    assistant_index: usize,
    current_turn: u32,
    calls: Vec<ToolCall>,
    slots: Vec<Slot>,
}

/// Where one call of a batch stands.
enum Slot {
    Running,
    /// Ran; not yet audited or spilled.
    Ran(CallOutcome, std::time::Duration),
    Prepared(PreparedResult),
}

type CallOutcome = (
    String,
    Vec<crate::domain::tool::ImageBlock>,
    Option<String>,
    bool,
);

impl Drop for OverlappingBatch<'_> {
    fn drop(&mut self) {
        let calls = std::mem::take(&mut self.calls);
        for (tc, slot) in calls.iter().zip(std::mem::take(&mut self.slots)) {
            let result = match slot {
                Slot::Running => continue,
                Slot::Prepared(result) => result,
                // Cancelled before it was prepared: its message is built
                // here, without the audit record or spill a finished batch
                // gives it (neither can be awaited in a drop).
                Slot::Ran((content, image_blocks, delivery_metadata, is_error), _) => {
                    let mut message = self.agent.build_tool_message(ToolMessageArgs {
                        tc,
                        content,
                        image_blocks,
                        is_error,
                    });
                    message.turn = Some(self.current_turn);
                    PreparedResult {
                        message,
                        tool_name: tc.name.clone(),
                        tool_arguments: tc.wire_arguments().into_owned(),
                        delivery_metadata,
                        is_error,
                    }
                }
            };
            self.agent
                .append_tool_result(self.messages, self.run_ledger, result);
        }
        self.messages[self.assistant_index].tool_calls = calls;
    }
}

/// A call's result, built and ready to append.
struct PreparedResult {
    message: Message,
    tool_name: String,
    tool_arguments: String,
    delivery_metadata: Option<String>,
    is_error: bool,
}

/// One call's outcome, ready to be recorded.
struct FinishedCall<'a> {
    idx: usize,
    tc: &'a ToolCall,
    outcome: CallOutcome,
    elapsed: std::time::Duration,
}

impl AgentLoopImpl {
    /// Execute the tool calls in `response`, appending the assistant message
    /// and each tool result to `messages` AND recording a clone of each in
    /// `run_ledger` at the moment it is appended (#1072). The ledger is the
    /// only authority on "messages appended this run" — later pruning passes
    /// may demote or drop the conversation copies in place.
    pub(super) async fn execute_tool_calls_for_response(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        response: LlmResponse,
        run_ledger: &mut Vec<Message>,
    ) {
        let mut assistant =
            Message::assistant(response.content.unwrap_or_default(), response.tool_calls);
        assistant.stop_reason = response.stop_reason;
        assistant.thinking_blocks = response.thinking_blocks;
        // Stamp the turn: the creation-time spill files this as
        // turn{N}:msg:assistant (#1046).
        assistant.turn = Some(current_turn);
        self.spill_conversation_message(&mut assistant).await;
        run_ledger.push(assistant.clone());
        messages.push(assistant);
        let assistant_index = messages.len() - 1;
        let call_count = messages[assistant_index].tool_calls.len();

        // #2169: when every call of the response may overlap (the registry
        // says so: tools that change nothing), they run at once; otherwise
        // one at a time, as before. Either way each call is recorded before
        // it runs and its result is appended in call order. The calls are
        // borrowed, never cloned: their arguments may be large (#993).
        let calls = &messages[assistant_index].tool_calls;
        let overlap = call_count > 1
            && calls.iter().enumerate().all(|(i, tc)| {
                let arguments = tc.wire_arguments();
                // Two identical calls would race on the tools' own caches
                // (read's), so they run one at a time (#2175 review).
                let repeated = calls[..i]
                    .iter()
                    .any(|earlier| earlier.name == tc.name && earlier.arguments == tc.arguments);
                !repeated && self.tool_executor().overlaps_safely(&tc.name, &arguments)
            });
        if overlap {
            // The calls move into the batch (never cloned, #993). While they
            // run, nothing else is awaited, so each is timed alone; then each
            // result is prepared (audited, spilled) in call order. Dropped at
            // its end or by a cancellation, the batch gives the calls back
            // and appends every finished result in call order (#2175 review).
            let calls = std::mem::take(&mut messages[assistant_index].tool_calls);
            let mut batch = OverlappingBatch {
                agent: self,
                messages,
                run_ledger,
                assistant_index,
                current_turn,
                slots: (0..calls.len()).map(|_| Slot::Running).collect(),
                calls,
            };
            let calls = &batch.calls;
            for tc in calls {
                self.audit_tool_call(current_turn, tc).await;
            }
            let mut running: futures::stream::FuturesUnordered<_> = calls
                .iter()
                .enumerate()
                .map(|(idx, tc)| async move {
                    let started = std::time::Instant::now();
                    let outcome = self.execute_single_tool_call(tc).await;
                    (idx, outcome, started.elapsed())
                })
                .collect();
            while let Some((idx, outcome, elapsed)) = futures::StreamExt::next(&mut running).await {
                debug_assert!(
                    matches!(batch.slots[idx], Slot::Running),
                    "each call finishes once"
                );
                batch.slots[idx] = Slot::Ran(outcome, elapsed);
            }
            drop(running);
            for idx in 0..calls.len() {
                let Slot::Ran(outcome, elapsed) =
                    std::mem::replace(&mut batch.slots[idx], Slot::Running)
                else {
                    unreachable!("every call ran before any is prepared");
                };
                let finished = FinishedCall {
                    idx,
                    tc: &calls[idx],
                    outcome,
                    elapsed,
                };
                batch.slots[idx] =
                    Slot::Prepared(self.prepare_tool_result(current_turn, finished).await);
            }
        } else {
            // One at a time, each result appended before the next call runs.
            for idx in 0..call_count {
                let result = {
                    let tc = &messages[assistant_index].tool_calls[idx];
                    self.audit_tool_call(current_turn, tc).await;
                    let started = std::time::Instant::now();
                    let outcome = self.execute_single_tool_call(tc).await;
                    let elapsed = started.elapsed();
                    let finished = FinishedCall {
                        idx,
                        tc,
                        outcome,
                        elapsed,
                    };
                    self.prepare_tool_result(current_turn, finished).await
                };
                self.append_tool_result(messages, run_ledger, result);
            }
        }
    }

    /// Audit a call before it runs: what it runs with, and what the model
    /// sent when that differs (#2123).
    async fn audit_tool_call(&self, current_turn: u32, tc: &ToolCall) {
        if self.audit_log.is_some() {
            let delivered = tc.wire_arguments().into_owned();
            self.audit(
                current_turn,
                AuditEvent::ToolCall {
                    tool: tc.name.clone(),
                    call_id: tc.id.clone(),
                    // What the tool ran with (#2123), as `argument_bytes`
                    // measures it (#2150).
                    raw_arguments: (tc.arguments != delivered).then(|| tc.arguments.clone()),
                    arguments: delivered,
                },
            )
            .await;
        }
    }

    /// Prepare a finished call's result: audit it and build (and spill) its
    /// message, ready to append.
    async fn prepare_tool_result(
        &self,
        current_turn: u32,
        finished: FinishedCall<'_>,
    ) -> PreparedResult {
        let FinishedCall {
            idx,
            tc,
            outcome: (content, image_blocks, delivery_metadata, is_error),
            elapsed: tool_elapsed,
        } = finished;
        // The text the tool actually ran with (#2123).
        let delivered_tool_arguments = tc.wire_arguments().into_owned();
        // Audit: ToolResult (guarded — avoid estimate_tokens/preview when disabled)
        if self.audit_log.is_some() {
            let content_tokens = context_pruning::estimate_tokens(&content);
            let preview = crate::domain::audit::content_preview(&content, 200);
            self.audit(
                current_turn,
                AuditEvent::ToolResult {
                    call_id: tc.id.clone(),
                    tool: tc.name.clone(),
                    is_error,
                    content_tokens,
                    content_preview: preview,
                    duration_ms: u64::try_from(tool_elapsed.as_millis()).unwrap_or(u64::MAX),
                    argument_bytes: delivered_tool_arguments.len(),
                    content_bytes: content.len(),
                },
            )
            .await;
        }

        let spill_id = format!("turn{}:{}:{}", current_turn, tc.name, idx);
        let mut tool_msg = self.build_tool_message(ToolMessageArgs {
            tc,
            content,
            image_blocks,
            is_error,
        });
        tool_msg.turn = Some(current_turn);
        // Stamps `spill_id` on the message only if the append succeeds.
        self.spill_tool_message(&mut tool_msg, spill_id).await;
        PreparedResult {
            message: tool_msg,
            tool_name: tc.name.clone(),
            tool_arguments: delivered_tool_arguments,
            delivery_metadata,
            is_error,
        }
    }

    /// Append a prepared result to the conversation and the ledger, then
    /// acknowledge its delivery.
    fn append_tool_result(
        &self,
        messages: &mut Vec<Message>,
        run_ledger: &mut Vec<Message>,
        result: PreparedResult,
    ) {
        run_ledger.push(result.message.clone());
        messages.push(result.message);
        if !result.is_error {
            let delivered = crate::domain::tool::ToolResult {
                content: messages
                    .last()
                    .map(|m| m.content.clone())
                    .unwrap_or_default(),
                image_blocks: messages
                    .last()
                    .map(|m| m.image_blocks.clone())
                    .unwrap_or_default(),
                delivery_metadata: result.delivery_metadata,
                is_error: result.is_error,
            };
            self.tool_executor().result_delivered(
                &result.tool_name,
                &result.tool_arguments,
                &delivered,
            );
        }
    }

    async fn execute_single_tool_call(
        &self,
        tc: &ToolCall,
    ) -> (
        String,
        Vec<crate::domain::tool::ImageBlock>,
        Option<String>,
        bool,
    ) {
        // Emit ToolStarted before executing so interactive clients can show the tool name
        // immediately, even if the tool itself takes a long time.
        // Clones inside the closure are only evaluated when a callback is
        // registered (zero-cost on headless paths via notify's guard).
        self.notify(|| AgentProgressEvent::ToolStarted {
            tool_call_id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
        });

        let start = std::time::Instant::now();
        let disabled_by_runtime_policy = self
            .tool_policy_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .blocks_execution(tc.name.as_str(), self.tool_profile_context);
        let disabled = || crate::domain::tool::ToolResult {
            content: format!("tool '{}' is disabled by runtime policy", tc.name),
            image_blocks: vec![],
            delivery_metadata: None,
            is_error: true,
        };
        let tool_result = match tc.argument_shape() {
            // A disabled tool is reported as disabled, never as "resend it".
            crate::domain::message::ToolArguments::Invalid(_) if disabled_by_runtime_policy => {
                Ok(disabled())
            }
            // #2123: never run a call whose arguments are not a JSON object;
            // tell the model what it sent so it can resend a whole call.
            crate::domain::message::ToolArguments::Invalid(raw) => {
                Ok(invalid_arguments(&tc.name, raw))
            }
            crate::domain::message::ToolArguments::Object(_)
            | crate::domain::message::ToolArguments::Empty => {
                let admission = match &self.tool_admission {
                    Some(policy) => policy.check(&tc.name, &tc.wire_arguments()).await,
                    None => Ok(()),
                };
                if let Err(error) = admission {
                    Err(error)
                } else if disabled_by_runtime_policy {
                    Ok(disabled())
                } else {
                    self.tool_executor()
                        .execute(&tc.name, &tc.wire_arguments())
                        .await
                }
            }
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        let (content, image_blocks, delivery_metadata, is_err) = match tool_result {
            Ok(tr) => (
                tr.content,
                tr.image_blocks,
                tr.delivery_metadata,
                tr.is_error,
            ),
            Err(e) => (format!("Error: {}", e), vec![], None, true),
        };

        // Emit ToolFinished so the REPL can replace the spinner line.
        // Build the bounded preview inside notify so headless runs allocate none.
        self.notify(|| AgentProgressEvent::ToolFinished {
            tool_call_id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
            result_content: agent_loop_preview::tool_result_preview(&content),
            duration_ms,
            is_error: is_err,
        });

        tracing::info!(
            target: "tool_exec",
            tool_name = tc.name.as_str(),
            duration_ms,
            is_error = is_err,
            "tool executed"
        );
        (content, image_blocks, delivery_metadata, is_err)
    }
}

/// The error returned for a call whose arguments are not a JSON object
/// (#2123). It shows both ends of what was received: for a call cut off at
/// the output limit, the end is where it broke.
fn invalid_arguments(tool: &str, raw: &str) -> crate::domain::tool::ToolResult {
    const HEAD: usize = 200;
    const TAIL: usize = 300;
    let count = raw.chars().count();
    let received = if count <= HEAD + TAIL {
        raw.to_string()
    } else {
        let head: String = raw.chars().take(HEAD).collect();
        let tail: String = raw.chars().skip(count - TAIL).collect();
        format!("{head} … {tail}")
    };
    crate::domain::tool::ToolResult {
        content: format!(
            "the arguments for tool '{tool}' were not a JSON object, so it was not run (they \
             may have been cut off at the output limit). Resend the call with one complete JSON \
             object of arguments. Received: {received}"
        ),
        image_blocks: vec![],
        delivery_metadata: None,
        is_error: true,
    }
}
