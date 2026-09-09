//! Join the shared admission authority before any provider is composed
//! (#1679 P3). Roots register themselves; descendants bind the capability
//! their parent registered. Either failure stops startup before inference and
//! before a child announces socket readiness.
use std::path::Path;

use crate::infrastructure::admission::{Negotiation, process};
use crate::infrastructure::config::Config;

pub(super) fn negotiate(
    config: &Config,
    admission_context: Option<&Path>,
    stderr: &mut String,
) -> bool {
    let configured = match config.admission_proposal() {
        Ok(configured) => configured,
        Err(error) => {
            stderr.push_str(&format!("agent: {error}\n"));
            return false;
        }
    };
    let negotiation = match (admission_context, configured) {
        (Some(context), _) => Negotiation::Child {
            context: context.to_path_buf(),
        },
        (None, Some((directory, _))) => Negotiation::Root { directory },
        (None, None) => return true,
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
