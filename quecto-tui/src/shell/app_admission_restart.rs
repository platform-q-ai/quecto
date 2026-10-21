//! `/admission` (#2615): the broker's health on demand, and
//! `/admission restart`, which asks the master agent to restart the shared
//! admission broker and rejoin it (`restart_admission`). The TUI only sends
//! the command and presents the answer; the agent does the restart.

use super::super::*;

/// The correlation id of a restart request.
const RESTART_ID: &str = "admission-restart";
pub(in crate::shell::app) const ADMISSION_USAGE: &str = "Usage: /admission restart";

impl App {
    /// `/admission [args]`: `restart` restarts the broker; nothing shows the
    /// broker's health; anything else shows the usage.
    pub(in crate::shell::app) fn handle_admission_command(&mut self, args: &str) {
        let _ = (args, RESTART_ID, ADMISSION_USAGE);
    }

    /// Present a `restart_admission` response.
    pub(in crate::shell::app) fn handle_admission_restart(
        &mut self,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        let _ = (success, data, error);
    }
}

#[cfg(test)]
#[path = "app_admission_restart_tests.rs"]
mod tests;
