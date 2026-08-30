use std::collections::HashSet;
use std::time::Duration;

use super::{App, NotifyLevel};

const ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);

impl App {
    pub(crate) fn request_ordinary_exit(&mut self) {
        self.should_exit = true;
    }

    pub(crate) fn set_ordinary_exit_kill_owned(&mut self, kill_owned: bool) {
        self.ordinary_exit_kill_owned = kill_owned;
    }

    pub(crate) async fn finalize_ordinary_exit(&mut self) {
        self.request_ordinary_exit();
        let ids = match self.enqueue_ordinary_exit_snapshot_persists() {
            Ok(ids) => ids,
            Err((ids, err)) => {
                self.notify(
                    &format!("ordinary-exit persistence enqueue failed: {err}"),
                    NotifyLevel::Error,
                );
                ids
            }
        };
        self.await_ordinary_exit_durability_barrier(ids).await;
        let watches = if self.ordinary_exit_kill_owned {
            self.take_all_child_exit_watches()
        } else {
            Vec::new()
        };
        for watch in watches {
            watch.terminate().await;
        }
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
    }

    async fn await_ordinary_exit_durability_barrier(&mut self, ids: Vec<String>) {
        let mut pending: HashSet<String> = ids.into_iter().collect();
        let deadline = tokio::time::Instant::now() + ORDINARY_EXIT_DURABILITY_BARRIER_TIMEOUT;
        while !pending.is_empty() {
            let recv = tokio::time::timeout_at(deadline, self.tab_event_rx.recv()).await;
            let Ok(Some(crate::shell::connection::SourcedEvent::Tab(tab, event))) = recv else {
                self.notify(
                    "ordinary-exit persistence barrier timed out",
                    NotifyLevel::Error,
                );
                return;
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
            self.notify(&format!("tab {}: {msg}", tab.0), NotifyLevel::Error);
            return;
        }
    }
}

#[cfg(test)]
#[path = "app_ordinary_exit_tests.rs"]
mod tests;
