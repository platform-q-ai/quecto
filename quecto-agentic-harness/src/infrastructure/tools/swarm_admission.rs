//! Release unlaunched reservations; confirm the death of a launched member
//! whose rollback observed its exit through the owned handle (#1961).
use super::swarm_bridge::SwarmContext;
use crate::application::swarm::ports::CoordinationPort;
use crate::domain::error::DomainError;
use crate::domain::swarm::{MemberExit, ProcessIdentity};

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
    pub async fn rolled_back(&mut self, exit: MemberExit) -> Result<(), DomainError> {
        let (context, member) = (self.context.clone(), self.member.clone());
        off_the_workers(move || context.confirm_dead(&member, exit)).await?;
        self.launched = true;
        Ok(())
    }

    pub async fn launched(&mut self, pid: u32) -> Result<(), DomainError> {
        // Once spawn succeeds an ambiguous error must retain capacity. Startup
        // joins this exact reservation; reconciliation observes kernel death.
        self.launched = true;
        let start = super::swarm_bridge::process_start(pid).ok_or_else(|| {
            DomainError::Tool(
                "cannot establish swarm child process identity; reservation retained".into(),
            )
        })?;
        let (context, member, token) = (
            self.context.clone(),
            self.member.clone(),
            self.token.clone(),
        );
        off_the_workers(move || {
            context.record_launch(
                &member,
                &token,
                &ProcessIdentity {
                    pid,
                    started: start,
                },
            )
        })
        .await
    }
}

/// A reservation's board call, off the async workers (#2278 review L6).
async fn off_the_workers(
    job: impl FnOnce() -> Result<(), DomainError> + Send + 'static,
) -> Result<(), DomainError> {
    super::call_work::off_the_workers(job)
        .await
        .map_err(|error| DomainError::Tool(format!("swarm reservation update: {error}")))?
}

impl Drop for LaunchReservation {
    fn drop(&mut self) {
        match self.launched {
            true => {}
            false => {
                let (context, member) = (self.context.clone(), self.member.clone());
                super::swarm_lifecycle::run_off_the_workers(move || {
                    if let Err(error) = context.confirm_unlaunched(&member) {
                        tracing::error!(%error, "swarm failed-launch reservation retained");
                    }
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "swarm_admission_tests.rs"]
mod tests;
