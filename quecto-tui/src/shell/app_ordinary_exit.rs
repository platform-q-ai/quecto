use std::collections::HashSet;
use std::io::Write;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::{App, NotifyLevel};

const ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const ORDINARY_EXIT_OWNED_CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const ORDINARY_EXIT_OWNED_CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);

#[cfg(test)]
fn ordinary_exit_finalization_errors_for_tests() -> &'static Mutex<Vec<String>> {
    static ERRORS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    ERRORS.get_or_init(|| Mutex::new(Vec::new()))
}

impl App {
    pub(crate) fn request_ordinary_exit(&mut self) {
        self.should_exit = true;
    }

    pub(crate) fn set_ordinary_exit_kill_owned(&mut self, kill_owned: bool) {
        self.ordinary_exit_kill_owned = kill_owned;
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
        errors.extend(self.apply_ordinary_exit_owned_child_policy().await);
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
        Self::emit_ordinary_exit_finalization_errors(&errors);
        errors
    }

    async fn apply_ordinary_exit_owned_child_policy(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if !self.ordinary_exit_kill_owned {
            return errors;
        }
        for watch in self.take_all_child_exit_watches() {
            if watch
                .terminate_with_timeout(ORDINARY_EXIT_OWNED_CHILD_CLEANUP_TIMEOUT)
                .await
            {
                continue;
            }
            let msg = match watch.pid() {
                Some(pid) => format!("ordinary-exit owned-child cleanup timed out for pid {pid}"),
                None => "ordinary-exit owned-child cleanup timed out".to_string(),
            };
            self.notify(&msg, NotifyLevel::Error);
            errors.push(msg);
        }
        errors
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

#[cfg(test)]
#[path = "app_ordinary_exit_tests.rs"]
mod tests;
