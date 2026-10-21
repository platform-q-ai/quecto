//! `/admission` (#2615): the broker's health on demand, and
//! `/admission restart`, which asks the master agent to restart the shared
//! admission broker and rejoin it (`restart_admission`). The TUI only sends
//! the command and presents the answer; the agent does the restart.

use super::super::*;

/// The correlation id of a restart request.
const RESTART_ID: &str = "admission-restart";
pub(in crate::shell::app) const ADMISSION_USAGE: &str = "Usage: /admission restart";
/// The one-step restart offered when the broker's policy changed (#2615).
pub(in crate::shell::app) const SESSION_RESTART_OFFER: &str = "The admission broker came back with a different policy; this session must restart to use it. /admission restart restarts the broker and this session.";
/// How long a restart that others share waits for its confirmation.
const CONFIRM_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

impl App {
    /// Offer the one-step restart once the agent reports that only a
    /// session restart adopts the broker's changed policy (#2615).
    pub(in crate::shell::app) fn offer_session_restart(&mut self) {
        self.notify(SESSION_RESTART_OFFER, NotifyLevel::Warning);
    }

    /// The broker is back with a different policy: restart this session
    /// (#2615). A TUI that owns its agent ends it as an ordinary exit does
    /// and starts itself again; one attached to an agent it does not end
    /// says what to restart instead.
    fn restart_session_for_policy(&mut self, done: &str) {
        let owns_agent = self.exit_policy.kill_owned && self.ac().child_exit_watch.is_some();
        match owns_agent {
            true => {
                self.notify(
                    &format!("{done}; restarting this session to use its policy…"),
                    NotifyLevel::Info,
                );
                self.exit_policy.restart_session = true;
                self.request_ordinary_exit();
            }
            false => self.notify(
                &format!(
                    "{done}; its policy changed, and this TUI does not end its agent: restart that agent to use it"
                ),
                NotifyLevel::Warning,
            ),
        }
    }

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
                    .is_some_and(|until| now <= until);
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
                Some(tokio::time::Instant::now() + CONFIRM_WITHIN);
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
            _ => "Admission broker restarted",
        };
        if view.session_restart_required {
            self.restart_session_for_policy(done);
            return;
        }
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
