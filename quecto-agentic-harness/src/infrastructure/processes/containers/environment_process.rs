//! The environments capability's [`EnvironmentProcess`] port over the
//! record's own retained scripts (#2024 S4d): liveness through the
//! retained `inspect` argv (the same bounded run the post-mortem uses,
//! judged by the script's `status` word), cleanup through the retained
//! `cleanup` argv. Synchronous, so the startup restore and the CLI
//! collector need no runtime.
use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::ports::EnvironmentProcess;
use crate::domain::environment_registry::EnvironmentRecord;

use super::retained_scripts::{INSPECT_TIMEOUT, run_cleanup_sync, run_inspect_sync_bounded};

#[derive(Debug, Default, Clone, Copy)]
pub struct ScriptEnvironmentProcess;

/// The inspect status words the shipped scripts print, and what they mean
/// for a restore: a script that says the container runs is believed; one
/// that says it is dead (removed, exited, OOM-killed) makes the record
/// stale; anything else leaves the record unverified.
pub fn liveness_from_inspect_status(status: Option<&str>) -> EnvironmentLiveness {
    match status {
        Some("running") => EnvironmentLiveness::Running,
        Some("dead") | Some("exited") | Some("removed") | Some("stopped") => {
            EnvironmentLiveness::Gone
        }
        Some(other) => EnvironmentLiveness::Unknown(format!(
            "retained inspect reported status '{other}', which names neither a live nor a dead container"
        )),
        None => EnvironmentLiveness::Unknown(
            "retained inspect reported no status; the record is kept as recorded".to_string(),
        ),
    }
}

impl EnvironmentProcess for ScriptEnvironmentProcess {
    fn observe(&self, record: &EnvironmentRecord) -> EnvironmentLiveness {
        if record.retained_inspect_argv.is_empty() {
            return EnvironmentLiveness::Unknown(
                "no retained inspect argv: the script set cannot report liveness".to_string(),
            );
        }
        match run_inspect_sync_bounded(
            &record.environment_id,
            &record.retained_inspect_argv,
            INSPECT_TIMEOUT,
        ) {
            Ok(metadata) => liveness_from_inspect_status(
                metadata
                    .get("inspect_status")
                    .and_then(serde_json::Value::as_str),
            ),
            Err(error) => EnvironmentLiveness::Unknown(error),
        }
    }

    fn cleanup(&self, record: &EnvironmentRecord) -> Result<(), String> {
        run_cleanup_sync(&record.environment_id, &record.retained_cleanup_argv)
    }
}

#[cfg(test)]
#[path = "environment_process_tests.rs"]
mod tests;
