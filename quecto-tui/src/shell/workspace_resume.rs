//! Session-only `/resume`.
//!
//! `/resume` lists persisted sessions directly; selecting a session or using
//! `/resume <key>` resumes it in the current single session.

use crate::components::select_list::SelectItem;

/// Value prefix for bare session rows (optional; bare keys still accepted).
pub(crate) const SESSION_RESUME_PREFIX: &str = crate::sessions::resume_rows::SESSION_ROW_PREFIX;

impl super::App {
    /// Open the resume selector with agent-listed sessions only.
    pub(super) fn open_resume_selector_with_items(
        &mut self,
        session_items: Vec<SelectItem>,
        empty_status: Option<&str>,
    ) {
        let items = session_items;
        if items.is_empty() {
            self.ac_mut().master_session.chat.add_entry(
                crate::components::chat::ChatEntry::Status {
                    text: empty_status
                        .unwrap_or("No persisted sessions found.")
                        .to_string(),
                },
            );
        }
        let scope = self.ac().sessions.scope;
        if let Some(picker) = self.ac_mut().sessions.resume_selector.as_mut() {
            // An empty listing says why under `Sessions`, never "No items".
            picker.set_notice(
                empty_status
                    .filter(|_| items.is_empty())
                    .map(str::to_string),
            );
            picker.sync_items(items);
        } else {
            self.ac_mut().sessions.resume_selector = Some(
                crate::sessions::resume_picker::ResumePicker::new(items, scope)
                    .with_clock(self.clock.clone()),
            );
        }
    }

    /// Dispatch a resume-selector choice into the current session.
    pub(super) fn apply_resume_selection(&mut self, raw: &str) {
        let session = raw
            .strip_prefix(SESSION_RESUME_PREFIX)
            .unwrap_or(raw)
            .trim();
        if session.is_empty() {
            self.send_list_sessions();
        } else {
            // Latch when the connection is disconnected (AC5).
            self.queue_or_send_session_resume(session);
        }
    }

    /// Queue a session resume for the active tab; apply immediately if connected.
    pub(crate) fn queue_or_send_session_resume(&mut self, session: &str) {
        let session = session.trim();
        if session.is_empty() {
            return;
        }
        if self.ac().agent_connected {
            self.ac_mut().pending_session_resume = Some(session.to_string());
            self.send_resume_session(session);
        } else {
            self.ac_mut().pending_session_resume = Some(session.to_string());
        }
    }
}

#[cfg(test)]
#[path = "workspace_resume_tests.rs"]
mod workspace_resume_tests;
