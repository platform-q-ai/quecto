//! Decide how a starting process joins the authority (#2024 S3, #2023), and
//! make sure a root has one to join (#2615).
//!
//! This is the one home of the rule that a child inherits its parent's
//! authority unconditionally: an inherited `--admission-context` always wins,
//! whatever the child's own config says (which S1 made unable to carry an
//! `admission` section at all, and whose explicit `--config` may still differ
//! or be `null`). The byte-equal candidate check that used to reject a
//! diverging child is retired in favour of this rule plus the `inherit` flag
//! the composition passes to the runtime factory.
//!
//! The decision is pure: the interface observes the inputs (does a section
//! resolve to a directory, is a context present) and performs the resulting
//! plan through infrastructure. A root whose authority does not answer
//! starts one on demand through the launcher port (ADR-0032); a child never
//! does, since it can only bind the capability its parent registered.

use std::sync::Arc;

use crate::application::admission::dto::negotiation::{
    AuthorityAvailability, NegotiationInputs, NegotiationPlan,
};
use crate::application::admission::ports::{AuthorityLaunch, AuthorityLauncher, LaunchState};

/// How many probes follow a launch: the link's reconnect bound (≈10 s with
/// its backoff), ample for a broker to bind its sockets.
pub const ENSURE_ATTEMPTS: u32 = 8;

pub struct NegotiateAuthority {
    launcher: Arc<dyn AuthorityLauncher>,
}

impl std::fmt::Debug for NegotiateAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NegotiateAuthority").finish_non_exhaustive()
    }
}

impl NegotiateAuthority {
    pub fn new(launcher: Arc<dyn AuthorityLauncher>) -> Self {
        Self { launcher }
    }

    pub fn execute(&self, inputs: NegotiationInputs) -> NegotiationPlan {
        // A child wins unconditionally: an inherited context binds the
        // parent's capability regardless of the child's own configuration.
        if let Some(context) = inputs.inherited_context {
            return NegotiationPlan::Child { context };
        }
        match inputs.configured_directory {
            Some(directory) => NegotiationPlan::Root { directory },
            None => NegotiationPlan::Disabled,
        }
    }

    /// Make sure a broker answers at `launch.directory` before a root
    /// registers there (#2615).
    ///
    /// A broker that already answers is joined as it is (whether this
    /// process, another session or the systemd unit started it). Otherwise
    /// one is launched detached and probed through the bounded backoff. A
    /// launch that loses the singleton lock to a concurrent starter waits
    /// for the winner; only if the winner still does not answer a step
    /// later (it was stopping for being idle when this start lost to it) is
    /// another launched. A broker that exits for any other reason ends the
    /// wait with its reason, and one that never answers is given up after
    /// [`ENSURE_ATTEMPTS`] probes.
    pub fn ensure_running(
        &self,
        launch: &AuthorityLaunch,
    ) -> Result<AuthorityAvailability, String> {
        if self.launcher.probe(&launch.directory).is_ok() {
            return Ok(AuthorityAvailability::AlreadyRunning);
        }
        let mut launched = self.launcher.launch(launch)?;
        let mut lost_race = false;
        let mut last = String::from("not probed");
        for attempt in 0..ENSURE_ATTEMPTS {
            self.launcher.pause(attempt);
            match self.launcher.probe(&launch.directory) {
                Ok(()) => return Ok(AuthorityAvailability::StartedOnDemand),
                Err(reason) => last = reason,
            }
            match launched.state() {
                LaunchState::Running => {}
                LaunchState::LostRace => match lost_race {
                    false => lost_race = true,
                    true => {
                        launched = self.launcher.launch(launch)?;
                        lost_race = false;
                    }
                },
                LaunchState::Failed { reason } => return Err(reason),
            }
        }
        Err(format!(
            "the admission broker started for {} did not answer ({last})",
            launch.directory.display()
        ))
    }
}

#[cfg(test)]
#[path = "negotiate_authority_tests.rs"]
mod tests;
