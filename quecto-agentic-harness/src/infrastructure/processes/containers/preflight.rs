//! The create script's own preflight behind the environments capability's
//! [`ContainerRuntimePreflight`] port (#2024 S4b). The config's `create`
//! argv is run with `--preflight-only` appended and no child command: a
//! conforming script (the official adapter set) evaluates every
//! check and prints one `status<TAB>check<TAB>detail<TAB>remedy` line per
//! check on stdout; a script that predates the mode refuses the unknown
//! flag, and its refusal — with its stderr — is the report.
//!
//! The adapter fails closed (#2024 S4b review): a script that dies before
//! its checks are complete — a usage error after four `ok` lines, a
//! stuck runtime — exits non-zero without a `fail` line, and that is an
//! error carrying its stderr, never the healthy-looking prefix it managed
//! to print. A check-shaped line that is not a check (a status word with
//! too few fields, a word no status) is an error naming the line rather
//! than a line to skip. A report the script exited 0 with must carry
//! every check both shipped scripts evaluate on every run, ending with
//! `state-dir` (the last check of each), or it was cut short.
use std::time::Duration;

use crate::application::environments::dto::{
    CheckStatus, DiagnosableContainerConfig, PreflightCheck,
};
use crate::application::environments::ports::ContainerRuntimePreflight;

use super::script_stderr::{ScriptStdout, run_sync_capturing_stderr_tail};

/// The flag a conforming create script answers with its check lines.
pub const PREFLIGHT_ONLY_FLAG: &str = "--preflight-only";

/// Bound on one preflight run. The official adapter makes five bounded
/// probes (image lookup, the image's shell and git, its label, its declared
/// tools, the repository), each `QUECTO_REPO_CHECK_TIMEOUT` seconds (15 by
/// default): 75 s at worst, so a hang here is a stuck runtime CLI. An
/// operator who raises that variable to 18 or more can outlast this bound;
/// the refusal then carries the script's last words.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(90);

/// The checks every shipped create script (the host-local reference set
/// and the official container adapter under `scripts/container-runtime/`)
/// reports on every run, whatever its flags: the host-local set has no
/// runtime CLI or image to check, so these are the common set.
/// `state-dir` is the last check of both, so a successful report without
/// it was cut short.
pub const MANDATORY_CHECKS: &[&str] = &["jq", "git", "repo", "state-dir"];

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
        // The argv names the config's `--repo`; its userinfo is never
        // part of a report.
        let script = crate::domain::redaction::redact_url_userinfo(&config.create.join(" "));
        let output = run_sync_capturing_stderr_tail(cmd, ScriptStdout::Result, PREFLIGHT_TIMEOUT)
            .map_err(|error| format!("create script `{script}`: {error}"))?;
        let checks = parse_checks(&output.stdout)
            .map_err(|line| format!("create script `{script}` printed a malformed check line {line:?} (expected `ok|warn|fail<TAB>check<TAB>detail[<TAB>remedy]`)"))?;
        if checks.is_empty() {
            return Err(with_stderr(
                format!(
                    "create script `{script}` does not support {PREFLIGHT_ONLY_FLAG} or reported no checks (exit {})",
                    output.status
                ),
                &output.stderr_tail,
            ));
        }
        let failed = checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed);
        if !output.status.success() && !failed {
            // Died mid-report: the checks it printed are not the checks
            // it would have made.
            return Err(with_stderr(
                format!(
                    "create script `{script}` exited {} after {} check{} without reporting a failure",
                    output.status,
                    checks.len(),
                    if checks.len() == 1 { "" } else { "s" }
                ),
                &output.stderr_tail,
            ));
        }
        if output.status.success() {
            let missing: Vec<&str> = MANDATORY_CHECKS
                .iter()
                .copied()
                .filter(|name| !checks.iter().any(|check| check.name == *name))
                .collect();
            if !missing.is_empty() {
                return Err(with_stderr(
                    format!(
                        "create script `{script}` exited 0 without reporting the mandatory check{} {} (a report cut short is not a healthy one)",
                        if missing.len() == 1 { "" } else { "s" },
                        missing.join(", ")
                    ),
                    &output.stderr_tail,
                ));
            }
        }
        Ok(checks)
    }
}

fn with_stderr(mut detail: String, stderr_tail: &str) -> String {
    if !stderr_tail.is_empty() {
        detail.push_str(": ");
        detail.push_str(stderr_tail);
    }
    detail
}

/// Every check line of `stdout`, in order. A line without a tab is a
/// stray log line and ignored; a line with one is a check and must be
/// well formed — a known status word, a name, a detail — or the whole
/// report is refused naming it (`Err(line)`): a `fail<TAB>image` cut
/// short is a failure the doctor must not skip.
fn parse_checks(stdout: &[u8]) -> Result<Vec<PreflightCheck>, String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|line| line.contains('\t'))
        .map(|line| {
            let mut fields = line.splitn(4, '\t');
            let status = fields.next().and_then(CheckStatus::parse);
            let name = fields.next().map(str::trim).filter(|name| !name.is_empty());
            let detail = fields.next().map(str::trim);
            let remedy = fields.next().unwrap_or_default().trim();
            match (status, name, detail) {
                (Some(status), Some(name), Some(detail)) => Ok(PreflightCheck {
                    name: name.to_string(),
                    status,
                    detail: super::script_stderr::sanitise(detail),
                    remedy: super::script_stderr::sanitise(remedy),
                }),
                _ => Err(line.to_string()),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
