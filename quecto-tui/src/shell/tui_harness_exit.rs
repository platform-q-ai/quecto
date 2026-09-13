//! Harness drivers for the ordinary-exit leader termination (#1956): adopt a
//! real spawned stand-in harness as a TUI-owned child, run the production
//! `finalize_ordinary_exit` with the headless agent acking the persist
//! barrier, and read back the settling notices and canary/kill reports.
//! Used by the `tui_ctrl_d_exit.feature` BDD steps.

use super::TuiHarness;
use crate::shell::child_watch::{StderrTail, watch_child};
use crate::shell::process::{LeaderBudget, LeaderEnd, LeaderTermination};
use std::time::Duration;

impl TuiHarness {
    /// Spawn `sh -c script` in its own process group exactly like the
    /// production spawn, put it under the production child watcher, and
    /// register it as a TUI-owned harness (as an in-flight tab spawn is).
    /// Returns its pid.
    pub fn adopt_owned_harness_script(&mut self, script: &str) -> u32 {
        let child = tokio::process::Command::new("sh")
            .args(["-c", script])
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn stand-in harness");
        let pid = child.id().expect("child pid");
        let watch = watch_child(child, StderrTail::default());
        self.app
            .pending_tab_child_watches
            .lock()
            .expect("pending watches lock")
            .push(watch);
        pid
    }

    /// Override the per-leader budget for the scenario (settle, force).
    pub fn set_leader_budget(&mut self, settle: Duration, force: Duration) {
        self.app.set_leader_budget(LeaderBudget { settle, force });
    }

    /// Kill-on-exit (default) or detach-on-exit policy.
    pub fn set_kill_owned_on_exit(&mut self, kill: bool) {
        self.app.set_ordinary_exit_kill_owned(kill);
    }

    /// Run the production ordinary-exit finalizer while the headless agent
    /// acknowledges the persist barrier, returning the post-cleanup
    /// finalization messages and the outcomes reported for each leader.
    pub async fn finalize_exit(&mut self) -> Vec<String> {
        let tx = self.agent_event_tx.clone();
        let finalize = self.app.finalize_ordinary_exit();
        tokio::pin!(finalize);
        loop {
            tokio::select! {
                errors = &mut finalize => return errors,
                cmd = self.cmd_rx.recv() => {
                    let (Some(cmd), Some(tx)) = (cmd, tx.as_ref()) else { continue };
                    let v = cmd.parse::<serde_json::Value>().unwrap_or_default();
                    if v["type"] == "persist_session" {
                        let ack = serde_json::json!({
                            "type": "response",
                            "command": "persist_session",
                            "id": v["id"],
                            "success": true,
                        });
                        let _ = tx.send(format!("{ack}\n")).await;
                    }
                }
            }
        }
    }

    /// Every settling notice shown during the exit, in order.
    pub fn settling_notices_shown(&self) -> Vec<String> {
        self.app.exit_policy.settling_notices_shown.clone()
    }

    /// Whether a settling notice is still on the notification stack.
    pub fn settling_notice_visible(&self) -> bool {
        self.app
            .notifications
            .messages()
            .iter()
            .any(|m| m.starts_with(crate::shell::app::app_ordinary_exit::SETTLING_NOTICE_PREFIX))
    }

    /// Whether any TUI-owned watch is still held (detach leaves them).
    pub fn owned_watches_remaining(&mut self) -> usize {
        self.app.take_all_child_exit_watches().len()
    }

    /// Terminate one adopted harness directly through the watcher API with
    /// `budget` (the same helper tab close and `/new` use) and report.
    pub async fn terminate_adopted_harness(
        &mut self,
        settle: Duration,
    ) -> Option<LeaderTermination> {
        let budget = LeaderBudget {
            settle,
            force: settle,
        };
        let watch = self
            .app
            .pending_tab_child_watches
            .lock()
            .expect("pending watches lock")
            .pop()?;
        watch.terminate_with_budget(budget).await
    }

    /// Human-readable name of a leader end, for step assertions.
    pub fn leader_end_name(end: LeaderEnd) -> &'static str {
        match end {
            LeaderEnd::AlreadyExited => "already-exited",
            LeaderEnd::ExitedAfterTerm => "exited-after-term",
            LeaderEnd::ExitedAfterRepeatedTerm => "exited-after-repeated-term",
            LeaderEnd::Killed => "killed",
            LeaderEnd::NoPid => "no-pid",
        }
    }
}
