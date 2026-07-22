use super::*;

#[derive(Debug, Clone)]
pub(crate) struct PendingMessageRecovery {
    pub(crate) message_id: String,
    pub(crate) batch_id: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) content: String,
    pub(crate) offset: usize,
}

#[derive(Debug)]
pub(crate) struct MessageRecoveryBatch {
    pub(crate) refs: Vec<String>,
    pub(crate) responses: std::collections::HashMap<String, serde_json::Value>,
    pub(crate) target_start: usize,
    pub(crate) target_end: usize,
    pub(crate) agent_id: Option<String>,
}

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
        let force = self.open_tool_calls > 0;
        if !force
            && !self.needs_message_recovery_for(
                refs,
                &self.latest_assistant_text(),
                self.tools_this_turn,
                expected_content_len,
            )
        {
            return;
        }
        if refs.iter().any(|message_id| {
            self.pending_message_recovery
                .values()
                .any(|pending| pending.message_id == *message_id)
        }) {
            return;
        }
        let batch_id = format!("recovery-batch-{}", super::app_events::uuid_like());
        let target_end = self.master_session.chat.entry_count();
        self.message_recovery_batches.insert(
            batch_id.clone(),
            MessageRecoveryBatch {
                refs: refs.to_vec(),
                responses: std::collections::HashMap::new(),
                target_start: self.active_turn_start.min(target_end),
                target_end,
                agent_id: None,
            },
        );
        for message_id in refs {
            if self
                .pending_message_recovery
                .values()
                .any(|pending| pending.message_id == *message_id)
            {
                continue;
            }
            let req_id = format!("msg-recovery-{}", super::app_events::uuid_like());
            self.pending_message_recovery.insert(
                req_id.clone(),
                PendingMessageRecovery {
                    message_id: message_id.clone(),
                    batch_id: batch_id.clone(),
                    agent_id: None,
                    content: String::new(),
                    offset: 0,
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

    pub(super) fn needs_message_recovery_for(
        &self,
        refs: &[String],
        assistant_text: &str,
        tools_this_turn: usize,
        expected_content_len: Option<u64>,
    ) -> bool {
        if refs.is_empty() {
            return false;
        }
        let trimmed = assistant_text.trim();
        if trimmed.is_empty() || trimmed == "…" || trimmed == "..." {
            return true;
        }
        if let Some(expected) = expected_content_len
            && (assistant_text.len() as u64) < expected
        {
            return true;
        }
        let expected_refs = tools_this_turn.saturating_mul(2).saturating_add(1);
        refs.len() != expected_refs
    }

    fn latest_assistant_text(&self) -> String {
        self.master_session
            .chat
            .entries()
            .iter()
            .rev()
            .find_map(|e| match e {
                crate::interface::components::chat::ChatEntry::Assistant { text, .. } => {
                    Some(text.clone())
                }
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
        let Some(pending) = self.pending_message_recovery.remove(req_id) else {
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
        if data.get("id").and_then(|v| v.as_str()) != Some(pending.message_id.as_str()) {
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        }
        let update =
            super::range_accumulator::RangeAccumulator::new(pending.content, pending.offset)
                .apply(&data);
        let accumulated = match update {
            Ok(super::range_accumulator::RangeUpdate::Continue {
                content,
                next_offset,
            }) => {
                let req_id = format!("msg-recovery-{}", super::app_events::uuid_like());
                let message_id = pending.message_id;
                let batch_id = pending.batch_id;
                let agent_id = pending.agent_id;
                self.pending_message_recovery.insert(
                    req_id.clone(),
                    PendingMessageRecovery {
                        message_id: message_id.clone(),
                        batch_id,
                        agent_id: agent_id.clone(),
                        content,
                        offset: next_offset,
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
            Ok(super::range_accumulator::RangeUpdate::Complete(content)) => content,
            Err(_) => {
                self.abandon_recovery_batch(&pending.batch_id);
                return;
            }
        };
        let mut data = data;
        data["content"] = serde_json::Value::String(accumulated);
        data["hasMoreContent"] = serde_json::Value::Bool(false);
        let Some(batch) = self.message_recovery_batches.get_mut(&pending.batch_id) else {
            return;
        };
        batch.responses.insert(pending.message_id, data);
        if batch.responses.len() != batch.refs.len() {
            return;
        }
        let batch = self
            .message_recovery_batches
            .remove(&pending.batch_id)
            .unwrap();
        let entries = recovered_chat_entries(&batch.refs, &batch.responses);
        match &batch.agent_id {
            None => {
                self.master_session.chat.replace_range(
                    batch.target_start,
                    batch.target_end,
                    entries,
                );
            }
            Some(child) => {
                if let Some(session) = self.subagents.sessions.get_mut(child) {
                    let recovered_len = entries.len();
                    if recovered_len > 0 {
                        for message_id in &batch.refs {
                            session.seen_message_ids.insert(message_id.clone());
                        }
                    }
                    session
                        .chat
                        .replace_range(batch.target_start, batch.target_end, entries);
                    session.master_stream_appended_len = session
                        .master_stream_appended_len
                        .saturating_sub(batch.target_end.saturating_sub(batch.target_start))
                        .saturating_add(recovered_len);
                }
            }
        }
    }

    pub(super) fn abandon_recovery_batch(&mut self, batch_id: &str) {
        self.message_recovery_batches.remove(batch_id);
        self.pending_message_recovery
            .retain(|_, pending| pending.batch_id != batch_id);
    }
}

pub(crate) fn recovered_chat_entries(
    refs: &[String],
    responses: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<crate::interface::components::chat::ChatEntry> {
    use crate::interface::components::chat::ChatEntry;
    let mut entries = Vec::new();
    let mut tools = std::collections::HashMap::<String, usize>::new();
    let mut suppressed_calls = std::collections::HashSet::<String>::new();
    for message_id in refs {
        let Some(data) = responses.get(message_id) else {
            continue;
        };
        let role = data.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        match role {
            "user" if !content.is_empty() => {
                entries.push(ChatEntry::User {
                    text: content.to_string(),
                });
            }
            "assistant" => {
                if let Some(calls) = data
                    .get("toolCalls")
                    .or_else(|| data.get("tool_calls"))
                    .and_then(|v| v.as_array())
                {
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if id.is_empty() {
                            continue;
                        }
                        let name = call
                            .get("name")
                            .or_else(|| call.pointer("/function/name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string();
                        if super::app_events::suppress_tool_box(&name, &serde_json::Value::Null) {
                            suppressed_calls.insert(id);
                            continue;
                        }
                        let args = call
                            .get("arguments")
                            .or_else(|| call.pointer("/function/arguments"))
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| v.to_string())
                            })
                            .unwrap_or_else(|| "{}".into());
                        tools.insert(id.clone(), entries.len());
                        entries.push(ChatEntry::ToolExecution {
                            tool_call_id: id,
                            tool_name: name,
                            parsed_args: serde_json::from_str(&args).ok(),
                            args,
                            result: None,
                            is_error: false,
                            duration_ms: None,
                        });
                    }
                }
                if !content.is_empty() {
                    entries.push(ChatEntry::Assistant {
                        text: content.to_string(),
                        streaming: false,
                    });
                }
            }
            "tool" => {
                let call_id = data
                    .get("toolCallId")
                    .or_else(|| data.get("tool_call_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if suppressed_calls.contains(&call_id) {
                    continue;
                }
                let name = data
                    .get("toolName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let is_error = data
                    .get("isError")
                    .or_else(|| data.get("is_error"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if let Some(idx) = tools.get(&call_id).copied()
                    && let Some(ChatEntry::ToolExecution {
                        result,
                        is_error: err,
                        ..
                    }) = entries.get_mut(idx)
                {
                    *result = Some(content.to_string());
                    *err = is_error;
                    continue;
                }
                if !call_id.is_empty() {
                    entries.push(ChatEntry::ToolExecution {
                        tool_call_id: call_id,
                        tool_name: name,
                        parsed_args: None,
                        args: String::new(),
                        result: Some(content.to_string()),
                        is_error,
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
    use crate::interface::components::chat::ChatEntry;

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
                    "toolName": "bash",
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
