//! The `set_model` response handler.
//!
//! Kept beside `app_response.rs` rather than inside it so that file stays within
//! the module size gate.

use super::App;
use crate::components::notification::NotifyLevel;

impl App {
    /// Apply a successful master-stream `set_model` response. When a child is
    /// focused, only the master's retained footer may update — never toast or
    /// clobber the focused child's displayed model (#1085, mirrors effort).
    pub(in crate::shell::app) fn handle_set_model_success(
        &mut self,
        data: Option<serde_json::Value>,
    ) {
        let (model, unavailable) = data
            .as_ref()
            .map(|d| {
                crate::protocol::state_payloads::parse_set_model(
                    d,
                    &crate::components::ansi::sanitize_control,
                )
            })
            .unwrap_or((None, None));
        if let Some(model) = model {
            self.ac_mut().master_session.footer.set_model(&model);
            if self.ac().roster.active_agent_id.is_none() {
                self.ac_mut().inference.current_model = Some(model);
            }
        }
        // The agent records the switch even when the model cannot currently run,
        // and says why. Surfacing that here is the difference between "switched"
        // and a turn that fails for reasons the user was never told.
        if self.ac().roster.active_agent_id.is_none() {
            match unavailable {
                Some(reason) => self.notify(
                    &format!("Model switched, but {reason}"),
                    NotifyLevel::Warning,
                ),
                None => self.notify("Model switched", NotifyLevel::Success),
            }
            // A model switch can change the provider's effort vocabulary
            // and context window — re-sync from the agent (#1067).
            self.send_state_resync();
        }
    }
}
