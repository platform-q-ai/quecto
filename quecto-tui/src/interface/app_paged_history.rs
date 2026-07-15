use super::*;

/// A demoted history stub whose full body has been requested (#1061 auto-recall).
/// Owning session: `agent_id` None = master, `Some(id)` = that sub-agent's chat.
#[derive(Debug, Clone)]
pub(crate) struct StubRecall {
    pub(super) agent_id: Option<String>,
    pub(super) message_id: String,
}

/// This session's in-flight older-page request (#1061 review). Responses are
/// applied only when their id matches `request_id` EXACTLY: `get_messages`
/// responses are broadcast to every connected client, so a prefix match would
/// let another client's page (at a different paging depth) — or our own page
/// still in flight across a resume — prepend history at the wrong depth,
/// silently creating an interior gap.
#[derive(Debug, Clone)]
pub(crate) struct PendingHistoryPage {
    pub(super) request_id: String,
    pub(super) before: String,
}

impl SessionView {
    /// Whether `id` correlates to this session's own in-flight older-page request.
    pub(super) fn is_pending_history_page(&self, id: Option<&str>) -> bool {
        id.is_some()
            && id
                == self
                    .history_pending_page
                    .as_ref()
                    .map(|p| p.request_id.as_str())
    }

    /// Unblock retry after a failed/no-data older-page response (#1061 review).
    pub(super) fn clear_pending_history_page(&mut self) {
        self.history_pending_page = None;
    }
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
                self.active_session_mut().history_pending_page = None;
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
        if !session.history_has_more_before || !session.chat.is_at_oldest_loaded_history() {
            return None;
        }
        let before = session.history_before_cursor.clone()?;
        if session
            .history_pending_page
            .as_ref()
            .is_some_and(|p| p.before == before)
        {
            return None;
        }
        session.history_page_seq = session.history_page_seq.wrapping_add(1);
        // `get_messages` responses are broadcast to every connected client, so
        // a per-session sequence alone (`history-page-1`) is not sufficient:
        // two clients paging at different depths could accept each other's
        // response. Include a process-unique token while retaining the sequence
        // suffix for readable diagnostics.
        let request_id = format!(
            "history-page-{}-{}",
            super::app_events::uuid_like(),
            session.history_page_seq
        );
        session.history_pending_page = Some(PendingHistoryPage {
            request_id: request_id.clone(),
            before: before.clone(),
        });
        Some((request_id, before, target_child))
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
        let stub_ids = self.active_session().chat.visible_stub_message_ids();
        for message_id in stub_ids {
            let recall_key = (agent_id.clone(), message_id.clone());
            if self.failed_stub_recalls.contains(&recall_key)
                || self
                    .pending_stub_recall
                    .values()
                    .any(|r| r.agent_id == agent_id && r.message_id == message_id)
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
    /// the #1060 recovery path). Failed, malformed, or mismatched responses mark
    /// that session/message pair failed so a permanent error cannot cause one
    /// new request per scroll action.
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
        let recall_key = (recall.agent_id.clone(), recall.message_id.clone());
        if !success {
            self.failed_stub_recalls.insert(recall_key);
            return true;
        }
        let Some(data) = data else {
            self.failed_stub_recalls.insert(recall_key);
            return true;
        };
        // Reject a mismatched body (stale / rerouted response) and require the
        // authoritative response role to agree with the original stub metadata.
        let response_matches =
            data.get("id").and_then(|v| v.as_str()) == Some(recall.message_id.as_str());
        let role = data.get("role").and_then(|v| v.as_str());
        let chat = match &recall.agent_id {
            None => Some(&mut self.master_session.chat),
            Some(child) => self
                .subagents
                .sessions
                .get_mut(child)
                .map(|session| &mut session.chat),
        };
        let Some(chat) = chat else {
            self.failed_stub_recalls.insert(recall_key);
            return true;
        };
        if !response_matches
            || !role.is_some_and(|role| chat.stub_role_matches(&recall.message_id, role))
        {
            self.failed_stub_recalls.insert(recall_key);
            return true;
        }
        let Some(content) = data.get("content").and_then(|v| v.as_str()) else {
            self.failed_stub_recalls.insert(recall_key);
            return true;
        };
        // Untrusted transcript text (especially sub-agents): strip control
        // sequences before rendering, matching the backfill path (#828 security).
        let content = crate::interface::ansi::sanitize_control_keep_newlines(content);
        if !chat.recall_stub(&recall.message_id, &content) {
            self.failed_stub_recalls.insert(recall_key);
        }
        true
    }

    /// Drop cross-conversation fetch state — #1060 recovery batches AND #1061
    /// stub recalls — when the server-side conversation is swapped or truncated
    /// (resume, rewind, clear history). In-flight responses would target
    /// messages that no longer exist, and stale entries must not suppress
    /// recall in the new conversation: a pending recall whose response was lost
    /// (e.g. disconnect mid-flight) would otherwise dedupe that stub forever.
    pub(super) fn clear_message_recovery(&mut self) {
        self.message_recovery_batches.clear();
        self.pending_message_recovery.clear();
        self.pending_stub_recall.clear();
        self.failed_stub_recalls.clear();
    }

    /// Whether a `get_messages` payload carries paged-history metadata. #1061
    /// servers always attach it; a legacy payload without it falls back to the
    /// wholesale-replacement path.
    pub(super) fn is_history_page_payload(data: &serde_json::Value) -> bool {
        data.get("messages").and_then(|v| v.as_array()).is_some()
            && (data
                .get("hasMoreBefore")
                .and_then(|v| v.as_bool())
                .is_some()
                || data.get("before").is_some())
    }

    /// Replace the master transcript with a fresh newest page and reconcile the
    /// paging cursors from it. Used when the server-side conversation was
    /// swapped or truncated (resume, rewind): the pre-existing cursor state
    /// refers to the OLD conversation and must not survive (#1061 review — a
    /// stale cursor after a rewind loops on "history cursor not found").
    pub(super) fn replace_master_chat_with_history_page(
        &mut self,
        data: &serde_json::Value,
        status: &str,
    ) {
        self.master_session.chat.clear();
        self.master_session.history_backfilled = false;
        self.master_session.partial_backfill_len = None;
        // The cursors themselves are reconciled from `data` below.
        Self::reconcile_backfill_history(&mut self.master_session, data);
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: status.to_string(),
        });
    }
}
