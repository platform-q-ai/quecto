//! Join the shared admission authority before any provider is composed
//! (#1679 P3, #2024 S3). The plan — disabled, root or child — is the
//! `NegotiateAuthority` use case's decision; a child inherits its parent's
//! authority unconditionally (#2023). The interface observes the inputs
//! (does a section resolve to a directory, is a context present), asks the
//! use case, then performs the plan through the infrastructure binding.
use std::path::Path;

use crate::application::admission::dto::{NegotiationInputs, NegotiationPlan};
use crate::application::admission::use_cases::NegotiateAuthority;
use crate::infrastructure::admission::{Negotiation, process};
use crate::infrastructure::config::Config;

pub(super) fn negotiate(
    config: &Config,
    admission_context: Option<&Path>,
    negotiate_use_case: &NegotiateAuthority,
    stderr: &mut String,
) -> bool {
    // The configured authority directory, if any; an invalid section stops
    // startup here rather than at composition.
    let configured_directory = match config.admission_proposal() {
        Ok(configured) => configured.map(|(directory, _)| directory),
        Err(error) => {
            stderr.push_str(&format!("agent: {error}\n"));
            return false;
        }
    };
    let plan = negotiate_use_case.execute(NegotiationInputs {
        configured_directory,
        inherited_context: admission_context.map(Path::to_path_buf),
    });
    let negotiation = match plan {
        NegotiationPlan::Disabled => return true,
        NegotiationPlan::Root { directory } => Negotiation::Root { directory },
        NegotiationPlan::Child { context } => Negotiation::Child { context },
    };
    match process::install(negotiation) {
        Ok(_) => true,
        Err(error) => {
            stderr.push_str(&format!("agent: admission negotiation failed: {error}\n"));
            false
        }
    }
}

/// Bounded orderly exit of the process's admission binding (no-op when disabled).
pub(crate) fn shutdown() {
    process::shutdown(std::time::Duration::from_secs(3));
}
