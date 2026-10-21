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
        match args.trim() {
            "restart" => {
                if self.send_command(Command::RestartAdmission {
                    id: Some(RESTART_ID.to_string()),
                }) {
                    self.notify("Restarting the admission broker…", NotifyLevel::Info);
                }
            }
            "" => {
                let health = self
                    .ac()
                    .master_session
                    .footer
                    .admission_health()
                    .map(str::to_string);
                match health {
                    Some(health) => self.notify(
                        &format!("{health} — {ADMISSION_USAGE} to restart the broker"),
                        NotifyLevel::Info,
                    ),
                    None => self.notify("This session has no admission broker", NotifyLevel::Info),
                }
            }
            _ => self.notify(ADMISSION_USAGE, NotifyLevel::Warning),
        }
    }

    /// Present a `restart_admission` response.
    pub(in crate::shell::app) fn handle_admission_restart(
        &mut self,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        if !success {
            let reason = error.unwrap_or_else(|| "unknown error".into());
            self.notify(
                &format!("Admission restart failed: {reason}"),
                NotifyLevel::Error,
            );
            return;
        }
        let view = crate::protocol::admission_payloads::parse_admission_restart(
            data.as_ref(),
            &crate::components::ansi::sanitize_control,
        );
        let done = match view.outcome.as_deref() {
            Some("service_restarted") => "Admission broker service restarted",
            Some("started") => "Admission broker started",
            _ => "Admission broker restarted",
        };
        match view.authority_status.as_deref() {
            Some("connected") => self.notify(
                &format!("{done}; this session reconnected"),
                NotifyLevel::Info,
            ),
            status => self.notify(
                &format!("{done}; this session is {}", status.unwrap_or("unknown")),
                NotifyLevel::Warning,
            ),
        }
    }
}

#[cfg(test)]
#[path = "app_admission_restart_tests.rs"]
mod tests;
