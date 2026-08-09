use super::*;

fn routed_subagent_id<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = id.strip_prefix(prefix)?;
    let (len, rest) = rest.split_once(':')?;
    let len = len.parse::<usize>().ok()?;
    let agent_id = rest.get(..len)?;
    rest.as_bytes()
        .get(len)
        .is_some_and(|b| *b == b':')
        .then_some(agent_id)
}

fn routed_subagent_prefix<'a>(id: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes
        .iter()
        .find_map(|prefix| routed_subagent_id(id, prefix))
}
use crate::protocol::client::{Event, ToolCatalogueEntry};

/// Family prefix for the master attach-time history backfill (#1050 / #1237).
/// Minted ids are `attach-backfill-{uuid_like}-{seq}`; bare legacy literals are
/// dropped unless they exactly match a pending id (they never will after minting).
pub(super) const ATTACH_BACKFILL_ID_PREFIX: &str = "attach-backfill";

/// Test-facing bare attach id. Production mints unique ids; unit helpers may
/// arm [`App::test_arm_attach_backfill`] with this token so exact-pending match
/// still exercises the reconcile path without going through the socket emit.
/// BDD steps use the literal `"attach-backfill"` string directly.
#[cfg(test)]
pub(super) const ATTACH_BACKFILL_ID: &str = "attach-backfill";

/// Family prefixes for solicited master `get_messages` that must never fall
/// through to the legacy wholesale-replace arm when they are not this client's
/// exact pending id (#1237). Readable prefixes are diagnostics only.
const RESUME_MESSAGES_ID_PREFIX: &str = "resume-messages";
const REWIND_REFRESH_ID_PREFIX: &str = "rewind-refresh";

/// Kind of a matched own-client solicited transcript response (#1237).
/// Status text is derived from kind, never from literal id equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SolicitedGetMessagesKind {
    Resume,
    RewindRefresh,
    Attach,
}

impl SolicitedGetMessagesKind {
    fn id_prefix(self) -> &'static str {
        match self {
            Self::Resume => RESUME_MESSAGES_ID_PREFIX,
            Self::RewindRefresh => REWIND_REFRESH_ID_PREFIX,
            Self::Attach => ATTACH_BACKFILL_ID_PREFIX,
        }
    }

    fn replace_status(self) -> &'static str {
        match self {
            Self::Resume => "Session resumed",
            Self::RewindRefresh => "Conversation rewound",
            // Attach never wholesale-replaces; status is unused.
            Self::Attach => "",
        }
    }
}

impl App {
    /// Mint a process-unique solicited `get_messages` id and store it as the
    /// exact pending correlation token for `kind` (#1237). Overwrites any prior
    /// same-kind pending so a stale late response can no longer match.
    fn mint_pending_solicited_get_messages(&mut self, kind: SolicitedGetMessagesKind) -> String {
        self.solicited_get_messages_seq = self.solicited_get_messages_seq.wrapping_add(1);
        let id = format!(
            "{}-{}-{}",
            kind.id_prefix(),
            super::app_events::uuid_like(),
            self.solicited_get_messages_seq
        );
        match kind {
            SolicitedGetMessagesKind::Resume => {
                self.pending_resume_messages_id = Some(id.clone());
            }
            SolicitedGetMessagesKind::RewindRefresh => {
                self.pending_rewind_refresh_id = Some(id.clone());
            }
            SolicitedGetMessagesKind::Attach => {
                self.pending_attach_backfill_id = Some(id.clone());
            }
        }
        id
    }

    /// Drop all solicited resume/rewind/attach pending ids (lifecycle boundary).
    pub(super) fn clear_pending_solicited_get_messages(&mut self) {
        self.pending_resume_messages_id = None;
        self.pending_rewind_refresh_id = None;
        self.pending_attach_backfill_id = None;
    }

    /// Roll back a failed enqueue of a solicited get_messages when the id still
    /// matches the pending slot for that family.
    pub(super) fn rollback_pending_solicited_get_messages(&mut self, id: &str) {
        if self.pending_resume_messages_id.as_deref() == Some(id) {
            self.pending_resume_messages_id = None;
        }
        if self.pending_rewind_refresh_id.as_deref() == Some(id) {
            self.pending_rewind_refresh_id = None;
        }
        if self.pending_attach_backfill_id.as_deref() == Some(id) {
            self.pending_attach_backfill_id = None;
        }
    }

    fn take_pending_resume_or_rewind(
        &mut self,
        id: Option<&str>,
    ) -> Option<SolicitedGetMessagesKind> {
        let id = id?;
        if self.pending_resume_messages_id.as_deref() == Some(id) {
            self.pending_resume_messages_id = None;
            return Some(SolicitedGetMessagesKind::Resume);
        }
        if self.pending_rewind_refresh_id.as_deref() == Some(id) {
            self.pending_rewind_refresh_id = None;
            return Some(SolicitedGetMessagesKind::RewindRefresh);
        }
        None
    }

    fn is_pending_attach_backfill(&self, id: Option<&str>) -> bool {
        matches!(
            (id, self.pending_attach_backfill_id.as_deref()),
            (Some(got), Some(pending)) if got == pending
        )
    }

    /// Known solicited families that must never hit the legacy replace fallback
    /// when they are not this client's exact pending id (#1237).
    fn is_foreign_solicited_get_messages_family(id: &str) -> bool {
        fn matches_family(id: &str, prefix: &str) -> bool {
            id == prefix
                || (id.starts_with(prefix) && id.as_bytes().get(prefix.len()) == Some(&b'-'))
        }
        matches_family(id, RESUME_MESSAGES_ID_PREFIX)
            || matches_family(id, REWIND_REFRESH_ID_PREFIX)
            || matches_family(id, ATTACH_BACKFILL_ID_PREFIX)
    }

    /// Request durable master session history after connecting (including
    /// `--socket` attach). Mints a unique id so only this client reconciles the
    /// solicited reply; id-less busy-connect snapshots still reconcile (#1050).
    pub(crate) fn request_master_attach_backfill(&mut self) {
        let id = self.mint_pending_solicited_get_messages(SolicitedGetMessagesKind::Attach);
        self.send_command(Command::GetMessages {
            agent_id: None,
            id: Some(id),
            before: None,
        });
    }

    pub(super) fn handle_response(
        &mut self,
        id: Option<String>,
        command: String,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        if let Some(agent_id) = id.as_deref().and_then(|id| {
            routed_subagent_prefix(
                id,
                &[
                    "subagent-state:",
                    "subagent-sync:",
                    "subagent-tail:",
                    "subagent-message:",
                    "subagent-messages:",
                ],
            )
        }) {
            self.route_subagent_event(
                agent_id,
                Event::Response {
                    id: id.clone(),
                    command: command.clone(),
                    success,
                    data: data.clone(),
                    error: error.clone(),
                },
            );
            return;
        }
        match command.as_str() {
            "get_message" => self.handle_get_message_response(id, success, data, error),
            "get_state" if success => self.handle_get_state(data),
            "set_model" if success => self.handle_set_model_success(data),
            // Late master failure must not toast over a focused child (#1085).
            "set_model" if self.subagents.active_agent_id.is_none() => {
                self.notify_response_error("Model switch failed", error)
            }
            "set_model" => {}
            "set_effort" if success => self.handle_set_effort_success(data),
            "set_effort" => self.notify_response_error("Effort switch failed", error),
            "set_workflow_automation" if success => self.handle_workflow_automation(data),
            "set_workflow_automation" => {
                self.notify_response_error("Workflow automation update failed", error)
            }
            "get_session_stats" if success => {
                if let Some(data) = data {
                    // A quiet footer refresh (id "stats-footer") updates the
                    // cost/context indicators without adding a chat Status line.
                    if id.as_deref() == Some("stats-footer") {
                        self.update_footer_stats(&data);
                    } else {
                        self.show_session_stats(&data);
                    }
                }
            }
            "list_models" if success => self.handle_list_models(data),
            "list_models" => {
                let was_pending = self.inference.model_registry.open_pending;
                self.inference.model_registry.open_pending = false;
                self.notify_response_error("Could not list models", error);
                // Refresh failed but the user asked to open the selector: fall
                // back to whatever models we already have cached.
                if was_pending {
                    self.open_model_selector_now();
                }
            }
            "list_sessions" if success => {
                if let Some(data) = data {
                    self.open_resume_selector(&data);
                }
            }
            "list_sessions" => self.notify_response_error("Could not list sessions", error),
            "get_tool_catalogue" if success => self.handle_get_tool_catalogue(id.as_deref(), data),
            "get_tool_catalogue" => {
                if self.is_pending_tool_policy_catalogue_response(id.as_deref()) {
                    self.tool_policy_modal_pending_catalogue_id = None;
                    self.notify_response_error("Could not load tool catalogue", error);
                }
            }
            "resume_session" if success => {
                self.clear_message_recovery();
                self.handle_resume_success(data);
            }
            "resume_session" => self.notify_response_error("Resume failed", error),
            "get_messages" if success => {
                self.handle_get_messages_success(id.as_deref(), data);
            }
            // Failed page fetch: same retry-unblock as the no-data case above.
            "get_messages" if self.master_session.is_pending_history_page(id.as_deref()) => {
                self.master_session.clear_pending_history_page();
            }
            "get_messages" if self.take_pending_resume_or_rewind(id.as_deref()).is_some() => {}
            "get_messages" if self.is_pending_attach_backfill(id.as_deref()) => {
                self.pending_attach_backfill_id = None;
            }
            "rewind_to" if id.is_some() && id == self.rewind.pending_apply_id && success => {
                self.rewind.pending_apply_id = None;
                if let Some(text) = self.rewind.pending_apply_text.take() {
                    let editor_unchanged = self
                        .rewind
                        .pending_apply_editor_baseline
                        .take()
                        .is_some_and(|baseline| baseline == self.editor.text());
                    if editor_unchanged {
                        self.editor.set_text(&text);
                        self.autocomplete.update(&self.editor.text());
                        self.refresh_files_autocomplete_from_editor();
                    } else {
                        self.notify(
                            "Rewound conversation; editor kept your newer draft",
                            NotifyLevel::Info,
                        );
                    }
                }
                self.clear_message_recovery();
                self.notify("Rewound conversation", NotifyLevel::Success);
                let refresh_id = self
                    .mint_pending_solicited_get_messages(SolicitedGetMessagesKind::RewindRefresh);
                self.send_command(Command::GetMessages {
                    id: Some(refresh_id),
                    before: None,
                    agent_id: None,
                });
            }
            "rewind_to" if id.is_some() && id == self.rewind.pending_apply_id => {
                self.rewind.pending_apply_id = None;
                self.rewind.pending_apply_editor_baseline = None;
                self.rewind.pending_apply_text = None;
                self.notify_response_error("Rewind failed", error);
            }
            "rewind_to" => {}
            "clear_history" if success => {
                self.clear_message_recovery();
                // #897: history and workflow are orthogonal — clearing the
                // conversation deliberately retains the workflow engine state.
                // Signal both so the distinction is never silently entangled.
                self.notify("History cleared · workflow retained", NotifyLevel::Info);
            }
            "get_subagents" if success => self.handle_get_subagents(data),
            "delete_all_subagents" if success => {
                self.notify("Deleted all subagents", NotifyLevel::Success)
            }
            "delete_all_subagents" => {
                self.notify_response_error("Could not delete subagents", error)
            }
            "agent_error" => self.handle_agent_error(error),
            _ => {}
        }
    }

    fn handle_get_tool_catalogue(&mut self, id: Option<&str>, data: Option<serde_json::Value>) {
        if !self.is_pending_tool_policy_catalogue_response(id) {
            return;
        }
        let Some(tools_value) = data.and_then(|data| data.get("tools").cloned()) else {
            self.tool_policy_modal_pending_catalogue_id = None;
            self.replace_tool_catalogue(Vec::new());
            self.notify("Tool catalogue response missing tools", NotifyLevel::Error);
            return;
        };
        let Ok(tools) = serde_json::from_value::<Vec<ToolCatalogueEntry>>(tools_value) else {
            self.tool_policy_modal_pending_catalogue_id = None;
            self.replace_tool_catalogue(Vec::new());
            self.notify("Tool catalogue response malformed", NotifyLevel::Error);
            return;
        };
        self.replace_tool_catalogue(tools);
    }

    /// Correlate a successful master `get_messages` response (#1061 / #1237).
    /// Order is load-bearing: exact own solicited ids first, then history pages,
    /// attach/id-less reconcile, foreign-family drop, and only then legacy replace.
    fn handle_get_messages_success(&mut self, id: Option<&str>, data: Option<serde_json::Value>) {
        let own_page = self.master_session.is_pending_history_page(id);
        let Some(data) = data else {
            if own_page {
                // Success but no data: clear the in-flight request so the same
                // older page can be retried on the next scroll (#1061 review).
                self.master_session.clear_pending_history_page();
            } else if self.take_pending_resume_or_rewind(id).is_some() {
                // Exact pending matched but no payload — still clear pending
                // so a later foreign id of the same family cannot apply.
            } else if self.is_pending_attach_backfill(id) {
                self.pending_attach_backfill_id = None;
            }
            return;
        };

        if id.is_some() && id == self.rewind.pending_open_id.as_deref() {
            self.rewind.pending_open_id = None;
            self.open_rewind_selector(&data);
            return;
        }
        if let Some(kind) = self.take_pending_resume_or_rewind(id) {
            // Own resume/rewind swapped or truncated the server-side
            // conversation: replace the transcript. Status comes from
            // the matched kind, never literal id equality (#1237).
            let status = kind.replace_status();
            if Self::is_history_page_payload(&data) {
                self.replace_master_chat_with_history_page(&data, status);
            } else {
                self.clear_message_recovery();
                if self.replace_chat_with_messages_with_empty_status(&data, status) {
                    self.master_session.chat.add_entry(ChatEntry::Status {
                        text: status.to_string(),
                    });
                }
            }
            return;
        }
        if own_page {
            // This client's own older page extends the loaded prefix.
            Self::reconcile_master_backfill_history(&mut self.master_session, &data, true);
            return;
        }
        if id.is_some_and(|id| id.starts_with("history-page-")) {
            // Another client's older page (get_messages responses are broadcast
            // to every client) or one orphaned by a resume: it is paged from a
            // DIFFERENT depth, so prepending it would create an interior gap.
            return;
        }
        if self.is_pending_attach_backfill(id) || id.is_none() {
            // Exact own attach backfill OR unsolicited busy-connect snapshot
            // (id-less, see uds_snapshots). Do NOT clear attach pending on an
            // id-less snapshot — a trimmed snapshot must leave the later full
            // attach able to restore omitted history (#1050 / #1237).
            if self.is_pending_attach_backfill(id) {
                self.pending_attach_backfill_id = None;
            }
            Self::reconcile_master_backfill_history(&mut self.master_session, &data, false);
            return;
        }
        if id.is_some_and(Self::is_foreign_solicited_get_messages_family) {
            // Foreign (or stale bare-literal) resume/rewind/attach family id:
            // drop. Must not fall through to legacy replace — that is the
            // multi-client clobber (#1237).
            return;
        }
        self.replace_chat_with_messages(&data);
    }

    /// Apply a successful master-stream `set_model` response. When a child is
    /// focused, only the master's retained footer may update — never toast or
    /// clobber the focused child's displayed model (#1085, mirrors effort).
    fn handle_set_model_success(&mut self, data: Option<serde_json::Value>) {
        if let Some(model) = data.as_ref().and_then(|d| {
            crate::protocol::state_payloads::parse_set_model_id(
                d,
                &crate::components::ansi::sanitize_control,
            )
        }) {
            self.master_session.footer.set_model(&model);
            if self.subagents.active_agent_id.is_none() {
                self.inference.current_model = Some(model);
            }
        }
        if self.subagents.active_agent_id.is_none() {
            self.notify("Model switched", NotifyLevel::Success);
            // A model switch can change the provider's effort vocabulary
            // and context window — re-sync from the agent (#1067).
            self.send_state_resync();
        }
    }

    fn handle_get_state(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        let snap = crate::protocol::state_payloads::parse_get_state(
            &data,
            &crate::components::ansi::sanitize_control,
        );
        // Shared get_state→footer mapping (model + context-window); see #805.
        // #1067/#1085: only mirror master model/effort into the active selector
        // when the master is selected. A late master get_state must not
        // overwrite a focused child's level/vocabulary/model.
        if let Some(model) = self
            .master_session
            .footer
            .apply_get_state_fields(&snap.footer)
        {
            if self.subagents.active_agent_id.is_none() {
                self.inference.current_model = Some(model);
            }
        }
        if self.subagents.active_agent_id.is_none() {
            self.inference.current_effort = snap.footer.effort.clone();
            if !snap.effort_levels.is_empty() {
                self.inference.effort_levels = snap.effort_levels;
            }
        }
        if snap.footer.max_context_tokens.is_some() {
            self.sessions.context_stats_requested = true;
        }
        // Learn the connected agent's own id from its sessionKey ("cli:<name>").
        if let Some(key) = snap.session_key.as_deref() {
            let name = key.rsplit(':').next().unwrap_or("");
            self.connected_agent_id = match name {
                "" | "default" => None,
                other => Some(crate::components::ansi::sanitize_control(other)),
            };
        }
        if let Some(wf) = snap.workflow.as_ref() {
            self.master_session.workflow_bar = workflow_bar::parse_workflow_event(wf);
            self.sync_workflow_automation(wf);
        }
    }

    fn sync_workflow_automation(&mut self, data: &serde_json::Value) {
        let flags = crate::protocol::workflow_payloads::parse_workflow_automation(data);
        if let Some(value) = flags.auto_continue {
            self.workflow.auto_continue = value;
        }
        if let Some(value) = flags.completion_nudge {
            self.workflow.completion_nudge = value;
        }
        self.mirror_automation_to_bar();
    }

    /// Mirror the live (App-global) automation flags onto the master workflow
    /// bar so the always-visible compact line reflects the real auto-continue
    /// state instead of the hard-coded `false` from `parse_workflow_event`
    /// (#897 AC2). Call after any (re)build of `master_session.workflow_bar`.
    pub(super) fn mirror_automation_to_bar(&mut self) {
        self.master_session.workflow_bar.workflow_auto_continue = self.workflow.auto_continue;
        self.master_session.workflow_bar.workflow_completion_nudge = self.workflow.completion_nudge;
    }

    fn handle_workflow_automation(&mut self, data: Option<serde_json::Value>) {
        if let Some(data) = data {
            self.sync_workflow_automation(&data);
        }
        let auto = if self.workflow.auto_continue {
            "ON"
        } else {
            "OFF"
        };
        let nudge = if self.workflow.completion_nudge {
            "ON"
        } else {
            "OFF"
        };
        self.notify(
            &format!("Workflow automation: auto-continue {auto}, completion nudge {nudge}"),
            NotifyLevel::Info,
        );
    }

    fn handle_resume_success(&mut self, data: Option<serde_json::Value>) {
        let session = data
            .as_ref()
            .map(crate::protocol::state_payloads::parse_resume_session_name)
            .unwrap_or_else(|| "session".to_string());
        self.notify(&format!("Resumed session {session}"), NotifyLevel::Success);
        // clear_message_recovery already ran in the resume_session arm; mint
        // AFTER that clear so the new pending survives (#1237).
        let id = self.mint_pending_solicited_get_messages(SolicitedGetMessagesKind::Resume);
        self.send_command(Command::GetMessages {
            agent_id: None,
            id: Some(id),
            before: None,
        });
        self.send_session_stats();
        // The agent resets session-scoped state (e.g. the effort override,
        // #1067) on resume_session; re-fetch so the footer tracks it.
        self.send_state_resync();
    }

    fn handle_get_subagents(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        self.update_subagent_bar(crate::protocol::presentation_payloads::subagents(&data));
    }

    fn handle_get_message_response(
        &mut self,
        id: Option<String>,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        if id.is_some() && id == self.rewind.pending_load_id {
            if success {
                self.handle_rewind_get_message_success(data);
            } else {
                self.handle_rewind_get_message_failure(error);
            }
            return;
        }

        // #1061 auto-recall of a demoted history stub takes precedence. The
        // guard applies the stub recall as a side effect; only when this
        // response is NOT a stub recall does it fall through to #1060's gated
        // ref-based end-of-turn miss recovery (a stub recall lands in `_`).
        if !self.handle_stub_recall_response(id.as_deref(), success, data.as_ref()) {
            self.handle_get_message_recovery(id.as_deref(), success, data);
        }
    }

    fn handle_rewind_get_message_success(&mut self, data: Option<serde_json::Value>) {
        self.rewind.pending_load_id = None;
        let Some(message_id) = self.rewind.pending_apply_message_id.clone() else {
            return;
        };
        let Some(data) = data else {
            self.clear_pending_rewind_load();
            self.notify(
                "Rewind failed: selected message not found",
                NotifyLevel::Error,
            );
            return;
        };
        if crate::protocol::presentation_payloads::response_identity(&data)
            .0
            .as_deref()
            != Some(message_id.as_str())
        {
            self.clear_pending_rewind_load();
            self.notify(
                "Rewind failed: selected message not found",
                NotifyLevel::Error,
            );
            return;
        }

        let update = crate::protocol::range_accumulator::RangeAccumulator::new_with_expected_len(
            std::mem::take(&mut self.rewind.pending_load_content),
            self.rewind.pending_load_offset,
            self.rewind.pending_load_content_len,
        )
        .apply(&data);
        let text = match update {
            Ok(crate::protocol::range_accumulator::RangeUpdate::Continue {
                content,
                next_offset,
                content_len,
            }) => {
                let id = self.next_rewind_request_id("load");
                self.rewind.pending_load_id = Some(id.clone());
                self.rewind.pending_load_content = content;
                self.rewind.pending_load_offset = next_offset;
                self.rewind.pending_load_content_len = content_len;
                self.send_command(Command::GetMessage {
                    id: Some(id),
                    message_id,
                    agent_id: None,
                    tool_call_id: None,
                    offset: Some(next_offset),
                    limit: Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES),
                });
                return;
            }
            Ok(crate::protocol::range_accumulator::RangeUpdate::Complete(text)) => text,
            Err(_) => {
                self.clear_pending_rewind_load();
                self.notify(
                    "Rewind failed: selected message not found",
                    NotifyLevel::Error,
                );
                return;
            }
        };

        self.clear_pending_rewind_load();
        let id = self.next_rewind_request_id("to");
        self.rewind.pending_apply_id = Some(id.clone());
        self.rewind.pending_apply_editor_baseline = Some(self.editor.text());
        self.rewind.pending_apply_text = Some(text);
        self.send_command(Command::RewindTo {
            id: Some(id),
            message_id,
        });
    }

    fn handle_rewind_get_message_failure(&mut self, error: Option<String>) {
        self.clear_pending_rewind_load();
        self.notify_response_error("Rewind failed", error);
    }

    fn clear_pending_rewind_load(&mut self) {
        self.rewind.pending_load_id = None;
        self.rewind.pending_apply_message_id = None;
        self.rewind.pending_load_content.clear();
        self.rewind.pending_load_offset = 0;
        self.rewind.pending_load_content_len = None;
    }

    fn handle_agent_error(&mut self, error: Option<String>) {
        let msg = error.unwrap_or_else(|| "unknown error".into());
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: format!("Error: {}", msg),
        });
        self.agent_state.reset();
        self.master_session.running = false;
        self.master_session.footer.set_streaming(false);
        self.spinner = None;
    }

    fn notify_response_error(&mut self, prefix: &str, error: Option<String>) {
        let msg = error.unwrap_or_else(|| "unknown error".into());
        self.notify(&format!("{prefix}: {msg}"), NotifyLevel::Error);
    }
}

#[cfg(any(test, feature = "test-harness"))]
impl App {
    /// Arm exact-pending attach correlation for a synthetic response delivery.
    pub fn test_arm_attach_backfill(&mut self, id: &str) {
        self.pending_attach_backfill_id = Some(id.to_string());
    }

    /// Arm exact-pending resume correlation for a synthetic response delivery.
    pub fn test_arm_resume_messages(&mut self, id: &str) {
        self.pending_resume_messages_id = Some(id.to_string());
    }

    /// Arm exact-pending rewind-refresh correlation for a synthetic response delivery.
    pub fn test_arm_rewind_refresh(&mut self, id: &str) {
        self.pending_rewind_refresh_id = Some(id.to_string());
    }

    /// Pending attach id (test inspection / capture after real mint).
    pub fn test_pending_attach_backfill_id(&self) -> Option<&str> {
        self.pending_attach_backfill_id.as_deref()
    }

    /// Pending resume id (test inspection / capture after real mint).
    pub fn test_pending_resume_messages_id(&self) -> Option<&str> {
        self.pending_resume_messages_id.as_deref()
    }

    /// Pending rewind-refresh id (test inspection / capture after real mint).
    pub fn test_pending_rewind_refresh_id(&self) -> Option<&str> {
        self.pending_rewind_refresh_id.as_deref()
    }
}
