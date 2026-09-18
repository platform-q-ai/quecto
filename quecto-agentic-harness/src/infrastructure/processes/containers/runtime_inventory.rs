//! The environments capability's [`ContainerRuntimeInventory`] port over
//! the container CLI and the filesystem (#2024 S4d): the containers the
//! runtime lists under the `quecto.environment_id` label the shipped
//! create scripts set, their removal, and the environment directories a
//! state root holds as the scripts lay them out (`<root>/env-*/container`).
//!
//! The runtime CLI is chosen the way the scripts choose it: rootless
//! `podman` first, `docker` as a fallback, `QUECTO_CONTAINER_CLI`
//! overriding. Removal is fenced: only a container this adapter would list
//! (by name, under the label) and only a directory directly under the
//! root it was listed from, named `env-…`.
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::application::environments::dto::{EnvironmentStateDir, RuntimeContainer};
use crate::application::environments::ports::ContainerRuntimeInventory;

use super::script_stderr::{ScriptStdout, run_sync_capturing_stderr_tail};

/// The label the shipped create scripts put on every environment
/// container.
pub const ENVIRONMENT_LABEL: &str = "quecto.environment_id";

/// The prefix every environment directory (and its container name after
/// `quecto-`) carries.
pub const ENVIRONMENT_DIR_PREFIX: &str = "env-";

/// A bound for the runtime probes: an unreachable daemon must not stall
/// `container gc` or `ls` for the tool's own connect timeout.
const RUNTIME_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct RuntimeCliInventory {
    cli: Option<PathBuf>,
}

impl Default for RuntimeCliInventory {
    fn default() -> Self {
        Self::from_path()
    }
}

impl RuntimeCliInventory {
    /// The runtime CLI on PATH: `QUECTO_CONTAINER_CLI` when set, else the
    /// first of `podman`, `docker`. `None` when there is none — the
    /// inventory then reports itself unavailable rather than empty.
    pub fn from_path() -> Self {
        let candidates: Vec<String> = match std::env::var("QUECTO_CONTAINER_CLI") {
            Ok(cli) if !cli.trim().is_empty() => vec![cli],
            _ => vec!["podman".to_string(), "docker".to_string()],
        };
        let cli = candidates.iter().find_map(|name| resolve_on_path(name));
        Self { cli }
    }

    /// An inventory over an explicit CLI (the contract suite's fake).
    pub fn with_cli(cli: PathBuf) -> Self {
        Self { cli: Some(cli) }
    }

    fn cli(&self) -> Result<&Path, String> {
        self.cli.as_deref().ok_or_else(|| {
            "no container runtime on PATH: neither podman nor docker was found (QUECTO_CONTAINER_CLI overrides)".to_string()
        })
    }
}

fn resolve_on_path(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

/// One `ps -a` line: `<name>\t<state>\t<label>`.
fn parse_ps_line(line: &str) -> Option<RuntimeContainer> {
    let mut fields = line.split('\t');
    let name = fields.next()?.trim();
    if name.is_empty() {
        return None;
    }
    let state = fields.next().unwrap_or("").trim().to_ascii_lowercase();
    let label = fields.next().unwrap_or("").trim();
    Some(RuntimeContainer {
        name: name.to_string(),
        environment_id: (!label.is_empty()).then(|| label.to_string()),
        running: state == "running",
    })
}

impl ContainerRuntimeInventory for RuntimeCliInventory {
    fn containers(&self) -> Result<Vec<RuntimeContainer>, String> {
        let cli = self.cli()?;
        let mut cmd = std::process::Command::new(cli);
        cmd.args([
            "ps",
            "-a",
            "--filter",
            &format!("label={ENVIRONMENT_LABEL}"),
            "--format",
            &format!("{{{{.Names}}}}\t{{{{.State}}}}\t{{{{.Label \"{ENVIRONMENT_LABEL}\"}}}}"),
        ]);
        let output = run_sync_capturing_stderr_tail(cmd, ScriptStdout::Result, RUNTIME_TIMEOUT)
            .map_err(|error| format!("{} ps: {error}", cli.display()))?;
        if !output.status.success() {
            return Err(format!(
                "{} ps exited with {}: {}",
                cli.display(),
                output.status,
                output.stderr_tail
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.lines().filter_map(parse_ps_line).collect())
    }

    fn remove_container(&self, name: &str) -> Result<(), String> {
        let cli = self.cli()?;
        // Only what a listing would have named: an environment container.
        if !name.starts_with(&format!("quecto-{ENVIRONMENT_DIR_PREFIX}"))
            || name
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err(format!(
                "refusing to remove container '{name}': not an environment container name"
            ));
        }
        let mut cmd = std::process::Command::new(cli);
        cmd.args(["rm", "-f", name]);
        let output = run_sync_capturing_stderr_tail(cmd, ScriptStdout::Discard, RUNTIME_TIMEOUT)
            .map_err(|error| format!("{} rm -f {name}: {error}", cli.display()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} rm -f {name} exited with {}: {}",
                cli.display(),
                output.status,
                output.stderr_tail
            ))
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
            if !name.starts_with(ENVIRONMENT_DIR_PREFIX) || !entry.path().is_dir() {
                continue;
            }
            let path = entry.path();
            let container = std::fs::read_to_string(path.join("container"))
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty());
            dirs.push(EnvironmentStateDir {
                path,
                environment_id: name.to_string(),
                container,
            });
        }
        dirs.sort_by(|a, b| a.environment_id.cmp(&b.environment_id));
        Ok(dirs)
    }

    fn remove_environment_dir(&self, root: &Path, dir: &Path) -> Result<(), String> {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{} has no name", dir.display()))?;
        if !name.starts_with(ENVIRONMENT_DIR_PREFIX) || dir.parent() != Some(root) {
            return Err(format!(
                "refusing to remove {}: not an environment directory directly under {}",
                dir.display(),
                root.display()
            ));
        }
        // A link is never followed (its target is not this root's to
        // remove); an already-removed entry is done.
        match std::fs::symlink_metadata(dir) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to remove {}: it is a symbolic link",
                    dir.display()
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "refusing to remove {}: not a directory",
                    dir.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", dir.display())),
        }
        match std::fs::remove_dir_all(dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", dir.display())),
        }
    }
}

#[cfg(test)]
#[path = "runtime_inventory_tests.rs"]
mod tests;
