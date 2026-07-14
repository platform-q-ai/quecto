use super::*;

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

    pub(super) fn replace_master_chat_with_history_page(&mut self, data: &serde_json::Value) {
        self.master_session.chat.clear();
        self.master_session.history_backfilled = false;
        self.master_session.partial_backfill_len = None;
        Self::reconcile_backfill_history(&mut self.master_session, data);
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: "Session resumed".to_string(),
        });
    }

    #[cfg(test)]
    pub(crate) fn request_history_message_recall_for_test(&mut self, message_id: &str) {
        self.send_command(Command::GetMessage {
            id: Some(format!("history-recall-{}", super::app_events::uuid_like())),
            message_id: message_id.to_string(),
            agent_id: None,
        });
    }
}
