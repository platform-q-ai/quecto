//! `/admission` (#2615): the broker's health on demand, and
//! `/admission restart`, which asks the master agent to restart the shared
//! admission broker and rejoin it (`restart_admission`). The TUI only sends
//! the command and presents the answer; the agent does the restart.

use super::super::*;

/// The correlation id of a restart request.
const RESTART_ID: &str = "admission-restart";
pub(in crate::shell::app) const ADMISSION_USAGE: &str = "Usage: /admission restart";
/// How long a restart that others share waits for its confirmation.
const CONFIRM_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

impl App {
    /// `/admission [args]`: `restart` restarts the broker; nothing shows the
    /// broker's health; anything else shows the usage.
    pub(in crate::shell::app) fn handle_admission_command(&mut self, args: &str) {
        match args.trim() {
            "restart" => {
                let now = tokio::time::Instant::now();
                let confirm = self
                    .ac_mut()
                    .admission_restart_confirm_until
                    .take()
                    .filter(|(until, _)| now <= *until)
                    .map(|(_, others)| others);
                if self.send_command(Command::RestartAdmission {
                    id: Some(RESTART_ID.to_string()),
                    confirm,
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
        if view.outcome.as_deref() == Some("unconfirmed") {
            let others = view.shared_with.unwrap_or(0);
            self.ac_mut().admission_restart_confirm_until =
                Some((tokio::time::Instant::now() + CONFIRM_WITHIN, others));
            self.notify(
                &format!(
                    "The admission broker is shared with {others} other session(s) or sub-agent(s). Restarting it fails their sub-agents (respawn them) and their in-flight requests once. Run /admission restart again within a minute to confirm."
                ),
                NotifyLevel::Warning,
            );
            return;
        }
        let done = match view.outcome.as_deref() {
            Some("service_restarted") => "Admission broker service restarted",
            Some("started") => "Admission broker started",
            Some("restarted") => "Admission broker restarted",
            other => {
                self.notify(
                    &format!(
                        "The admission broker answered the restart with an unknown outcome ({}); check /admission",
                        other.unwrap_or("none")
                    ),
                    NotifyLevel::Warning,
                );
                return;
            }
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
