//! Release unlaunched reservations; confirm the death of a launched member
//! whose rollback observed its exit through the owned handle (#1961).
use super::swarm_bridge::SwarmContext;
use crate::domain::error::DomainError;
use crate::domain::swarm::{CoordinationPort, MemberExit, ProcessIdentity};

#[derive(Debug)]
pub struct LaunchReservation {
    context: SwarmContext,
    member: String,
    token: String,
    launched: bool,
}

impl LaunchReservation {
    pub fn reserve(context: SwarmContext) -> Result<Self, DomainError> {
        let member = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        super::swarm_lifecycle::reconcile(&context)?;
        context.reserve_member(&member, &token)?;
        Ok(Self {
            context,
            member,
            token,
            launched: false,
        })
    }

    /// The member id this reservation launched under (#1961).
    pub fn member(&self) -> &str {
        &self.member
    }

    pub fn configure(&self, command: &mut tokio::process::Command) {
        command
            .env("QUECTO_SWARM_MEMBER", &self.member)
            .env("QUECTO_SWARM_RESERVATION", &self.token)
            .env("QUECTO_SWARM_CHECKOUT", &self.context.checkout)
            .env("QUECTO_SWARM_BOOTSTRAP", "0");
    }

    /// The launch was rolled back and the owned handle's conclusion observed
    /// the child's exit (a still-running child never reaches here): that is
    /// the same authoritative death the reaper reports for a registered
    /// member, so the member is confirmed dead rather than quarantined, and
    /// the run keeps going. `exit` says whether the member ended orderly
    /// (it answered the protocol, or this harness's fallback signal reached
    /// its whole group) or was already gone before it was asked.
    pub fn rolled_back(&mut self, exit: MemberExit) -> Result<(), DomainError> {
        self.context.confirm_dead(&self.member, exit)?;
        self.launched = true;
        Ok(())
    }

    pub fn launched(&mut self, pid: u32) -> Result<(), DomainError> {
        // Once spawn succeeds an ambiguous error must retain capacity. Startup
        // joins this exact reservation; reconciliation observes kernel death.
        self.launched = true;
        let start = super::swarm_bridge::process_start(pid).ok_or_else(|| {
            DomainError::Tool(
                "cannot establish swarm child process identity; reservation retained".into(),
            )
        })?;
        self.context.record_launch(
            &self.member,
            &self.token,
            &ProcessIdentity {
                pid,
                started: start,
            },
        )?;
        Ok(())
    }
}

impl Drop for LaunchReservation {
    fn drop(&mut self) {
        if !self.launched {
            if let Err(error) = self.context.confirm_unlaunched(&self.member) {
                tracing::error!(%error, "swarm failed-launch reservation retained");
            }
        }
    }
}
