use std::io::Write;
use std::time::Duration;

use super::{App, NotifyLevel};
use crate::components::notification::Notification;
use crate::shell::process::{LeaderBudget, LeaderEnd, LeaderTermination, SETTLING_NOTICE_AFTER};

const ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);

/// Prefix of the progress notice shown while owned harnesses settle; the
/// elapsed seconds are appended and the notice is replaced in place.
pub(crate) const SETTLING_NOTICE_PREFIX: &str = "waiting for the agent to settle its subagents…";

/// What ordinary exit does with TUI-owned harnesses (#1956).
#[derive(Debug, Clone)]
pub(crate) struct OrdinaryExitPolicy {
    /// `--kill-on-exit` (default) terminates owned leaders; `--detach-on-exit` leaves them.
    pub(crate) kill_owned: bool,
    /// Test override of the leader budget (production derives it from
    /// the roster, see [`LeaderBudget::for_children`]).
    pub(crate) leader_budget_override: Option<LeaderBudget>,
    /// Every settling notice shown, in order (test seam).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) settling_notices_shown: Vec<String>,
    /// Every post-cleanup finalization error this instance emitted to
    /// stderr, in order (test seam). Kept on the instance rather than in a
    /// process-global sink so concurrently running tests cannot drain or
    /// interleave one another's emissions.
    #[cfg(test)]
    finalization_errors_emitted: Vec<String>,
}

impl Default for OrdinaryExitPolicy {
    fn default() -> Self {
        Self {
            kill_owned: true,
            leader_budget_override: None,
            #[cfg(any(test, feature = "test-harness"))]
            settling_notices_shown: Vec::new(),
            #[cfg(test)]
            finalization_errors_emitted: Vec::new(),
        }
    }
}

impl App {
    pub(crate) fn request_ordinary_exit(&mut self) {
        self.should_exit = true;
    }

    pub(crate) fn set_ordinary_exit_kill_owned(&mut self, kill_owned: bool) {
        self.exit_policy.kill_owned = kill_owned;
    }

    pub(crate) async fn finalize_ordinary_exit(&mut self) -> Vec<String> {
        self.request_ordinary_exit();
        let mut errors = Vec::new();
        match self.enqueue_ordinary_exit_snapshot_persist() {
            Ok(id) => errors.extend(self.await_ordinary_exit_durability_barrier(id).await),
            Err(err) => {
                let msg = format!("ordinary-exit persistence enqueue failed: {err}");
                self.notify(&msg, NotifyLevel::Error);
                errors.push(msg);
            }
        }
        errors.extend(self.settle_owned_harnesses().await);
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
        self.emit_ordinary_exit_finalization_errors(&errors);
        errors
    }

    /// Override the per-leader budget (tests: short waits so a
    /// SIGTERM-ignoring fake is SIGKILLed without waiting minutes).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn set_leader_budget(&mut self, budget: LeaderBudget) {
        self.exit_policy.leader_budget_override = Some(budget);
    }

    /// The budget for the owned leader: the fleet-derived wait for the
    /// direct children the roster last showed, unless a test overrides it.
    fn leader_budget(&self, children: usize) -> LeaderBudget {
        self.exit_policy
            .leader_budget_override
            .unwrap_or_else(|| LeaderBudget::for_children(children))
    }

    /// Ordinary exit with the kill-on-exit policy: SIGTERM the TUI-owned
    /// harness leader (one pid, never a group or a descendant), wait
    /// for it to exit within the budget while showing a settling
    /// notice after ~1 s, SIGKILL only a leader that outlives the budget,
    /// and report what the post-exit canary found (#1956).
    async fn settle_owned_harnesses(&mut self) -> Vec<String> {
        if !self.exit_policy.kill_owned {
            return Vec::new();
        }
        let Some((watch, children)) = self.take_child_exit_watch_with_roster() else {
            return Vec::new();
        };
        let budget = self.leader_budget(children);
        let longest = budget.total();
        let termination = watch.terminate_with_budget(budget);
        tokio::pin!(termination);
        let started = tokio::time::Instant::now();
        let mut next_notice = started + SETTLING_NOTICE_AFTER;
        let outcome = loop {
            tokio::select! {
                outcome = &mut termination => break outcome,
                _ = tokio::time::sleep_until(next_notice) => {
                    self.show_settling_notice(started.elapsed(), longest);
                    self.render();
                    next_notice += Duration::from_secs(1);
                }
            }
        };
        self.notifications.dismiss_prefixed(SETTLING_NOTICE_PREFIX);
        outcome
            .and_then(|outcome| Self::describe_leader_termination(&outcome, budget))
            .into_iter()
            .collect()
    }

    fn show_settling_notice(&mut self, elapsed: Duration, longest: Duration) {
        let message = format!("{SETTLING_NOTICE_PREFIX} ({}s)", elapsed.as_secs());
        #[cfg(any(test, feature = "test-harness"))]
        self.exit_policy
            .settling_notices_shown
            .push(message.clone());
        self.notifications.replace_prefixed(
            SETTLING_NOTICE_PREFIX,
            Notification::new(&message, NotifyLevel::Info)
                .with_duration(longest.saturating_add(Duration::from_secs(5))),
        );
    }

    /// Post-cleanup stderr line for an outcome worth reporting: a leader
    /// that had to be SIGKILLed, or strays the canary found (not signalled).
    pub(crate) fn describe_leader_termination(
        outcome: &LeaderTermination,
        budget: LeaderBudget,
    ) -> Option<String> {
        let pid = outcome.pid?;
        let mut notes = Vec::new();
        if outcome.end == LeaderEnd::Killed {
            notes.push(format!(
                "harness pid {pid} did not exit within {} of SIGTERM nor {} of a repeated SIGTERM; sent SIGKILL to that process only",
                format_budget(budget.settle),
                format_budget(budget.force)
            ));
        }
        if !outcome.strays.is_empty() {
            notes.push(format!(
                "post-exit canary: pids {:?} still name harness pid {pid} as parent or process group; not signalled",
                outcome.strays
            ));
        }
        (!notes.is_empty()).then(|| notes.join("; "))
    }

    /// Write the finalization errors to the real stderr after the terminal
    /// has been restored, recording them on this instance for tests.
    fn emit_ordinary_exit_finalization_errors(&mut self, errors: &[String]) {
        let mut stderr = std::io::stderr().lock();
        Self::emit_ordinary_exit_finalization_errors_to(errors, &mut stderr);
        #[cfg(test)]
        self.exit_policy
            .finalization_errors_emitted
            .extend(errors.iter().cloned());
    }

    /// Drain the finalization errors this instance has emitted so far.
    #[cfg(test)]
    pub(crate) fn take_ordinary_exit_finalization_errors_for_tests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.exit_policy.finalization_errors_emitted)
    }

    pub(crate) fn emit_ordinary_exit_finalization_errors_to(
        errors: &[String],
        stderr: &mut impl Write,
    ) {
        if errors.is_empty() {
            return;
        }
        for error in errors {
            let _ = writeln!(stderr, "quecto: ordinary-exit finalization error: {error}");
        }
        let _ = stderr.flush();
    }

    /// Enqueue the ordinary-exit roster/session persist on the connection
    /// and return its request id for the durability barrier. An owned agent
    /// about to be killed is recorded as stopped; a detached or external one
    /// keeps its live recovery.
    pub(crate) fn enqueue_ordinary_exit_snapshot_persist(
        &mut self,
    ) -> Result<String, crate::protocol::client::ClientError> {
        let id = format!("persist-exit-{}", super::app_events::uuid_like());
        let stops_owned_agent = self.exit_policy.kill_owned && self.ac().child_exit_watch.is_some();
        self.ac()
            .transport
            .clone_sender()
            .try_send_exit_durability(&crate::protocol::client::Command::PersistSession {
                id: Some(id.clone()),
                restore_reason: stops_owned_agent.then(|| "ordinary_tui_exit_stopped".to_string()),
            })?;
        Ok(id)
    }

    /// Detach the owned agent's `ChildWatch` so ordinary-exit cleanup can
    /// terminate it, paired with the number of subagents the roster last
    /// showed — the input to the fleet-derived leader budget (#1956).
    pub(crate) fn take_child_exit_watch_with_roster(
        &mut self,
    ) -> Option<(crate::shell::child_watch::ChildWatch, usize)> {
        let watch = self.ac_mut().child_exit_watch.take()?;
        Some((watch, self.ac().roster.tracked.len()))
    }

    /// Wait (bounded) for the agent's answer to the exit persist `id`. A
    /// master event channel that closes first still waits out the deadline
    /// and reports the timeout — the same observable exit as an agent that
    /// never answers.
    async fn await_ordinary_exit_durability_barrier(&mut self, id: String) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT;
        loop {
            let recv = tokio::time::timeout_at(deadline, self.master_event_rx.recv()).await;
            let sourced_event = match recv {
                Ok(Some(event)) => event,
                Ok(None) | Err(_) => {
                    tokio::time::sleep_until(deadline).await;
                    let msg = "ordinary-exit persistence barrier timed out".to_string();
                    self.notify(&msg, NotifyLevel::Error);
                    return vec![msg];
                }
            };
            let crate::shell::connection::SourcedEvent::Master(event) = sourced_event else {
                continue;
            };
            let Some((Some(answered), success, error)) =
                crate::protocol::event_barrier::persist_session_response(event)
            else {
                continue;
            };
            if answered != id {
                continue;
            }
            if success {
                return Vec::new();
            }
            let msg = error
                .unwrap_or_else(|| "failed to persist session before ordinary exit".to_string());
            self.notify(&msg, NotifyLevel::Error);
            return vec![msg];
        }
    }
}

/// `30s` for whole seconds, `300ms` otherwise.
fn format_budget(budget: Duration) -> String {
    if budget.subsec_millis() == 0 {
        format!("{}s", budget.as_secs())
    } else {
        format!("{}ms", budget.as_millis())
    }
}

#[cfg(test)]
#[path = "app_ordinary_exit_leader_tests.rs"]
mod leader_tests;
#[cfg(test)]
#[path = "app_ordinary_exit_tests.rs"]
mod tests;
