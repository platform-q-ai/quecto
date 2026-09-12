//! UDS startup helpers of the `agent` subcommand: consume the launch-bound
//! parent control sidecar before anything else can observe this process
//! (#1935) — a launcher-created child fails startup closed on a missing or
//! malformed credential, never starting as a bindless child no parent could
//! claim — and pick the auto-generated socket path.
use std::path::Path;

use crate::domain::harness_lifetime::HarnessLifetime;
use crate::domain::parent_control::ParentControlBinding;
use crate::infrastructure::processes::parent_control::take_sidecar;
use crate::interface::cli::uds_teardown_graph::{
    BIND_DEADLINE_ENV, BindDeadline, DEFAULT_BIND_DEADLINE, ParentControlLaunch,
};

/// `Some(binding)` to run with (an `unlaunched` binding for a top-level
/// harness), `None` when startup must stop with the error on `stderr`.
pub(super) fn consume(
    sidecar: Option<&Path>,
    composed: bool,
    stderr: &mut String,
) -> Option<Option<ParentControlLaunch>> {
    let Some(path) = sidecar else {
        return Some(None);
    };
    if !composed {
        // Fail closed: a launched child without the composed teardown graph
        // could never bind, so it must not start as a bindless orphan.
        stderr.push_str(
            "agent: a launched child needs the composed teardown graph (run through the quecto binary)\n",
        );
        let _ = take_sidecar(path);
        return None;
    }
    match take_sidecar(path) {
        Ok(credential) => Some(Some(ParentControlLaunch {
            binding: ParentControlBinding::launched(credential),
            bind_deadline: BindDeadline::After(bind_deadline(
                std::env::var(BIND_DEADLINE_ENV).ok().as_deref(),
            )),
        })),
        Err(error) => {
            stderr.push_str(&format!("agent: parent control: {error}\n"));
            None
        }
    }
}

/// Consume the sidecar, then decide the harness lifetime once (#1937): a
/// launcher-created child is launch-bound and can never persist. The sidecar
/// is consumed before the lifetime check, so a refused child leaves no
/// credential behind. `None` when startup must stop with the error on
/// `stderr`.
pub(super) fn consume_and_resolve_lifetime(
    sidecar: Option<&Path>,
    persist_requested: bool,
    composed: bool,
    stderr: &mut String,
) -> Option<(Option<ParentControlLaunch>, HarnessLifetime)> {
    let parent_control = consume(sidecar, composed, stderr)?;
    let lifetime = match HarnessLifetime::resolve(persist_requested, parent_control.is_some()) {
        Ok(lifetime) => lifetime,
        Err(error) => {
            stderr.push_str(&format!("agent: {error}\n"));
            return None;
        }
    };
    if lifetime == HarnessLifetime::Persistent {
        stderr.push_str(
            "WARNING: --persist keeps the agent alive indefinitely. Shutdown via SIGTERM/SIGINT only.\n",
        );
    }
    Some((parent_control, lifetime))
}

/// The bind deadline: the default unless a positive millisecond override is
/// given (tests); anything else keeps the default.
pub(super) fn bind_deadline(override_ms: Option<&str>) -> std::time::Duration {
    override_ms
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or(DEFAULT_BIND_DEADLINE)
}

/// Auto-generated socket path in `$XDG_RUNTIME_DIR` (or temp) for a harness
/// started without `--socket`.
pub(super) fn auto_socket_path() -> std::path::PathBuf {
    let dir = crate::interface::shared::xdg_runtime_dir_or_temp();
    // Best-effort: reap dead quecto-agent-*.sock files (kernel-table
    // probed, #1460 — a live agent is never touched, let alone severed,
    // regardless of age; the 24 h threshold only gates files whose probe
    // is inconclusive). Drop guards do not run on SIGKILL so dead
    // sockets can accumulate.
    crate::interface::cli::uds::reap_stale_sockets(&dir, std::time::Duration::from_secs(86_400));
    let id = uuid::Uuid::new_v4();
    dir.join(format!("quecto-agent-{id}.sock"))
}

#[cfg(test)]
#[path = "parent_control_startup_tests.rs"]
mod tests;
