use super::*;

/// A demoted history stub whose full body has been requested (#1061 auto-recall).
/// Owning session: `agent_id` None = master, `Some(id)` = that sub-agent's chat.
#[derive(Debug, Clone)]
pub(crate) struct StubRecall {
    pub(super) agent_id: Option<String>,
    pub(super) message_id: String,
}

impl App {
    pub(super) fn request_active_older_history_page(&mut self) {
        let Some((id, before, target_child)) = self.next_history_page_request() else {
            return;
        };
        let cmd = Command::GetMessages {
            id: Some(id),
            before: Some(before),
        };
        if target_child {
            if !self.send_to_active_subagent(cmd) {
                self.active_session_mut().history_pending_before_cursor = None;
                self.notify(
                    "Selected sub-agent is not ready for history backfill yet",
                    NotifyLevel::Warning,
                );
            }
        } else {
            self.send_command(cmd);
        }
    }

    pub(super) fn next_history_page_request(&mut self) -> Option<(String, String, bool)> {
        let target_child = self.subagents.active_agent_id.is_some();
        let session = self.active_session_mut();
        if !session.history_has_more_before {
            return None;
        }
        let before = session.history_before_cursor.clone()?;
        if session.history_pending_before_cursor.as_deref() == Some(before.as_str()) {
            return None;
        }
        session.history_pending_before_cursor = Some(before.clone());
        session.history_page_seq = session.history_page_seq.wrapping_add(1);
        Some((
            format!("history-page-{}", session.history_page_seq),
            before,
            target_child,
        ))
    }

    /// Auto-recall (#1061): request full content for any demoted history stub
    /// now loaded in the active session, so scrolling back reveals real content
    /// in place of the ladder stub. Each stub is fetched at most once while in
    /// flight (deduped by message id). A sub-agent stub is fetched through the
    /// MASTER connection carrying the child's agent id — mirroring #1060 recovery
    /// routing — so the response returns on the master stream and is applied to
    /// the child's chat.
    pub(super) fn request_active_visible_stub_recalls(&mut self) {
        let agent_id = self.subagents.active_agent_id.clone();
        let stub_ids = self.active_session().chat.stub_message_ids();
        for message_id in stub_ids {
            if self
                .pending_stub_recall
                .values()
                .any(|r| r.message_id == message_id)
            {
                continue;
            }
            let req_id = format!("stub-recall-{}", super::app_events::uuid_like());
            self.pending_stub_recall.insert(
                req_id.clone(),
                StubRecall {
                    agent_id: agent_id.clone(),
                    message_id: message_id.clone(),
                },
            );
            self.send_command(Command::GetMessage {
                id: Some(req_id),
                message_id,
                agent_id: agent_id.clone(),
            });
        }
    }

    /// Apply a `get_message` response issued to auto-recall a demoted history
    /// stub (#1061). Replaces the stub body in place for the owning session and
    /// returns whether `req_id` was a stub-recall request (so the caller skips
    /// the #1060 recovery path). A failed, no-data, or mismatched response just
    /// drops the pending entry, leaving the stub to retry on the next scroll.
    pub(super) fn handle_stub_recall_response(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<&serde_json::Value>,
    ) -> bool {
        let Some(req_id) = id else { return false };
        let Some(recall) = self.pending_stub_recall.remove(req_id) else {
            return false;
        };
        if !success {
            return true;
        }
        let Some(data) = data else { return true };
        // Ignore a mismatched body (stale / rerouted response).
        if data.get("id").and_then(|v| v.as_str()) != Some(recall.message_id.as_str()) {
            return true;
        }
        let Some(content) = data.get("content").and_then(|v| v.as_str()) else {
            return true;
        };
        // Untrusted transcript text (especially sub-agents): strip control
        // sequences before rendering, matching the backfill path (#828 security).
        let content = crate::interface::ansi::sanitize_control_keep_newlines(content);
        let chat = match &recall.agent_id {
            None => Some(&mut self.master_session.chat),
            Some(child) => self
                .subagents
                .sessions
                .get_mut(child)
                .map(|session| &mut session.chat),
        };
        if let Some(chat) = chat {
            chat.recall_stub(&recall.message_id, &content);
        }
        true
    }

    pub(super) fn replace_master_chat_with_history_page(&mut self, data: &serde_json::Value) {
        self.master_session.chat.clear();
        self.master_session.history_backfilled = false;
        self.master_session.partial_backfill_len = None;
        Self::reconcile_backfill_history(&mut self.master_session, data);
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: "Session resumed".to_string(),
        });
    }
}
