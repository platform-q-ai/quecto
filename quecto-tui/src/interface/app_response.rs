use super::*;
impl App {
    pub(super) fn handle_response(
        &mut self,
        id: Option<String>,
        command: String,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        match command.as_str() {
            "get_state" if success => self.handle_get_state(data),
            "set_model" if success => self.notify("Model switched", NotifyLevel::Success),
            "set_model" => self.notify_response_error("Model switch failed", error),
            "set_workflow_automation" if success => self.handle_workflow_automation(data),
            "set_workflow_automation" => {
                self.notify_response_error("Workflow automation update failed", error)
            }
            "get_session_stats" if success => {
                if let Some(data) = data {
                    self.show_session_stats(&data);
                }
            }
            "list_models" if success => self.handle_list_models(data),
            "list_models" => {
                let was_pending = self.model_registry.1;
                self.model_registry.1 = false;
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
            "resume_session" if success => self.handle_resume_success(data),
            "resume_session" => self.notify_response_error("Resume failed", error),
            "get_messages" if success => {
                if let Some(data) = data {
                    if id.is_some() && id == self.pending_rewind_open_id {
                        self.pending_rewind_open_id = None;
                        self.open_rewind_selector(&data);
                    } else {
                        self.replace_chat_with_messages(&data);
                    }
                }
            }
            "rewind_to" if id.is_some() && id == self.pending_rewind_apply_id && success => {
                self.pending_rewind_apply_id = None;
                self.notify("Rewound conversation", NotifyLevel::Success);
                self.send_command(Command::GetMessages {
                    id: Some("rewind-refresh".into()),
                });
            }
            "rewind_to" if id.is_some() && id == self.pending_rewind_apply_id => {
                self.pending_rewind_apply_id = None;
                self.notify_response_error("Rewind failed", error);
            }
            "rewind_to" => {}
            "clear_history" if success => {}
            "get_subagents" if success => self.handle_get_subagents(data),
            "agent_error" => self.handle_agent_error(error),
            _ => {}
        }
    }

    fn handle_get_state(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
            let sanitized: String = model.chars().filter(|c| !c.is_control()).collect();
            self.footer.set_model(&sanitized);
            self.current_model = Some(sanitized);
        }
        if let Some(max_ctx) = data.get("maxContextTokens").and_then(|v| v.as_u64()) {
            self.footer.set_context_window(max_ctx as usize);
            self.context_stats_requested = true;
        }
        // Learn the connected agent's own id from its sessionKey ("cli:<name>").
        if let Some(key) = data.get("sessionKey").and_then(|v| v.as_str()) {
            let name = key.rsplit(':').next().unwrap_or("");
            self.connected_agent_id = match name {
                "" | "default" => None,
                other => Some(other.chars().filter(|c| !c.is_control()).collect()),
            };
        }
        if let Some(wf) = data.get("workflow") {
            self.workflow_bar = workflow_bar::parse_workflow_event(wf);
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
        });
        self.send_session_stats();
    }

    fn handle_get_subagents(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        let Some(arr) = data.get("subagents") else {
            return;
        };
        if let Ok(infos) = serde_json::from_value::<
            Vec<crate::infrastructure::client::SubagentInfoEvent>,
        >(arr.clone())
        {
            self.update_subagent_bar(infos);
        }
    }

    fn handle_agent_error(&mut self, error: Option<String>) {
        let msg = error.unwrap_or_else(|| "unknown error".into());
        self.chat.add_entry(ChatEntry::Status {
            text: format!("Error: {}", msg),
        });
        self.agent_state.reset();
        self.footer.set_streaming(false);
        self.spinner = None;
    }

    fn notify_response_error(&mut self, prefix: &str, error: Option<String>) {
        let msg = error.unwrap_or_else(|| "unknown error".into());
        self.notify(&format!("{prefix}: {msg}"), NotifyLevel::Error);
    }
}
