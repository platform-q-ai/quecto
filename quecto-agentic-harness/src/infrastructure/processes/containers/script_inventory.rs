//! The environments capability's [`ContainerRuntimeInventory`] port over a
//! container config's own scripts (#2024 S4d): the harness knows no
//! runtime, so the config's `inspect` argv is asked for every environment
//! the runtime knows (`--list`, one JSON object per line) and its
//! `cleanup` argv removes one by environment id — the container and the
//! state directory together, as the shipped scripts do. The state
//! directories under a root are read from the filesystem as the scripts
//! lay them out (`<root>/env-*/container`).
use std::path::Path;
use std::time::Duration;

use crate::application::environments::dto::{
    DiagnosableContainerConfig, EnvironmentLiveness, EnvironmentStateDir, RuntimeContainer,
};
use crate::application::environments::ports::ContainerRuntimeInventory;

use super::environment_process::liveness_from_inspect_status;
use super::retained_scripts::{
    ENVIRONMENT_ID_VAR, INSPECT_TIMEOUT, run_cleanup_sync, run_inspect_sync_bounded,
};
use super::script_stderr::{ScriptStdout, run_sync_capturing_stderr_tail};
use super::standard::integrity::refuse_altered_script;

/// The prefix every environment directory the shipped scripts create
/// carries.
pub const ENVIRONMENT_DIR_PREFIX: &str = "env-";

/// The flag that turns an `inspect` into a listing.
pub const LIST_FLAG: &str = "--list";

/// A bound for the listing: an unreachable runtime must not stall
/// `container gc` for the runtime's own connect timeout.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, Clone, Copy)]
pub struct ScriptInventory;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListedWire {
    environment_id: String,
    container: String,
    status: String,
}

/// Parse the listing's lines: one JSON object each, blank lines ignored,
/// a malformed line refuses the whole listing (a partial inventory would
/// call the unlisted orphans).
pub fn parse_listing(stdout: &[u8]) -> Result<Vec<RuntimeContainer>, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|error| format!("inspect --list printed non-UTF8 output: {error}"))?;
    let mut listed = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let wire: ListedWire = serde_json::from_str(line).map_err(|error| {
            format!(
                "inspect --list printed a malformed line {}: {error} (expected {{\"environment_id\", \"container\", \"status\"}})",
                index + 1
            )
        })?;
        if wire.environment_id.is_empty() {
            return Err(format!(
                "inspect --list printed a line {} without an environment_id",
                index + 1
            ));
        }
        listed.push(RuntimeContainer {
            environment_id: wire.environment_id,
            container: wire.container,
            running: wire.status == "running",
        });
    }
    Ok(listed)
}

impl ContainerRuntimeInventory for ScriptInventory {
    fn containers(
        &self,
        config: &DiagnosableContainerConfig,
    ) -> Result<Vec<RuntimeContainer>, String> {
        let Some((program, args)) = config.inspect.split_first() else {
            return Err(format!(
                "container config '{}' has no inspect script",
                config.name
            ));
        };
        refuse_altered_script(&config.inspect)
            .map_err(|reason| format!("inspect --list refused: {reason}"))?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        cmd.arg(LIST_FLAG);
        // No environment is being inspected; a script that insists on one
        // predates the listing contract and says so on stderr.
        cmd.env_remove(ENVIRONMENT_ID_VAR);
        let output = run_sync_capturing_stderr_tail(cmd, ScriptStdout::Result, LIST_TIMEOUT)
            .map_err(|error| format!("inspect --list: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "inspect --list exited with {}: {} (a script set without --list cannot serve the collector)",
                output.status, output.stderr_tail
            ));
        }
        parse_listing(&output.stdout)
    }

    fn inspect(
        &self,
        config: &DiagnosableContainerConfig,
        environment_id: &str,
    ) -> EnvironmentLiveness {
        if !well_formed_id(environment_id) {
            return EnvironmentLiveness::Unknown(format!(
                "'{environment_id}' is not an environment id"
            ));
        }
        match run_inspect_sync_bounded(environment_id, &config.inspect, INSPECT_TIMEOUT) {
            Ok(metadata) => liveness_from_inspect_status(
                metadata
                    .get("inspect_status")
                    .and_then(serde_json::Value::as_str),
            ),
            Err(error) => EnvironmentLiveness::Unknown(error),
        }
    }

    fn environment_dirs(&self, root: &Path) -> Result<Vec<EnvironmentStateDir>, String> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("{}: {error}", root.display())),
        };
        let mut dirs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            // A link is not an environment directory (its target is not
            // this root's); only a real directory is listed.
            let file_type = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            if !name.starts_with(ENVIRONMENT_DIR_PREFIX) || !file_type.is_dir() {
                continue;
            }
            let path = entry.path();
            let age_secs = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .map(|age| age.as_secs());
            let container = std::fs::read_to_string(path.join("container"))
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty());
            dirs.push(EnvironmentStateDir {
                path,
                environment_id: name.to_string(),
                container,
                age_secs,
            });
        }
        dirs.sort_by(|a, b| a.environment_id.cmp(&b.environment_id));
        Ok(dirs)
    }

    fn canonical_root(&self, root: &Path) -> std::path::PathBuf {
        std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
    }

    fn remove(
        &self,
        config: &DiagnosableContainerConfig,
        environment_id: &str,
    ) -> Result<(), String> {
        // Only an id the listing could have produced: a path-shaped or
        // empty id never reaches a script.
        if !well_formed_id(environment_id) {
            return Err(format!(
                "refusing to remove '{environment_id}': not an environment id"
            ));
        }
        run_cleanup_sync(environment_id, &config.cleanup)
    }
}

/// An environment id as the shipped scripts mint them: never empty, never
/// path-shaped, nothing a shell could read as more than a word.
fn well_formed_id(environment_id: &str) -> bool {
    !environment_id.is_empty()
        && environment_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
#[path = "script_inventory_tests.rs"]
mod tests;
