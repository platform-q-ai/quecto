//! The startup model's warnings (#2435, #2126): the harness reports them in
//! `get_state`, since the TUI never shows the agent's stderr. Each is shown
//! once, as a warning notice and a transcript status line.

use super::*;

impl App {
    /// Show the startup warnings not shown yet.
    pub(in crate::shell) fn show_startup_warnings(&mut self, warnings: Vec<String>) {
        for warning in warnings {
            if self.shown_startup_warnings.insert(warning.clone()) {
                self.notify(&warning, NotifyLevel::Warning);
                self.ac_mut()
                    .master_session
                    .chat
                    .add_entry(crate::components::chat::ChatEntry::Status { text: warning });
            }
        }
    }
}

#[cfg(test)]
#[path = "app_startup_warnings_tests.rs"]
mod tests;
