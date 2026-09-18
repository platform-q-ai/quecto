//! The create script's own preflight behind the environments capability's
//! [`ContainerRuntimePreflight`] port (#2024 S4b). The config's `create`
//! argv is run with `--preflight-only` appended and no child command: a
//! conforming script (the official adapter set) evaluates every
//! check and prints one `status<TAB>check<TAB>detail<TAB>remedy` line per
//! check on stdout; a script that predates the mode refuses the unknown
//! flag, and its refusal — with its stderr — is the report.
use std::time::Duration;

use crate::application::environments::dto::{
    CheckStatus, DiagnosableContainerConfig, PreflightCheck,
};
use crate::application::environments::ports::ContainerRuntimePreflight;

use super::script_stderr::{ScriptStdout, run_sync_capturing_stderr_tail};

/// The flag a conforming create script answers with its check lines.
pub const PREFLIGHT_ONLY_FLAG: &str = "--preflight-only";

/// Bound on one preflight run: the script's own repository probe is
/// bounded well under this, so a hang here is a stuck runtime CLI.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Default, Clone, Copy)]
pub struct ScriptPreflight;

impl ContainerRuntimePreflight for ScriptPreflight {
    fn preflight(
        &self,
        config: &DiagnosableContainerConfig,
    ) -> Result<Vec<PreflightCheck>, String> {
        let Some((program, args)) = config.create.split_first() else {
            return Err("the config has no create argv".to_string());
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        cmd.arg(PREFLIGHT_ONLY_FLAG);
        cmd.env("QUECTO_CONTAINER_CONFIG", &config.name);
        // No environment ref, no child command: a script that ignores the
        // flag has nothing to create with.
        cmd.env_remove("QUECTO_CONTAINER_ENVIRONMENT_REF");
        cmd.stdin(std::process::Stdio::null());
        let script = config.create.join(" ");
        let output = run_sync_capturing_stderr_tail(cmd, ScriptStdout::Result, PREFLIGHT_TIMEOUT)
            .map_err(|error| format!("create script `{script}`: {error}"))?;
        let checks = parse_checks(&output.stdout);
        if checks.is_empty() {
            let mut detail = format!(
                "create script `{script}` does not support {PREFLIGHT_ONLY_FLAG} or reported no checks (exit {})",
                output.status
            );
            if !output.stderr_tail.is_empty() {
                detail.push_str(": ");
                detail.push_str(&output.stderr_tail);
            }
            return Err(detail);
        }
        Ok(checks)
    }
}

/// Every well-formed check line of `stdout`, in order; anything else
/// (a stray log line) is ignored rather than mistaken for a check.
fn parse_checks(stdout: &[u8]) -> Vec<PreflightCheck> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let status = CheckStatus::parse(fields.next()?)?;
            let name = fields.next()?.trim();
            let detail = fields.next()?.trim();
            let remedy = fields.next().unwrap_or_default().trim();
            (!name.is_empty()).then(|| PreflightCheck {
                name: name.to_string(),
                status,
                detail: super::script_stderr::sanitise(detail),
                remedy: super::script_stderr::sanitise(remedy),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
