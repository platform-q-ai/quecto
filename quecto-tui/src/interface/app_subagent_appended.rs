use super::*;
use crate::interface::app::app_message_recovery::recovered_chat_entries;

impl App {
    /// Route the parent's forwarded child turn-appended event into the retained
    /// child session even when that child is not focused (#1186). Direct
    /// connect-on-select streaming remains authoritative for the selected child;
    /// when stable refs are available, ids make this path idempotent across
    /// master-stream delivery, later focus backfill, and repeated events.
    pub(super) fn handle_subagent_messages_appended(
        &mut self,
        agent_id: String,
        messages: Vec<serde_json::Value>,
        message_refs: Vec<String>,
    ) {
        let agent_id = sanitize_agent_id(&agent_id);
        if !self.is_retained_or_tracked_agent(&agent_id) {
            return;
        }
        if self.subagents.active_agent_id.as_deref() == Some(agent_id.as_str())
            && self
                .subagents
                .active_conn
                .as_ref()
                .is_some_and(|(connected, _)| connected == &agent_id)
        {
            return;
        }
        self.ensure_session(&agent_id);
        if messages.is_empty() {
            if !message_refs.is_empty() {
                self.recover_appended_child_message_refs(&agent_id, &message_refs);
            }
            return;
        }
        let Some(session) = self.subagents.sessions.get_mut(&agent_id) else {
            return;
        };
        let entries = Self::appended_child_message_entries(session, &messages, &message_refs);
        if entries.is_empty() {
            return;
        }
        for entry in entries {
            session.chat.add_entry(entry);
            session.master_stream_appended_len += 1;
        }
    }

    fn appended_child_message_entries(
        session: &mut SessionView,
        messages: &[serde_json::Value],
        message_refs: &[String],
    ) -> Vec<ChatEntry> {
        let identities = Self::appended_child_message_identities(messages, message_refs);
        let new_refs: Vec<String> = identities
            .iter()
            .filter_map(|id| {
                id.as_ref()
                    .and_then(|id| (!session.seen_message_ids.contains(id)).then(|| id.clone()))
            })
            .collect();
        if identities.iter().all(Option::is_some) && new_refs.is_empty() {
            return Vec::new();
        }
        let entries = recovered_chat_entries(
            &new_refs,
            &Self::responses_by_identity(messages, &identities),
        )
        .into_iter()
        .map(Self::sanitize_appended_child_entry)
        .collect::<Vec<_>>();
        if !entries.is_empty() {
            for id in new_refs {
                session.seen_message_ids.insert(id);
            }
            return entries;
        }
        Self::plain_appended_child_message_entries(session, messages, message_refs)
    }

    fn appended_child_message_identities(
        messages: &[serde_json::Value],
        message_refs: &[String],
    ) -> Vec<Option<String>> {
        messages
            .iter()
            .enumerate()
            .map(|(idx, message)| {
                message
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| message_refs.get(idx).cloned())
            })
            .collect()
    }

    fn responses_by_identity(
        messages: &[serde_json::Value],
        identities: &[Option<String>],
    ) -> std::collections::HashMap<String, serde_json::Value> {
        identities
            .iter()
            .zip(messages.iter())
            .filter_map(|(id, message)| id.as_ref().map(|id| (id.clone(), message.clone())))
            .collect()
    }

    fn plain_appended_child_message_entries(
        session: &mut SessionView,
        messages: &[serde_json::Value],
        message_refs: &[String],
    ) -> Vec<ChatEntry> {
        use crate::application::session_payloads::{self, ResumedChatMessage};
        let data = serde_json::json!({ "messages": messages });
        let Ok(messages) = session_payloads::parse_resumed_messages(&data) else {
            return Vec::new();
        };
        messages
            .into_iter()
            .enumerate()
            .filter_map(|(idx, message)| {
                let (text, id, stub, is_user) = match message {
                    ResumedChatMessage::User { text, id, stub } => (text, id, stub, true),
                    ResumedChatMessage::Assistant { text, id, stub } => (text, id, stub, false),
                };
                let identity = id.clone().or_else(|| message_refs.get(idx).cloned());
                if identity
                    .as_ref()
                    .is_some_and(|id| !session.seen_message_ids.insert(id.clone()))
                {
                    return None;
                }
                let text = crate::interface::ansi::sanitize_control_keep_newlines(&text);
                Some(Self::history_entry(text, id, stub, is_user))
            })
            .collect()
    }

    fn sanitize_appended_child_entry(entry: ChatEntry) -> ChatEntry {
        match entry {
            ChatEntry::Assistant { text, streaming } => ChatEntry::Assistant {
                text: crate::interface::ansi::sanitize_control_keep_newlines(&text),
                streaming,
            },
            ChatEntry::ToolExecution {
                tool_call_id,
                tool_name,
                args,
                parsed_args,
                result,
                is_error,
                duration_ms,
            } => ChatEntry::ToolExecution {
                tool_call_id,
                tool_name: crate::interface::ansi::sanitize_control_keep_newlines(&tool_name),
                args: crate::interface::ansi::sanitize_control_keep_newlines(&args),
                parsed_args,
                result: result
                    .map(|result| crate::interface::ansi::sanitize_control_keep_newlines(&result)),
                is_error,
                duration_ms,
            },
            entry => entry,
        }
    }

    fn recover_appended_child_message_refs(&mut self, agent_id: &str, refs: &[String]) {
        if refs.is_empty()
            || refs.iter().all(|message_id| {
                self.subagents
                    .sessions
                    .get(agent_id)
                    .is_some_and(|session| session.seen_message_ids.contains(message_id))
            })
        {
            return;
        }
        if refs.iter().any(|message_id| {
            self.pending_message_recovery.values().any(|pending| {
                pending.agent_id.as_deref() == Some(agent_id) && pending.message_id == *message_id
            })
        }) {
            return;
        }
        let Some(session) = self.subagents.sessions.get(agent_id) else {
            return;
        };
        let batch_id = format!(
            "child-appended-{agent_id}-{}",
            super::app_events::uuid_like()
        );
        let target = session.chat.entry_count();
        let reserved = refs.len();
        if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
            session.chat.replace_range(
                target,
                target,
                (0..reserved)
                    .map(|_| ChatEntry::Status {
                        text: String::new(),
                    })
                    .collect(),
            );
            session.master_stream_appended_len += reserved;
        }
        self.message_recovery_batches.insert(
            batch_id.clone(),
            MessageRecoveryBatch {
                refs: refs.to_vec(),
                responses: std::collections::HashMap::new(),
                target_start: target,
                target_end: target + reserved,
                agent_id: Some(agent_id.to_string()),
            },
        );
        for message_id in refs {
            let req_id = format!("msg-recovery-{}", super::app_events::uuid_like());
            self.pending_message_recovery.insert(
                req_id.clone(),
                PendingMessageRecovery {
                    message_id: message_id.clone(),
                    batch_id: batch_id.clone(),
                    agent_id: Some(agent_id.to_string()),
                    content: String::new(),
                    offset: 0,
                },
            );
            self.send_command(Command::GetMessage {
                id: Some(req_id),
                message_id: message_id.clone(),
                agent_id: Some(agent_id.to_string()),
                tool_call_id: None,
                offset: Some(0),
                limit: Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES),
            });
        }
    }
}

#[cfg(test)]
#[path = "app_subagent_appended_tests.rs"]
mod tests;
