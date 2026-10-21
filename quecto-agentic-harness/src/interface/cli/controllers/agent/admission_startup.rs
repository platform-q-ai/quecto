//! Join the shared admission authority before any provider is composed
//! (#1679 P3, #2024 S3). The plan — disabled, root or child — is the
//! `NegotiateAuthority` use case's decision; a child inherits its parent's
//! authority unconditionally (#2023). The interface observes the inputs
//! (does a section resolve to a directory, is a context present), asks the
//! use case, then performs the plan through the infrastructure binding. A
//! root first has the use case make sure a broker answers, starting one on
//! demand (#2615, ADR-0032).
use std::path::{Path, PathBuf};

use crate::application::admission::dto::negotiation::{NegotiationInputs, NegotiationPlan};
use crate::application::admission::ports::AuthorityLaunch;
use crate::application::admission::use_cases::NegotiateAuthority;
use crate::application::configuration::dto::ConfigSelection;
use crate::infrastructure::admission::{Negotiation, process};
use crate::infrastructure::config::Config;

/// Where this process's configuration came from, for a broker started on
/// demand to serve the same `admission` section (#2615).
pub(super) struct ConfigOrigin<'a> {
    pub(super) base_dir: &'a Path,
    pub(super) selection: &'a ConfigSelection,
}

/// Build the composed negotiation use case from the agent flags and run it
/// (#2024 S3). Keeps `agent.rs` free of the capability-not-composed plumbing.
pub(super) fn negotiate_from_flags(
    config: &Config,
    base_dir: &Path,
    selection: &ConfigSelection,
    flags: &super::AgentFlags,
    stderr: &mut String,
) -> bool {
    let Some(build_admission) = flags.admission else {
        stderr.push_str("agent: admission capability not composed\n");
        return false;
    };
    let handles = build_admission();
    let joined = negotiate(
        config,
        &ConfigOrigin {
            base_dir,
            selection,
        },
        flags.admission_context.as_deref(),
        &handles.negotiate,
        stderr,
    );
    match joined {
        Some(Joined::Root(launch)) => {
            // `/admission restart` (#2615) restarts what this root joined.
            if let Some(binding) = process::current() {
                binding.remember_launch(launch);
            }
            crate::interface::uds::admission::restart_controller::register(handles.restart.clone());
            true
        }
        Some(Joined::Disabled | Joined::Child) => true,
        None => false,
    }
}

/// How this process joined the authority, once it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Joined {
    /// No `admission` section: nothing joined.
    Disabled,
    /// A root, registered at the broker serving this launch.
    Root(AuthorityLaunch),
    /// A child, bound to its parent's capability.
    Child,
}

/// The plan for this process: a child inherits whatever its own config says
/// (the section is global-only and the parent already composed against the
/// published policy, so even an invalid section in the child's explicit
/// `--config` is not its concern); a root's section is validated here so an
/// invalid one stops startup rather than composition.
pub(super) fn plan(
    config: &Config,
    admission_context: Option<&Path>,
    negotiate_use_case: &NegotiateAuthority,
) -> Result<NegotiationPlan, String> {
    let configured_directory = if admission_context.is_some() {
        None
    } else {
        config
            .admission_proposal()
            .map_err(|error| error.to_string())?
            .map(|(directory, _)| directory)
    };
    Ok(negotiate_use_case.execute(NegotiationInputs {
        configured_directory,
        inherited_context: admission_context.map(Path::to_path_buf),
    }))
}

/// What a broker started on demand for `directory` serves: the global
/// config this process's `admission` section comes from (an explicit
/// `--config`, else `<base_dir>/config.json`; the section is global-only),
/// made absolute because the broker runs from `/`.
pub(super) fn launch_request(directory: &Path, origin: &ConfigOrigin<'_>) -> AuthorityLaunch {
    let config = match origin.selection {
        ConfigSelection::Explicit(path) => path.clone(),
        ConfigSelection::Layered(layers) => layers.global.clone(),
    };
    AuthorityLaunch {
        directory: directory.to_path_buf(),
        config: absolute(config),
        base_dir: absolute(origin.base_dir.to_path_buf()),
    }
}

fn absolute(path: PathBuf) -> PathBuf {
    match path.is_absolute() {
        true => path,
        false => std::path::absolute(&path).unwrap_or(path),
    }
}

/// Join as the plan says; `None` (with the reason on `stderr`) when this
/// process must not start.
pub(super) fn negotiate(
    config: &Config,
    origin: &ConfigOrigin<'_>,
    admission_context: Option<&Path>,
    negotiate_use_case: &NegotiateAuthority,
    stderr: &mut String,
) -> Option<Joined> {
    let plan = match plan(config, admission_context, negotiate_use_case) {
        Ok(plan) => plan,
        Err(error) => {
            stderr.push_str(&format!("agent: {error}\n"));
            return None;
        }
    };
    let outcome = match plan {
        NegotiationPlan::Disabled => return Some(Joined::Disabled),
        NegotiationPlan::Root { directory } => {
            join_root(&directory, origin, negotiate_use_case).map(Joined::Root)
        }
        NegotiationPlan::Child { context } => {
            process::install(Negotiation::Child { context }).map(|_| Joined::Child)
        }
    };
    match outcome {
        Ok(joined) => Some(joined),
        Err(error) => {
            stderr.push_str(&format!("agent: admission negotiation failed: {error}\n"));
            None
        }
    }
}

/// A root makes sure a broker answers, then registers there. A broker that
/// stopped itself for being idle in the instant this root reached it
/// (#2615) closes the registration; the root then makes sure once more,
/// starting a fresh one, and registers again. Any other failure is final
/// on the second attempt as well.
fn join_root(
    directory: &Path,
    origin: &ConfigOrigin<'_>,
    negotiate_use_case: &NegotiateAuthority,
) -> Result<AuthorityLaunch, String> {
    let launch = launch_request(directory, origin);
    let mut last = String::new();
    for _ in 0..2 {
        negotiate_use_case.ensure_running(&launch)?;
        match process::install(Negotiation::Root {
            directory: directory.to_path_buf(),
        }) {
            Ok(_) => return Ok(launch),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Bounded orderly exit of the process's admission binding (no-op when disabled).
pub(crate) fn shutdown() {
    process::shutdown(std::time::Duration::from_secs(3));
}

#[cfg(test)]
#[path = "admission_startup_tests.rs"]
mod tests;
