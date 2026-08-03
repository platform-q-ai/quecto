//! Tool-call execution for the agent loop: appends the assistant tool-call
//! message and each tool result to the conversation, recording every append
//! in the per-run ledger (#1072).

use super::*;

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
        let tool_call_count = messages[assistant_index].tool_calls.len();

        for idx in 0..tool_call_count {
            let tc = &messages[assistant_index].tool_calls[idx];
            // Audit: ToolCall (guarded — avoid clones when audit is disabled)
            if self.audit_log.is_some() {
                self.audit(
                    current_turn,
                    AuditEvent::ToolCall {
                        tool: tc.name.clone(),
                        call_id: tc.id.clone(),
                        arguments: tc.arguments.clone(),
                    },
                )
                .await;
            }

            let (content, image_blocks, is_error) = self.execute_single_tool_call(tc).await;

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
            run_ledger.push(tool_msg.clone());
            messages.push(tool_msg);
        }
    }

    async fn execute_single_tool_call(
        &self,
        tc: &ToolCall,
    ) -> (String, Vec<crate::domain::tool::ImageBlock>, bool) {
        // Emit ToolStarted before executing so the REPL can show the tool name
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
        let tool_result = if disabled_by_runtime_policy {
            Ok(crate::domain::tool::ToolResult {
                content: format!("tool '{}' is disabled by runtime policy", tc.name),
                image_blocks: vec![],
                is_error: true,
            })
        } else {
            self.tool_executor().execute(&tc.name, &tc.arguments).await
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        let (content, image_blocks, is_err) = match tool_result {
            Ok(tr) => (tr.content, tr.image_blocks, tr.is_error),
            Err(e) => (format!("Error: {}", e), vec![], true),
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
        (content, image_blocks, is_err)
    }
}
