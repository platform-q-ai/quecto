//! The retained script argv of an environment, run synchronously against
//! its runtime id (#1369 slice 3, #2024 S4d): the bounded `inspect` whose
//! result is parsed through the strict wire contract, and the best-effort
//! `cleanup`. Shared by the session's async command adapter
//! (`tools/environment_commands.rs`) and the synchronous liveness/cleanup
//! adapter behind the restore and the collector (`environment_process.rs`).
use std::time::Duration;

use super::script_stderr::{ScriptStdout, run_sync_capturing_stderr_tail};

/// Bound on one retained-inspect invocation. A hung inspect script must not
/// stall the death pipeline indefinitely: the exit signal (and the awaits it
/// wakes) fires only after this job finishes, and `classify_dead_socket`'s
/// grace window is sized just above this bound.
pub const INSPECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The environment variable every retained script receives.
pub const ENVIRONMENT_ID_VAR: &str = "QUECTO_CONTAINER_ENVIRONMENT_ID";

/// Strict wire parse shared by the create, exec, and inspect result
/// contracts: UTF-8 only, exactly one JSON value, trailing data rejected.
/// Unknown-key rejection comes from each wire type's `deny_unknown_fields`.
/// Returns a plain error string so both launch-path (`DomainError`) and
/// post-mortem (`String`) callers share one definition.
pub fn parse_strict_wire<T: serde::de::DeserializeOwned>(
    stdout: &[u8],
    operation: &str,
) -> Result<T, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|e| format!("script-managed {operation} returned non-UTF8 JSON: {e}"))?;
    let mut de = serde_json::Deserializer::from_str(text);
    let wire = T::deserialize(&mut de)
        .map_err(|e| format!("script-managed {operation} returned invalid JSON contract: {e}"))?;
    de.end()
        .map_err(|e| format!("script-managed {operation} returned extra JSON data: {e}"))?;
    Ok(wire)
}

/// Inspect script contract: invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID`,
/// prints one JSON object `{"status": "...", "metadata": {...}}` on stdout.
/// The result is parsed through the same strict wire path as create/exec:
/// unknown keys, trailing data, and non-UTF8 output are rejected. The
/// subprocess is bounded by `timeout`; on timeout it is killed and the
/// failure is reported with the retained argv kept for retry. The
/// script's `status` travels on the metadata as `inspect_status`.
pub fn run_inspect_sync_bounded(
    environment_id: &str,
    argv: &[String],
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InspectResultWire {
        #[serde(default)]
        status: Option<String>,
        metadata: serde_json::Value,
    }
    let Some((program, args)) = argv.split_first() else {
        return Err("no retained inspect argv".to_string());
    };
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.env(ENVIRONMENT_ID_VAR, environment_id);
    let output = run_sync_capturing_stderr_tail(cmd, ScriptStdout::Result, timeout)
        .map_err(|error| format!("retained inspect: {error}; retained argv kept for retry"))?;
    if !output.status.success() {
        return Err(format!(
            "retained inspect exited with {}: {}",
            output.status, output.stderr_tail
        ));
    }
    let wire: InspectResultWire = parse_strict_wire(&output.stdout, "inspect")?;
    if !wire.metadata.is_object() {
        return Err("retained inspect result must contain a metadata object".to_string());
    }
    let mut metadata = wire.metadata;
    if let (Some(object), Some(status)) = (metadata.as_object_mut(), wire.status) {
        object.insert("inspect_status".to_string(), serde_json::json!(status));
    }
    Ok(metadata)
}

/// The retained-cleanup invocation, reported rather than silent (#2024
/// S4b): a cleanup that fails leaves an environment behind and its stderr
/// tail is the operator's only lead. `Ok` when the script reported
/// success; `Err` carries the script's own account (or why it could not
/// be started — typically a full pid cgroup).
pub fn run_cleanup_sync(environment_id: &str, argv: &[String]) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("no retained cleanup argv".to_string());
    };
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.env(ENVIRONMENT_ID_VAR, environment_id);
    match run_sync_capturing_stderr_tail(cmd, ScriptStdout::Discard, Duration::MAX) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(output.failure_message("cleanup")),
        Err(error) => Err(format!(
            "retained container script could not be started: {error}"
        )),
    }
}
