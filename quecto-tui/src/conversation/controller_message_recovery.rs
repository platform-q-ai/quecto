use super::*;

#[derive(Debug, Clone)]
pub(crate) struct PendingMessageRecovery {
    pub(crate) message_id: String,
    pub(crate) batch_id: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) content: String,
    pub(crate) offset: usize,
    pub(crate) content_len: Option<usize>,
}

/// A turn awaiting rebuild from its refs. The atomicity invariant lives in
/// `conversation::turn_recovery`; this alias binds it to the raw response payloads
/// the shell composition layer buffers.
pub(crate) type MessageRecoveryBatch =
    crate::conversation::turn_recovery::RecoveryBatch<serde_json::Value>;

impl App {
    /// Both `turn_end` and `agent_end` may carry the same refs; skip message ids
    /// that already have an in-flight recovery request so we never double-fetch.
    pub(super) fn maybe_recover_from_refs(&mut self, refs: &[String]) {
        self.maybe_recover_from_refs_with_len(refs, None);
    }

    pub(super) fn maybe_recover_from_refs_with_len(
        &mut self,
        refs: &[String],
        expected_content_len: Option<u64>,
    ) {
        if refs.is_empty() {
            return;
        }
        // Fetch the rendered text lazily: the policy can force recovery without
        // reading it, and the bounded assistant text scan clones the assistant
        // body — which can be megabytes for an inlined command dump.
        use crate::conversation::turn_recovery::TurnOutcome;
        let open_tool_calls = self.ac().master_session.open_tool_calls;
        let tools_this_turn = self.ac().master_session.tools_this_turn;
        let target_end = self.ac().master_session.chat.entry_count();
        let target_start = self.ac().master_session.active_turn_start.min(target_end);
        let needs_recovery = TurnOutcome::forced_without_text(refs, open_tool_calls) || {
            let assistant_text = self.latest_assistant_text_in_range(target_start, target_end);
            TurnOutcome {
                refs,
                assistant_text: &assistant_text,
                tools_this_turn,
                open_tool_calls,
                expected_content_len,
            }
            .needs_recovery()
        };
        if !needs_recovery {
            return;
        }
        if refs.iter().any(|message_id| {
            self.ac()
                .pending_message_recovery
                .values()
                .any(|pending| pending.agent_id.is_none() && pending.message_id == *message_id)
        }) {
            return;
        }
        let batch_id = format!(
            "{}recovery-batch-{}",
            self.ac().id_namespace(),
            super::app_events::uuid_like()
        );
        self.ac_mut().message_recovery_batches.insert(
            batch_id.clone(),
            MessageRecoveryBatch::new(refs.to_vec(), target_start, target_end, None),
        );
        for message_id in refs {
            if self
                .ac()
                .pending_message_recovery
                .values()
                .any(|pending| pending.agent_id.is_none() && pending.message_id == *message_id)
            {
                continue;
            }
            let req_id = format!(
                "{}msg-recovery-{}",
                self.ac().id_namespace(),
                super::app_events::uuid_like()
            );
            self.ac_mut().pending_message_recovery.insert(
                req_id.clone(),
                PendingMessageRecovery {
                    message_id: message_id.clone(),
                    batch_id: batch_id.clone(),
                    agent_id: None,
                    content: String::new(),
                    offset: 0,
                    content_len: (refs.len() == 1)
                        .then_some(expected_content_len)
                        .flatten()
                        .and_then(|n| usize::try_from(n).ok()),
                },
            );
            self.send_command(Command::GetMessage {
                id: Some(req_id),
                message_id: message_id.clone(),
                agent_id: None,
                tool_call_id: None,
                offset: Some(0),
                limit: Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES),
            });
        }
    }

    fn latest_assistant_text_in_range(&self, start: usize, end: usize) -> String {
        let entries = self.ac().master_session.chat.entries();
        entries[start.min(entries.len())..end.min(entries.len())]
            .iter()
            .rev()
            .find_map(|e| match e {
                crate::components::chat::ChatEntry::Assistant { text, .. } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Buffer a gated response and atomically replace the original turn once all
    /// refs arrive. Unknown, stale, failed, or mismatched responses never mutate
    /// chat state.
    pub(super) fn handle_get_message_recovery(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<serde_json::Value>,
    ) {
        let Some(req_id) = id else { return };
        let Some(pending) = self.ac_mut().pending_message_recovery.remove(req_id) else {
            return;
        };
        if !success {
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        }
        let Some(data) = data else {
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        };
        if crate::protocol::presentation_payloads::response_identity(&data)
            .0
            .as_deref()
            != Some(pending.message_id.as_str())
        {
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        }
        let update = crate::protocol::range_accumulator::RangeAccumulator::new_with_expected_len(
            pending.content,
            pending.offset,
            pending.content_len,
        )
        .apply(&data);
        let accumulated = match update {
            Ok(crate::protocol::range_accumulator::RangeUpdate::Continue {
                content,
                next_offset,
                content_len,
            }) => {
                let req_id = format!(
                    "{}msg-recovery-{}",
                    self.ac().id_namespace(),
                    super::app_events::uuid_like()
                );
                let message_id = pending.message_id;
                let batch_id = pending.batch_id;
                let agent_id = pending.agent_id;
                self.ac_mut().pending_message_recovery.insert(
                    req_id.clone(),
                    PendingMessageRecovery {
                        message_id: message_id.clone(),
                        batch_id,
                        agent_id: agent_id.clone(),
                        content,
                        offset: next_offset,
                        content_len,
                    },
                );
                self.send_command(Command::GetMessage {
                    id: Some(req_id),
                    message_id,
                    agent_id,
                    tool_call_id: None,
                    offset: Some(next_offset),
                    limit: Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES),
                });
                return;
            }
            Ok(crate::protocol::range_accumulator::RangeUpdate::Complete(content)) => content,
            Err(_) => {
                self.abandon_recovery_batch(&pending.batch_id);
                return;
            }
        };
        let mut data = data;
        data["content"] = serde_json::Value::String(accumulated);
        data["hasMoreContent"] = serde_json::Value::Bool(false);
        let Some(batch) = self
            .ac_mut()
            .message_recovery_batches
            .get_mut(&pending.batch_id)
        else {
            return;
        };
        batch.responses.insert(pending.message_id, data);
        if !batch.is_complete() {
            return;
        }
        let batch = self
            .ac_mut()
            .message_recovery_batches
            .remove(&pending.batch_id)
            .unwrap();
        let entries = recovered_chat_entries(&batch.refs, &batch.responses);
        match &batch.agent_id {
            None => {
                self.ac_mut().master_session.chat.replace_range(
                    batch.target_start,
                    batch.target_end,
                    entries,
                );
                self.reconcile_master_retention_trim();
            }
            Some(child) => {
                if let Some(session) = self.ac_mut().roster.sessions.get_mut(child) {
                    session
                        .chat
                        .replace_range(batch.target_start, batch.target_end, entries);
                    session.reconcile_chat_retention_trim();
                }
            }
        }
    }

    pub(super) fn abandon_recovery_batch(&mut self, batch_id: &str) {
        self.ac_mut().message_recovery_batches.remove(batch_id);
        self.ac_mut()
            .pending_message_recovery
            .retain(|_, pending| pending.batch_id != batch_id);
    }
}

pub(crate) fn recovered_chat_entries(
    refs: &[String],
    responses: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<crate::components::chat::ChatEntry> {
    use crate::components::chat::ChatEntry;
    let mut entries = Vec::new();
    let mut tools = std::collections::HashMap::<String, usize>::new();
    let mut suppressed_calls = std::collections::HashSet::<String>::new();
    // Ordering is the domain's rule, not this function's: walk in ref order,
    // never arrival order.
    for data in crate::conversation::turn_recovery::ordered_by_refs(refs, responses) {
        let message = crate::protocol::presentation_payloads::recovered_message(data);
        let role = message.role();
        let content = message.content();
        match role {
            // Sub-agent notes are user-role turns on the wire but operator
            // status in the UI; the live event path already renders them (#1338).
            "user"
                if !content.is_empty()
                    && !crate::protocol::presentation_payloads::is_subagent_note(content) =>
            {
                entries.push(ChatEntry::User {
                    text: content.to_string(),
                })
            }
            "assistant" => {
                for call in message.tool_calls() {
                    let id = call.id().to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let name = call.name().to_string();
                    if super::app_events::suppress_tool_box(&name, &serde_json::Value::Null) {
                        suppressed_calls.insert(id);
                        continue;
                    }
                    let args = call.arguments();
                    tools.insert(id.clone(), entries.len());
                    entries.push(ChatEntry::ToolExecution {
                        tool_call_id: id,
                        tool_name: name,
                        parsed_args: crate::protocol::agent_ledger_payloads::parse_tool_args(&args),
                        args,
                        result: None,
                        is_error: false,
                        duration_ms: None,
                    });
                }
                if !content.is_empty() {
                    entries.push(ChatEntry::Assistant {
                        text: content.to_string(),
                        streaming: false,
                    });
                }
            }
            "tool" => {
                let call_id = message.tool_call_id().to_string();
                if suppressed_calls.contains(&call_id) {
                    continue;
                }
                if let Some(idx) = tools.get(&call_id).copied()
                    && let Some(ChatEntry::ToolExecution {
                        result, is_error, ..
                    }) = entries.get_mut(idx)
                {
                    *result = Some(content.to_string());
                    *is_error = message.is_error();
                    continue;
                }
                if !call_id.is_empty() {
                    entries.push(ChatEntry::ToolExecution {
                        tool_call_id: call_id,
                        tool_name: message.tool_name().to_string(),
                        parsed_args: None,
                        args: String::new(),
                        result: Some(content.to_string()),
                        is_error: message.is_error(),
                        duration_ms: None,
                    });
                }
            }
            _ => {}
        }
    }
    entries
}

#[cfg(test)]
mod recovery_cov_tests {
    use super::*;
    use crate::components::chat::ChatEntry;

    fn app_for_recovery_test() -> App {
        App::new(
            crate::shell::terminal::Terminal::new(),
            crate::protocol::client::Client::disconnected_for_tests(),
        )
    }

    #[test]
    fn recovery_decision_ignores_assistant_text_before_active_turn() {
        let mut app = app_for_recovery_test();
        app.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Assistant {
                text: "previous complete answer".to_string(),
                streaming: false,
            });
        app.ac_mut().master_session.active_turn_start =
            app.ac_mut().master_session.chat.entry_count();

        app.maybe_recover_from_refs(&["current-ref".to_string()]);

        assert_eq!(app.ac().pending_message_recovery.len(), 1);
        assert_eq!(app.ac().message_recovery_batches.len(), 1);
        let batch = app.ac().message_recovery_batches.values().next().unwrap();
        assert_eq!(
            batch.target_start,
            app.ac().master_session.active_turn_start
        );
        assert_eq!(batch.target_end, app.ac().master_session.chat.entry_count());
        let pending = app.ac().pending_message_recovery.values().next().unwrap();
        assert_eq!(pending.message_id, "current-ref");
    }

    #[test]
    fn recovery_decision_uses_complete_assistant_text_in_active_turn() {
        let mut app = app_for_recovery_test();
        app.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Assistant {
                text: "previous complete answer".to_string(),
                streaming: false,
            });
        app.ac_mut().master_session.active_turn_start =
            app.ac_mut().master_session.chat.entry_count();
        app.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Assistant {
                text: "current complete answer".to_string(),
                streaming: false,
            });

        app.maybe_recover_from_refs(&["current-ref".to_string()]);

        assert!(app.ac().pending_message_recovery.is_empty());
        assert!(app.ac().message_recovery_batches.is_empty());
    }

    #[test]
    fn recovered_chat_entries_handles_suppressed_calls_errors_and_unknown_roles() {
        let refs = vec![
            "suppressed-start".to_string(),
            "suppressed-result".to_string(),
            "standalone-tool".to_string(),
            "assistant-text".to_string(),
            "unknown".to_string(),
        ];
        let responses = std::collections::HashMap::from([
            (
                "suppressed-start".to_string(),
                serde_json::json!({
                    "role": "assistant",
                    "toolCalls": [{"id": "spawn-1", "name": "spawn", "arguments": {"task": "secret"}}]
                }),
            ),
            (
                "suppressed-result".to_string(),
                serde_json::json!({
                    "role": "tool",
                    "toolCallId": "spawn-1",
                    "toolName": "spawn",
                    "content": "hidden search result"
                }),
            ),
            (
                "standalone-tool".to_string(),
                serde_json::json!({
                    "role": "tool",
                    "toolCallId": "call-2",
                    "tool_name": "bash",
                    "content": "boom",
                    "isError": true
                }),
            ),
            (
                "assistant-text".to_string(),
                serde_json::json!({"role": "assistant", "content": "visible answer"}),
            ),
            (
                "unknown".to_string(),
                serde_json::json!({"role": "system", "content": "ignored"}),
            ),
        ]);

        let entries = recovered_chat_entries(&refs, &responses);

        assert_eq!(
            entries.len(),
            2,
            "suppressed and unknown records are skipped: {entries:?}"
        );
        match &entries[0] {
            ChatEntry::ToolExecution {
                tool_call_id,
                tool_name,
                result,
                is_error,
                ..
            } => {
                assert_eq!(tool_call_id, "call-2");
                assert_eq!(tool_name, "bash");
                assert_eq!(result.as_deref(), Some("boom"));
                assert!(*is_error);
            }
            other => panic!("expected standalone tool entry, got {other:?}"),
        }
        match &entries[1] {
            ChatEntry::Assistant { text, streaming } => {
                assert_eq!(text, "visible answer");
                assert!(!streaming);
            }
            other => panic!("expected assistant entry, got {other:?}"),
        }
    }
}
