use super::*;

/// Request id for the master attach-time history backfill (#1050). Distinct from
/// resume (`resume-messages`) and rewind (`rewind-open-*` / `rewind-refresh`) so
/// `handle_response` can reconcile (prepend + guard) rather than wholesale-replace.
pub(super) const ATTACH_BACKFILL_ID: &str = "attach-backfill";

impl App {
    /// Request durable master session history after connecting (including
    /// `--socket` attach). Uses [`ATTACH_BACKFILL_ID`] so the response path
    /// reuses the same prepend + `history_backfilled` reconcile as sub-agent
    /// panes (#828 / #1050).
    pub(crate) fn request_master_attach_backfill(&mut self) {
        self.send_command(Command::GetMessages {
            id: Some(ATTACH_BACKFILL_ID.into()),
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
        match command.as_str() {
            // #1060: gated recovery for ref-based end-of-turn miss path.
            "get_message" => self.handle_get_message_recovery(id.as_deref(), success, data),
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
                let was_pending = self.model_registry.open_pending;
                self.model_registry.open_pending = false;
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
            "resume_session" if success => {
                self.clear_message_recovery();
                self.handle_resume_success(data);
            }
            "resume_session" => self.notify_response_error("Resume failed", error),
            "get_messages" if success => {
                if let Some(data) = data {
                    if id.is_some() && id == self.rewind.pending_open_id {
                        self.rewind.pending_open_id = None;
                        self.open_rewind_selector(&data);
                    } else if id.as_deref() == Some("resume-messages")
                        && data.get("messages").and_then(|v| v.as_array()).is_some()
                        && (data
                            .get("hasMoreBefore")
                            .and_then(|v| v.as_bool())
                            .is_some()
                            || data.get("before").is_some())
                    {
                        self.replace_master_chat_with_history_page(&data);
                    } else if id.as_deref() == Some(ATTACH_BACKFILL_ID)
                        || id
                            .as_deref()
                            .is_some_and(|id| id.starts_with("history-page-"))
                        || id.is_none()
                    {
                        // Attach/resume backfill, explicit older-page fetches,
                        // OR unsolicited busy-connect snapshot (id-less, see
                        // uds_snapshots): prepend + cursor reconciliation.
                        if id
                            .as_deref()
                            .is_some_and(|id| id.starts_with("history-page-"))
                        {
                            // Older pages extend the existing prefix. The
                            // partial-prefix replacement path is only for a
                            // fuller initial backfill superseding a trimmed
                            // busy-connect snapshot.
                            self.master_session.partial_backfill_len = None;
                        }
                        Self::reconcile_backfill_history(&mut self.master_session, &data);
                    } else {
                        self.replace_chat_with_messages(&data);
                    }
                }
            }
            "rewind_to" if id.is_some() && id == self.rewind.pending_apply_id && success => {
                self.rewind.pending_apply_id = None;
                self.clear_message_recovery();
                self.notify("Rewound conversation", NotifyLevel::Success);
                self.send_command(Command::GetMessages {
                    id: Some("rewind-refresh".into()),
                    before: None,
                });
            }
            "rewind_to" if id.is_some() && id == self.rewind.pending_apply_id => {
                self.rewind.pending_apply_id = None;
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
            "agent_error" => self.handle_agent_error(error),
            _ => {}
        }
    }

    /// Apply a successful master-stream `set_model` response. When a child is
    /// focused, only the master's retained footer may update — never toast or
    /// clobber the focused child's displayed model (#1085, mirrors effort).
    fn handle_set_model_success(&mut self, data: Option<serde_json::Value>) {
        if let Some(model) = data
            .as_ref()
            .and_then(|d| d.get("model"))
            .and_then(|v| v.as_str())
            .map(crate::interface::ansi::sanitize_control)
        {
            self.master_session.footer.set_model(&model);
            if self.subagents.active_agent_id.is_none() {
                self.current_model = Some(model);
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
        // Shared get_state→footer mapping (model + context-window); see #805.
        // #1067/#1085: only mirror master model/effort into the active selector
        // when the master is selected. A late master get_state must not
        // overwrite a focused child's level/vocabulary/model.
        if let Some(model) = self.master_session.footer.apply_get_state(&data) {
            if self.subagents.active_agent_id.is_none() {
                self.current_model = Some(model);
            }
        }
        if self.subagents.active_agent_id.is_none() {
            self.current_effort = data
                .get("effort")
                .and_then(|v| v.as_str())
                .map(crate::interface::ansi::sanitize_control);
            if let Some(levels) = data.get("effortLevels").and_then(|v| v.as_array()) {
                let levels: Vec<String> = levels
                    .iter()
                    .filter_map(|l| l.as_str())
                    .map(crate::interface::ansi::sanitize_control)
                    .collect();
                if !levels.is_empty() {
                    self.effort_levels = levels;
                }
            }
        }
        if data
            .get("maxContextTokens")
            .and_then(|v| v.as_u64())
            .is_some()
        {
            self.context_stats_requested = true;
        }
        // Learn the connected agent's own id from its sessionKey ("cli:<name>").
        if let Some(key) = data.get("sessionKey").and_then(|v| v.as_str()) {
            let name = key.rsplit(':').next().unwrap_or("");
            self.connected_agent_id = match name {
                "" | "default" => None,
                other => Some(crate::interface::ansi::sanitize_control(other)),
            };
        }
        if let Some(wf) = data.get("workflow") {
            self.master_session.workflow_bar = workflow_bar::parse_workflow_event(wf);
            self.sync_workflow_automation(wf);
        }
    }

    fn sync_workflow_automation(&mut self, data: &serde_json::Value) {
        let automation = data.get("automation").unwrap_or(data);
        if let Some(value) = automation.get("autoContinue").and_then(|v| v.as_bool()) {
            self.workflow_auto_continue = value;
        }
        if let Some(value) = automation.get("completionNudge").and_then(|v| v.as_bool()) {
            self.workflow_completion_nudge = value;
        }
        self.mirror_automation_to_bar();
    }

    /// Mirror the live (App-global) automation flags onto the master workflow
    /// bar so the always-visible compact line reflects the real auto-continue
    /// state instead of the hard-coded `false` from `parse_workflow_event`
    /// (#897 AC2). Call after any (re)build of `master_session.workflow_bar`.
    pub(super) fn mirror_automation_to_bar(&mut self) {
        self.master_session.workflow_bar.workflow_auto_continue = self.workflow_auto_continue;
        self.master_session.workflow_bar.workflow_completion_nudge = self.workflow_completion_nudge;
    }

    fn handle_workflow_automation(&mut self, data: Option<serde_json::Value>) {
        if let Some(data) = data {
            self.sync_workflow_automation(&data);
        }
        let auto = if self.workflow_auto_continue {
            "ON"
        } else {
            "OFF"
        };
        let nudge = if self.workflow_completion_nudge {
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
            .and_then(|d| d.get("session").and_then(|v| v.as_str()))
            .unwrap_or("session");
        self.notify(&format!("Resumed session {session}"), NotifyLevel::Success);
        self.send_command(Command::GetMessages {
            id: Some("resume-messages".into()),
            before: None,
        });
        self.send_session_stats();
        // The agent resets session-scoped state (e.g. the effort override,
        // #1067) on resume_session; re-fetch so the footer tracks it.
        self.send_state_resync();
    }

    fn handle_get_subagents(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        let Some(arr) = data.get("subagents") else {
            return;
        };
        if let Ok(infos) = <Vec<crate::infrastructure::client::SubagentInfoEvent> as serde::Deserialize>::deserialize(arr)
        {
            self.update_subagent_bar(infos);
        }
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
