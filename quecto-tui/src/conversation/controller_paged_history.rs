use super::*;

/// A demoted history stub whose full body has been requested (#1061 auto-recall).
/// Owning session: `agent_id` None = master, `Some(id)` = that sub-agent's chat.
#[derive(Debug, Clone)]
pub(crate) struct StubRecall {
    pub(super) agent_id: Option<String>,
    pub(super) message_id: String,
    pub(super) content: String,
    pub(super) offset: usize,
    pub(super) content_len: Option<usize>,
}

pub(super) const GET_MESSAGE_PAGE_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES / 4;

// Correlation and retry for in-flight older-page requests live in
// `conversation::history_paging`. Responses are applied only when their id matches
// EXACTLY: `get_messages` responses are broadcast to every connected client, so
// a prefix match would let another client's page (at a different paging depth)
// — or our own page still in flight across a resume — prepend history at the
// wrong depth, silently creating an interior gap.
impl SessionView {
    /// Whether `id` correlates to this session's own in-flight older-page request.
    pub(super) fn is_pending_history_page(&self, id: Option<&str>) -> bool {
        self.history.is_pending_page(id)
    }

    /// Unblock retry after a failed/no-data older-page response (#1061 review).
    pub(super) fn clear_pending_history_page(&mut self) {
        self.history.clear_pending_page();
    }
}

impl App {
    /// `latch_failures`: true only for a disconnected refusal — a transient
    /// enqueue failure on a live connection must stay retryable, not be
    /// permanently latched into `failed_stub_recalls` (#1470 r5).
    pub(super) fn rollback_failed_history_command(
        &mut self,
        connection: &str,
        command: &Command,
        latch_failures: bool,
    ) {
        if connection != super::MASTER_CONNECTION_ID {
            return;
        }
        match command {
            Command::GetMessages { id: Some(id), .. } => {
                self.conn.master_session.history.rollback_pending_page(id);
                self.rollback_pending_solicited_get_messages(id);
            }
            Command::GetMessage { id: Some(id), .. } => {
                // On a dead connection, latch the failure too: without it,
                // every scroll re-issues every visible stub recall and
                // floods failure reports (#1470 r4/r5).
                if let Some(recall) = self.conn.pending_stub_recall.remove(id)
                    && latch_failures
                {
                    self.conn
                        .failed_stub_recalls
                        .insert((recall.agent_id.clone(), recall.message_id.clone()));
                }
            }
            _ => {}
        }
    }

    pub(super) fn request_active_older_history_page(&mut self) {
        let Some((id, before)) = self.next_history_page_request() else {
            return;
        };
        self.send_command(Command::GetMessages {
            agent_id: None,
            id: Some(id),
            before: Some(before),
        });
    }

    pub(super) fn next_history_page_request(&mut self) -> Option<(String, String)> {
        if self.subagents.active_agent_id.is_some() {
            return None;
        }
        let ns = self.conn.id_namespace();
        let session = self.active_session_mut();
        let at_oldest = session.chat.is_at_oldest_loaded_history();
        let request = session.history.next_page_request(
            at_oldest,
            std::time::Instant::now(),
            // `get_messages` responses are broadcast to every connected client,
            // so a per-session sequence alone (`history-page-1`) is not
            // sufficient: two clients paging at different depths could accept
            // each other's response. Include a process-unique token while
            // retaining the sequence suffix for readable diagnostics — under
            // the connection namespace (#1463).
            |seq| {
                format!(
                    "{ns}history-page-{}-{}",
                    super::app_events::uuid_like(),
                    seq
                )
            },
        )?;
        Some((request.request_id, request.before))
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
        let stub_infos = self.active_session().chat.visible_stub_message_infos();
        for (message_id, content_len) in stub_infos {
            let recall_key = (agent_id.clone(), message_id.clone());
            if self.conn.failed_stub_recalls.contains(&recall_key)
                || self
                    .conn
                    .pending_stub_recall
                    .values()
                    .any(|r| r.agent_id == agent_id && r.message_id == message_id)
            {
                continue;
            }
            let req_id = format!(
                "{}stub-recall-{}",
                self.conn.id_namespace(),
                super::app_events::uuid_like()
            );
            self.conn.pending_stub_recall.insert(
                req_id.clone(),
                StubRecall {
                    agent_id: agent_id.clone(),
                    message_id: message_id.clone(),
                    content: String::new(),
                    offset: 0,
                    content_len,
                },
            );
            self.send_command(Command::GetMessage {
                id: Some(req_id),
                message_id,
                agent_id: agent_id.clone(),
                tool_call_id: None,
                offset: Some(0),
                limit: Some(GET_MESSAGE_PAGE_BYTES),
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
        let Some(recall) = self.conn.pending_stub_recall.remove(req_id) else {
            return false;
        };
        let recall_key = (recall.agent_id.clone(), recall.message_id.clone());
        if !success {
            self.conn.failed_stub_recalls.insert(recall_key);
            return true;
        }
        let Some(data) = data else {
            self.conn.failed_stub_recalls.insert(recall_key);
            return true;
        };
        // Reject a mismatched body (stale / rerouted response) and require the
        // authoritative response role to agree with the original stub metadata.
        let (response_id, role) = crate::protocol::presentation_payloads::response_identity(data);
        let response_matches = response_id.as_deref() == Some(recall.message_id.as_str());
        let chat = match &recall.agent_id {
            None => Some(&mut self.conn.master_session.chat),
            Some(child) => self
                .subagents
                .sessions
                .get_mut(child)
                .map(|session| &mut session.chat),
        };
        let Some(chat) = chat else {
            self.conn.failed_stub_recalls.insert(recall_key);
            return true;
        };
        if !response_matches
            || !role
                .as_deref()
                .is_some_and(|role| chat.stub_role_matches(&recall.message_id, role))
        {
            self.conn.failed_stub_recalls.insert(recall_key);
            return true;
        }
        let update = crate::protocol::range_accumulator::RangeAccumulator::new_with_expected_len(
            recall.content,
            recall.offset,
            recall.content_len,
        )
        .apply(data);
        let accumulated = match update {
            Ok(crate::protocol::range_accumulator::RangeUpdate::Continue {
                content,
                next_offset,
                content_len,
            }) => {
                let req_id = format!(
                    "{}stub-recall-{}",
                    self.conn.id_namespace(),
                    super::app_events::uuid_like()
                );
                self.conn.pending_stub_recall.insert(
                    req_id.clone(),
                    StubRecall {
                        agent_id: recall.agent_id.clone(),
                        message_id: recall.message_id.clone(),
                        content,
                        offset: next_offset,
                        content_len,
                    },
                );
                self.send_command(Command::GetMessage {
                    id: Some(req_id),
                    message_id: recall.message_id,
                    agent_id: recall.agent_id,
                    tool_call_id: None,
                    offset: Some(next_offset),
                    limit: Some(GET_MESSAGE_PAGE_BYTES),
                });
                return true;
            }
            Ok(crate::protocol::range_accumulator::RangeUpdate::Complete(content)) => content,
            Err(_) => {
                self.conn.failed_stub_recalls.insert(recall_key);
                return true;
            }
        };
        // Untrusted transcript text (especially sub-agents): strip control
        // sequences once after all pages are reassembled, so split ANSI/control
        // sequences are interpreted identically to the original message.
        let accumulated = crate::components::ansi::sanitize_control_keep_newlines(&accumulated);
        if !chat.recall_stub(&recall.message_id, &accumulated) {
            self.conn.failed_stub_recalls.insert(recall_key);
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
        self.conn.message_recovery_batches.clear();
        self.conn.pending_message_recovery.clear();
        self.conn.pending_stub_recall.clear();
        self.conn.failed_stub_recalls.clear();
        // Every caller is a master-conversation lifecycle boundary (new,
        // resume, rewind, or clear). Invalidate paging correlation and cursors
        // with the message refs so a late page from the prior conversation
        // cannot prepend into the replacement transcript, and the new
        // conversation cannot request the prior cursor.
        self.conn.master_session.history.reset();
        // Drop solicited resume/rewind/attach correlation too (#1237): a late
        // response from the prior boundary must not replace the new transcript.
        // Callers that mint a new id (resume success, rewind_to success) do so
        // AFTER this clear.
        self.clear_pending_solicited_get_messages();
    }

    /// Whether a `get_messages` payload carries paged-history metadata. #1061
    /// servers always attach it; a legacy payload without it falls back to the
    /// wholesale-replacement path.
    pub(super) fn is_history_page_payload(data: &serde_json::Value) -> bool {
        crate::protocol::presentation_payloads::is_history_page(data)
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
        self.conn.master_session.chat.clear();
        self.conn.master_session.history.reopen_backfill();
        // The cursors themselves are reconciled from `data` below.
        Self::reconcile_master_backfill_history(&mut self.conn.master_session, data, false);
        self.conn.master_session.chat.add_entry(ChatEntry::Status {
            text: status.to_string(),
        });
    }
}
