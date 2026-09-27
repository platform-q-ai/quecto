//! Tool-call execution for the agent loop: appends the assistant tool-call
//! message and each tool result to the conversation, recording every append
//! in the per-run ledger (#1072).

use super::*;
use crate::application::tool_panic_scope;

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
    Prepared(Box<PreparedResult>),
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
                Slot::Prepared(result) => *result,
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
                    let outcome = self.execute_single_tool_call(current_turn, tc).await;
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
            for (idx, tc) in calls.iter().enumerate() {
                let Slot::Ran(outcome, elapsed) =
                    std::mem::replace(&mut batch.slots[idx], Slot::Running)
                else {
                    unreachable!("every call ran before any is prepared");
                };
                let finished = FinishedCall {
                    idx,
                    tc,
                    outcome,
                    elapsed,
                };
                let prepared = self.prepare_tool_result(current_turn, finished).await;
                batch.slots[idx] = Slot::Prepared(Box::new(prepared));
            }
        } else {
            // One at a time, each result appended before the next call runs.
            for idx in 0..call_count {
                let result = {
                    let tc = &messages[assistant_index].tool_calls[idx];
                    self.audit_tool_call(current_turn, tc).await;
                    let started = std::time::Instant::now();
                    let outcome = self.execute_single_tool_call(current_turn, tc).await;
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
            // A failure keeps its end too, where its cause is (#2159).
            let preview = match is_error {
                true => crate::domain::audit::error_preview(&content, 200, 800),
                false => crate::domain::audit::content_preview(&content, 200),
            };
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
        current_turn: u32,
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
                    self.execute_contained(current_turn, tc).await
                }
            }
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        let tr =
            tool_result.unwrap_or_else(|error| crate::domain::tool::ToolResult::from_error(&error));
        let (content, image_blocks, delivery_metadata, is_err) = (
            tr.content,
            tr.image_blocks,
            tr.delivery_metadata,
            tr.is_error,
        );

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

    /// Run a call's tool with a panic contained to that call (#2192): a tool
    /// that panics, whether before it returns its future or while it runs,
    /// answers an error result naming the crash, the event log records an
    /// `error` event with the panic's message and location, and the turn
    /// goes on. The call runs inside its tool scope, so the process panic
    /// hook lets the panic unwind to here instead of aborting, and records
    /// where it happened. The panic unwinds only through the tool's own
    /// frames; the loop holds no lock across this await, so none of its
    /// state is left half-updated.
    async fn execute_contained(
        &self,
        current_turn: u32,
        tc: &ToolCall,
    ) -> Result<crate::domain::tool::ToolResult, DomainError> {
        use futures::FutureExt;
        let arguments = tc.wire_arguments();
        let scope = tool_panic_scope::ToolScope::in_turn(tc.name.as_str(), current_turn);
        let run = async { self.tool_executor().execute(&tc.name, &arguments).await };
        let contained = std::panic::AssertUnwindSafe(tool_panic_scope::scoped(scope.clone(), run))
            .catch_unwind()
            .await;
        // Caught: the unwind the hook let through is over on this thread.
        tool_panic_scope::end_contained_unwind();
        // The call's future is gone, and with it the scope is closed: no
        // panic can be recorded on it any more (a later one is fatal), so
        // the record read below is final (#2192 review).
        assert!(
            !scope.is_open(),
            "the call's scope is closed before it is read"
        );
        let site = match contained {
            // A panic the call swallowed (carried work whose join error it
            // read as "no output") still fails the call: its answer may be
            // wrong while looking like success (#2192).
            Ok(result) => match scope.recorded_panic() {
                None => return result,
                Some(site) => site,
            },
            // The hook saw the panic first and kept its location; without a
            // hook (an embedding that installed none) the payload names it.
            Err(payload) => scope
                .recorded_panic()
                .unwrap_or_else(|| tool_panic_scope::PanicSite {
                    message: tool_panic_scope::payload_message(payload.as_ref()),
                    location: None,
                }),
        };
        // The message goes to the provider and the event log: secret shapes
        // redacted, and cut to a bound (#2192 review).
        let message = shown_panic_message(&site.message);
        tracing::error!(
            target: "tool_exec",
            tool_name = tc.name.as_str(),
            call_id = tc.id.as_str(),
            panic = message.as_str(),
            location = site.location.as_deref().unwrap_or("unknown"),
            "tool panicked; its call answers an error and the turn goes on"
        );
        self.audit(
            current_turn,
            AuditEvent::Error {
                source: tool_panic_scope::TOOL_PANIC_SOURCE.to_string(),
                tool: Some(tc.name.clone()),
                message: message.clone(),
                location: site.location.clone(),
            },
        )
        .await;
        Ok(crashed_tool_result(&tc.name, &message))
    }
}

/// The most of a panic's message a result or event carries, in bytes.
pub const MAX_PANIC_MESSAGE_BYTES: usize = 1024;

/// A panic's message as it may leave the process: known secret shapes
/// redacted first (so a cut never hides one from the redaction), then cut
/// on a character boundary to [`MAX_PANIC_MESSAGE_BYTES`], marked "…".
pub fn shown_panic_message(message: &str) -> String {
    let redacted = crate::domain::redaction::redact_secrets(message);
    let mut end = redacted.len().min(MAX_PANIC_MESSAGE_BYTES);
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    match end < redacted.len() {
        true => format!("{}…", &redacted[..end]),
        false => redacted,
    }
}

/// The error result of a call whose tool panicked (#2192).
fn crashed_tool_result(tool: &str, message: &str) -> crate::domain::tool::ToolResult {
    crate::domain::tool::ToolResult {
        content: format!(
            "internal error in tool '{tool}': {message}; the call stopped at the panic, and any \
             partial effects it had already made may remain"
        ),
        image_blocks: vec![],
        delivery_metadata: None,
        is_error: true,
    }
}

/// The error returned for a call whose arguments are not a JSON object
/// (#2123). It shows both ends of what was received: for a call cut off at
/// the output limit, the end is where it broke.
fn invalid_arguments(tool: &str, raw: &str) -> crate::domain::tool::ToolResult {
    let received = crate::domain::audit::error_preview(raw, 200, 300);
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
