//! `/effort` runtime reasoning-effort control (#1067): provider-scoped
//! vocabulary, direct set, selector overlay, and response handling.
//!
//! Mirrors the `/model` + `set_model` pattern: bare `/effort` opens a
//! selector limited to the ACTIVE model's provider vocabulary; `/effort
//! <level>` validates locally (so an invalid level never leaves the TUI and
//! the previous setting stays) and sends `set_effort` over the socket. The
//! footer only updates on a successful response.
//!
//! The vocabulary is NOT derived locally: the agent reports it in
//! `get_state` (`effortLevels`) so the provider→levels rule lives in one
//! place. Before the first `get_state` lands, `/effort <level>` is sent
//! through and the agent's own validation rejects an invalid level.

use super::*;

impl App {
    /// Handle `/effort` (bare → selector) and `/effort <level>` (direct set).
    pub(super) fn handle_effort_command(&mut self, arg: &str) {
        if arg.is_empty() {
            self.open_effort_selector();
            return;
        }
        // Local pre-validation against the agent-reported vocabulary; when it
        // hasn't arrived yet, defer to the agent's own validation (it rejects
        // invalid levels listing the valid ones).
        if self.effort_levels.is_empty() || self.effort_levels.iter().any(|l| l == arg) {
            self.send_set_effort(arg);
        } else {
            self.notify(
                &format!(
                    "Invalid effort level \"{arg}\" — valid levels: {}",
                    self.effort_levels.join(", ")
                ),
                NotifyLevel::Error,
            );
        }
    }

    pub(super) fn open_effort_selector(&mut self) {
        if self.effort_levels.is_empty() {
            self.notify(
                "Effort levels not known yet — still waiting for agent state",
                NotifyLevel::Warning,
            );
            return;
        }
        let levels: Vec<&str> = self.effort_levels.iter().map(String::as_str).collect();
        self.effort_selector = Some(EffortSelector::new(&levels, self.current_effort.as_deref()));
    }

    pub(super) fn handle_effort_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.effort_selector {
            selector.handle_input(key);
            match selector.take_result() {
                EffortSelectorResult::Selected(level) => {
                    self.effort_selector = None;
                    self.send_set_effort(&level);
                }
                EffortSelectorResult::Dismissed => {
                    self.effort_selector = None;
                }
                EffortSelectorResult::Pending => {}
            }
        }
    }

    /// Send `set_effort` over the socket. The footer is NOT updated here —
    /// only a successful response switches it, so a rejected or failed
    /// switch visibly keeps the previous level.
    pub(super) fn send_set_effort(&mut self, effort: &str) {
        let cmd = Command::SetEffort {
            id: Some("se".into()),
            effort: effort.to_string(),
        };
        if self.subagents.active_agent_id.is_some() {
            if !self.send_to_active_subagent(cmd) {
                self.notify(
                    "Selected sub-agent is not ready for effort changes yet",
                    NotifyLevel::Error,
                );
            }
        } else {
            self.send_command(cmd);
        }
    }

    /// Apply a successful `set_effort` response: the agent echoes the level
    /// it actually applied in `data.effort`.
    pub(super) fn handle_set_effort_success(&mut self, data: Option<serde_json::Value>) {
        let Some(level) = data
            .as_ref()
            .and_then(|d| d.get("effort"))
            .and_then(|v| v.as_str())
            .map(crate::interface::ansi::sanitize_control)
        else {
            return;
        };
        // Master responses can arrive after focus moved to a child. Preserve the
        // master's footer, but do not replace the focused child's selector state
        // or toast the master's level as if it were the child's (mirrors the
        // active-only notify on the sub-agent stream side).
        self.master_session.footer.set_effort(Some(level.clone()));
        if self.subagents.active_agent_id.is_none() {
            self.notify(&format!("Effort set to {level}"), NotifyLevel::Success);
            self.current_effort = Some(level);
        }
    }

    /// Re-fetch agent state after a session/model switch so session-scoped
    /// display state (effort level + vocabulary, model, context window)
    /// never goes stale (#1067).
    pub(super) fn send_state_resync(&mut self) {
        self.send_command(Command::GetState {
            id: Some("resync".into()),
        });
    }
}
