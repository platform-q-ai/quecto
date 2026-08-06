//! Script-managed implementation of the application's [`EnvironmentKillPort`]
//! (#1369 slice 2): terminate every member agent of the environment, then run
//! the environment's retained `kill` argv exactly once. Status transitions
//! (stopped / cleanup-failed) stay in the application use case.

use crate::domain::environment_registry::EnvironmentRecord;
use crate::domain::subagent_launch::LaunchFuture;
use crate::environment_control_app::EnvironmentKillPort;

use super::subagent_registry::{ExitSignal, SubagentRegistry};

pub struct ScriptEnvironmentKill {
    subagents: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
}

impl ScriptEnvironmentKill {
    pub fn new(
        subagents: SubagentRegistry,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    ) -> Self {
        Self {
            subagents,
            broadcast_tx,
        }
    }

    /// Terminate one member agent (and its sub-tree) exactly like `agent_cmd`
    /// `kill`: cascade-remove, claim per-entry cleanup, signal awaiters, and
    /// terminate processes/monitors. Membership removal happens inside the
    /// entry cleanup; the environment is already claimed by the caller, so no
    /// final-member cleanup can double-fire here.
    fn terminate_member(&self, registry_key: &str) {
        let super::subagent_cascade::CascadeOutcome { removed, event } =
            super::subagent_cascade::cascade_remove_and_state_changed(
                &self.subagents,
                registry_key,
            );
        let mut removed: Vec<_> = removed.into_iter().collect();
        super::subagent_cleanup::cleanup_removed_entries_once(&mut removed);
        for (_id, entry) in &removed {
            if let Some(ref tx) = entry.exit_signal_tx {
                let _ = tx.send(Some(ExitSignal {
                    exit_code: None,
                    signal: Some(15),
                }));
            }
            super::subagent_cascade::terminate_removed_entry(entry);
        }
        if let (Some(event), Some(tx)) = (event, self.broadcast_tx.as_ref()) {
            let _ = tx.send(event);
        }
    }
}

impl EnvironmentKillPort for ScriptEnvironmentKill {
    fn kill_environment<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            for member in &record.members {
                self.terminate_member(member);
            }
            if record.retained_kill_argv.is_empty() {
                return Err(format!(
                    "environment {} has no retained kill argv; its script set does not support kill_container",
                    record.environment_ref
                ));
            }
            run_retained_kill(record).await
        })
    }
}

async fn run_retained_kill(record: &EnvironmentRecord) -> Result<(), String> {
    let argv = &record.retained_kill_argv;
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", &record.environment_id);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    match cmd.output().await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "retained kill exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("failed to invoke retained kill: {e}")),
    }
}
