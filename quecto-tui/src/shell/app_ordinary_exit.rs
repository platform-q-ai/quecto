use std::collections::HashSet;
use std::io::Write;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::{App, NotifyLevel};
use crate::components::notification::Notification;
use crate::shell::process::{LeaderEnd, LeaderTermination, SETTLING_NOTICE_AFTER};

const ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);

/// Prefix of the progress notice shown while owned harnesses settle; the
/// elapsed seconds are appended and the notice is replaced in place.
pub(crate) const SETTLING_NOTICE_PREFIX: &str = "waiting for the agent to settle its subagents…";

#[cfg(test)]
fn ordinary_exit_finalization_errors_for_tests() -> &'static Mutex<Vec<String>> {
    static ERRORS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    ERRORS.get_or_init(|| Mutex::new(Vec::new()))
}

/// What ordinary exit does with TUI-owned harnesses (#1956).
#[derive(Debug, Clone)]
pub(crate) struct OrdinaryExitPolicy {
    /// `--kill-on-exit` (default) terminates owned leaders; `--detach-on-exit` leaves them.
    pub(crate) kill_owned: bool,
    /// Per-leader SIGTERM-to-SIGKILL wait; injectable for tests.
    pub(crate) leader_exit_budget: Duration,
    /// Every settling notice shown, in order (test seam).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) settling_notices_shown: Vec<String>,
}

impl Default for OrdinaryExitPolicy {
    fn default() -> Self {
        Self {
            kill_owned: true,
            leader_exit_budget: crate::shell::process::LEADER_EXIT_BUDGET,
            #[cfg(any(test, feature = "test-harness"))]
            settling_notices_shown: Vec::new(),
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
        let ids = match self.enqueue_ordinary_exit_snapshot_persists() {
            Ok(ids) => ids,
            Err((ids, err)) => {
                let msg = format!("ordinary-exit persistence enqueue failed: {err}");
                self.notify(&msg, NotifyLevel::Error);
                errors.push(msg);
                ids
            }
        };
        errors.extend(self.await_ordinary_exit_durability_barrier(ids).await);
        errors.extend(self.settle_owned_harnesses().await);
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
        Self::emit_ordinary_exit_finalization_errors(&errors);
        errors
    }

    /// Override the per-leader exit budget (tests: a short budget so a
    /// SIGTERM-ignoring fake is SIGKILLed without waiting 30 s).
    #[cfg(any(test, feature = "test-harness"))]
    pub(crate) fn set_leader_exit_budget(&mut self, budget: Duration) {
        self.exit_policy.leader_exit_budget = budget;
    }

    /// Ordinary exit with the kill-on-exit policy: SIGTERM every TUI-owned
    /// harness leader (one pid each, never a group or a descendant), wait
    /// for the leaders to exit within the budget while showing a settling
    /// notice after ~1 s, SIGKILL only a leader that outlives the budget,
    /// and report what the post-exit canary found (#1956).
    async fn settle_owned_harnesses(&mut self) -> Vec<String> {
        if !self.exit_policy.kill_owned {
            return Vec::new();
        }
        let watches = self.take_all_child_exit_watches();
        if watches.is_empty() {
            return Vec::new();
        }
        let budget = self.exit_policy.leader_exit_budget;
        let mut pending = tokio::task::JoinSet::new();
        for watch in watches {
            pending.spawn(async move { watch.terminate_with_budget(budget).await });
        }
        let started = tokio::time::Instant::now();
        let mut next_notice = started + SETTLING_NOTICE_AFTER;
        let mut outcomes = Vec::new();
        while !pending.is_empty() {
            tokio::select! {
                joined = pending.join_next() => {
                    if let Some(Ok(Some(outcome))) = joined {
                        outcomes.push(outcome);
                    }
                }
                _ = tokio::time::sleep_until(next_notice) => {
                    self.show_settling_notice(started.elapsed(), budget);
                    self.render();
                    next_notice += Duration::from_secs(1);
                }
            }
        }
        self.notifications.dismiss_prefixed(SETTLING_NOTICE_PREFIX);
        outcomes
            .iter()
            .filter_map(|outcome| Self::describe_leader_termination(outcome, budget))
            .collect()
    }

    fn show_settling_notice(&mut self, elapsed: Duration, budget: Duration) {
        let message = format!("{SETTLING_NOTICE_PREFIX} ({}s)", elapsed.as_secs());
        #[cfg(any(test, feature = "test-harness"))]
        self.exit_policy
            .settling_notices_shown
            .push(message.clone());
        self.notifications.replace_prefixed(
            SETTLING_NOTICE_PREFIX,
            Notification::new(&message, NotifyLevel::Info)
                .with_duration(budget.saturating_add(Duration::from_secs(5))),
        );
    }

    /// Post-cleanup stderr line for an outcome worth reporting: a leader
    /// that had to be SIGKILLed, or strays the canary found (not signalled).
    pub(crate) fn describe_leader_termination(
        outcome: &LeaderTermination,
        budget: Duration,
    ) -> Option<String> {
        let pid = outcome.pid?;
        let mut notes = Vec::new();
        if outcome.end == LeaderEnd::Killed {
            notes.push(format!(
                "harness pid {pid} did not exit within {} of SIGTERM; sent SIGKILL to that process only",
                format_budget(budget)
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

    pub(crate) fn emit_ordinary_exit_finalization_errors(errors: &[String]) {
        let mut stderr = std::io::stderr().lock();
        Self::emit_ordinary_exit_finalization_errors_to(errors, &mut stderr);
        #[cfg(test)]
        Self::record_ordinary_exit_finalization_errors_for_tests(errors);
    }

    #[cfg(test)]
    pub(crate) fn take_ordinary_exit_finalization_errors_for_tests() -> Vec<String> {
        ordinary_exit_finalization_errors_for_tests()
            .lock()
            .unwrap()
            .drain(..)
            .collect()
    }

    #[cfg(test)]
    fn record_ordinary_exit_finalization_errors_for_tests(errors: &[String]) {
        ordinary_exit_finalization_errors_for_tests()
            .lock()
            .unwrap()
            .extend(errors.iter().cloned());
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

    async fn await_ordinary_exit_durability_barrier(&mut self, ids: Vec<String>) -> Vec<String> {
        let mut errors = Vec::new();
        let mut pending: HashSet<String> = ids.into_iter().collect();
        let deadline = tokio::time::Instant::now() + ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT;
        while !pending.is_empty() {
            let recv = tokio::time::timeout_at(deadline, self.tab_event_rx.recv()).await;
            let sourced_event = match recv {
                Ok(Some(event)) => event,
                Ok(None) | Err(_) => {
                    let msg = "ordinary-exit persistence barrier timed out".to_string();
                    self.notify(&msg, NotifyLevel::Error);
                    errors.push(msg);
                    return errors;
                }
            };
            let crate::shell::connection::SourcedEvent::Tab(tab, event) = sourced_event else {
                continue;
            };
            let Some((id, success, error)) =
                crate::protocol::event_barrier::persist_session_response(event)
            else {
                continue;
            };
            let Some(id) = id else { continue };
            if !pending.remove(&id) {
                continue;
            }
            if success {
                continue;
            }
            let msg = error
                .unwrap_or_else(|| "failed to persist session before ordinary exit".to_string());
            let msg = format!("tab {}: {msg}", tab.0);
            self.notify(&msg, NotifyLevel::Error);
            errors.push(msg);
        }
        errors
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
#[path = "app_ordinary_exit_tests.rs"]
mod tests;
