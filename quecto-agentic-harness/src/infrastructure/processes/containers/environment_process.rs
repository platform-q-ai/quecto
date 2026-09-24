//! The environments capability's [`EnvironmentProcess`] port over the
//! record's own retained scripts (#2024 S4d): liveness through the
//! retained `inspect` argv (the same bounded run the post-mortem uses,
//! judged by the script's `status` word), cleanup through the retained
//! `cleanup` argv, and whether a state directory is still on disk (#2134)
//! through the host filesystem. Synchronous, so the startup restore and
//! the CLI collector need no runtime.
use std::path::Path;

use crate::application::environments::dto::{EnvironmentLiveness, StateOnDisk};
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

    /// Anything under the directory's name counts as present, a dangling
    /// symlink included. A missing name is absent only while its state
    /// root is a directory: a root that is itself missing (an unmounted
    /// disk) says nothing about what the environment left.
    fn state_on_disk(&self, environment_dir: &Path) -> StateOnDisk {
        match std::fs::symlink_metadata(environment_dir) {
            Ok(_) => StateOnDisk::Present,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match environment_dir.parent().map(std::fs::metadata) {
                    Some(Ok(root)) if root.is_dir() => StateOnDisk::Absent,
                    _ => StateOnDisk::Unknown(format!(
                        "{} is gone with its state root: is the disk mounted?",
                        environment_dir.display()
                    )),
                }
            }
            Err(error) => StateOnDisk::Unknown(format!(
                "{} could not be examined: {error}",
                environment_dir.display()
            )),
        }
    }

    fn inspect_clock_millis(&self) -> u64 {
        static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let elapsed = ORIGIN.get_or_init(std::time::Instant::now).elapsed();
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
#[path = "environment_process_tests.rs"]
mod tests;
