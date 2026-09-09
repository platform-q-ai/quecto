//! Workflow automation flag mirroring (split from `app_response.rs` for the
//! 750-line cap).

use super::super::*;

impl App {
    pub(in crate::shell::app) fn sync_workflow_automation(&mut self, data: &serde_json::Value) {
        let flags = crate::protocol::workflow_payloads::parse_workflow_automation(data);
        if let Some(value) = flags.auto_continue {
            self.ac_mut().workflow.auto_continue = value;
        }
        if let Some(value) = flags.completion_nudge {
            self.ac_mut().workflow.completion_nudge = value;
        }
        self.mirror_automation_to_bar();
    }

    /// Mirror the live (App-global) automation flags onto the master workflow
    /// bar so the always-visible compact line reflects the real auto-continue
    /// state instead of the hard-coded `false` from `parse_workflow_event`
    /// (#897 AC2). Call after any (re)build of `master_session.workflow_bar`.
    pub(in crate::shell::app) fn mirror_automation_to_bar(&mut self) {
        self.ac_mut()
            .master_session
            .workflow_bar
            .workflow_auto_continue = self.ac().workflow.auto_continue;
        self.ac_mut()
            .master_session
            .workflow_bar
            .workflow_completion_nudge = self.ac().workflow.completion_nudge;
    }
}
