//! `/admission` (#2615): the broker's health on demand, and
//! `/admission restart`, which asks the master agent to restart the shared
//! admission broker and rejoin it (`restart_admission`). The TUI only sends
//! the command and presents the answer; the agent does the restart.

use super::super::*;
use crate::shell::app::app_ordinary_exit::SessionRelaunch;

/// The correlation id of a restart request.
const RESTART_ID: &str = "admission-restart";
pub(in crate::shell::app) const ADMISSION_USAGE: &str = "Usage: /admission restart";
/// How long a restart that others share waits for its confirmation.
const CONFIRM_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

/// What a session restart for a changed policy would do.
enum SessionRestartPlan {
    /// Relaunch this TUI resuming the session.
    Relaunch(SessionRelaunch),
    /// The session is not persisted: a relaunch would lose it.
    NotPersisted,
    /// This TUI does not end its agent.
    NotOwned,
    /// The session's id is not known yet.
    Unknown,
}

impl App {
    /// Offer the restart once the agent reports that only a session restart
    /// adopts the broker's changed policy (#2615), in words that say what
    /// this TUI will actually do.
    pub(in crate::shell::app) fn offer_session_restart(&mut self) {
        let offer = match self.session_restart_plan() {
            SessionRestartPlan::Relaunch(_) => {
                "The admission broker came back with a different policy; this session must restart to use it. /admission restart restarts this session and resumes this conversation."
            }
            SessionRestartPlan::NotPersisted => {
                "The admission broker came back with a different policy; this session must restart to use it, and it is not persisted (--no-persist): restart quecto-tui yourself."
            }
            SessionRestartPlan::NotOwned => {
                "The admission broker came back with a different policy; this session must restart to use it. This TUI does not end its agent: restart that agent."
            }
            SessionRestartPlan::Unknown => {
                "The admission broker came back with a different policy; this session must restart to use it: restart quecto-tui yourself."
            }
        };
        self.notify(offer, NotifyLevel::Warning);
    }

    /// What restarting this session would do.
    fn session_restart_plan(&self) -> SessionRestartPlan {
        let owns_agent = self.exit_policy.kill_owned && self.ac().child_exit_watch.is_some();
        if !owns_agent {
            return SessionRestartPlan::NotOwned;
        }
        if !self.exit_policy.session_persisted {
            return SessionRestartPlan::NotPersisted;
        }
        let Some(session_key) = self.ac().session_key.clone() else {
            return SessionRestartPlan::Unknown;
        };
        let footer = &self.ac().master_session.footer;
        let model = Some(footer.model().to_string())
            .filter(|model| !model.is_empty() && model != "unknown");
        SessionRestartPlan::Relaunch(SessionRelaunch {
            session_key,
            model,
            effort: footer.effort().map(str::to_string),
        })
    }

    /// The broker serves a different policy: restart this session (#2615),
    /// resuming its conversation on the same model and effort. A TUI that
    /// owns its agent ends it as an ordinary exit does (persisting first)
    /// and starts itself again; otherwise it says what to restart instead.
    fn restart_session_for_policy(&mut self, done: &str) {
        match self.session_restart_plan() {
            SessionRestartPlan::Relaunch(relaunch) => {
                self.notify(
                    &format!("{done}; restarting this session to use its policy (resuming this conversation)…"),
                    NotifyLevel::Info,
                );
                self.exit_policy.relaunch = Some(relaunch);
                self.request_ordinary_exit();
            }
            SessionRestartPlan::NotPersisted => self.notify(
                &format!(
                    "{done}; its policy changed, and this session is not persisted (--no-persist), so restarting it would lose this conversation: restart quecto-tui yourself to use it"
                ),
                NotifyLevel::Warning,
            ),
            SessionRestartPlan::NotOwned => self.notify(
                &format!(
                    "{done}; its policy changed, and this TUI does not end its agent: restart that agent to use it"
                ),
                NotifyLevel::Warning,
            ),
            SessionRestartPlan::Unknown => self.notify(
                &format!(
                    "{done}; its policy changed, and this session's id is not known yet: restart quecto-tui yourself to use it"
                ),
                NotifyLevel::Warning,
            ),
        }
    }

    /// `/admission [args]`: `restart` restarts the broker; nothing shows the
    /// broker's health; anything else shows the usage.
    pub(in crate::shell::app) fn handle_admission_command(&mut self, args: &str) {
        match args.trim() {
            // Already told the broker serves another policy (#2615): only
            // this session restarts; the broker is never restarted again.
            "restart" if self.ac().admission_restart_required => {
                self.restart_session_for_policy("The admission broker serves a new policy");
            }
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
