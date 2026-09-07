//! A reservation is released only before launch, or after the child has exited.
use super::swarm_bridge::SwarmContext;
use crate::domain::error::DomainError;
use serde_json::json;

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
        context.call("_admit", json!([member, token]))?;
        Ok(Self {
            context,
            member,
            token,
            launched: false,
        })
    }

    pub fn configure(&self, command: &mut tokio::process::Command) {
        command
            .env("QUECTO_SWARM_MEMBER", &self.member)
            .env("QUECTO_SWARM_RESERVATION", &self.token)
            .env("QUECTO_SWARM_CHECKOUT", &self.context.checkout)
            .env("QUECTO_SWARM_BOOTSTRAP", "0");
    }

    pub fn confirmed_dead(&mut self) -> Result<(), DomainError> {
        self.context.call("_confirmed_dead", json!([self.member]))?;
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
        self.context.call(
            "_record_launch",
            json!([self.member, self.token, pid, start]),
        )?;
        Ok(())
    }
}

impl Drop for LaunchReservation {
    fn drop(&mut self) {
        if !self.launched {
            if let Err(error) = self.context.call("_confirmed_dead", json!([self.member])) {
                tracing::error!(%error, "swarm failed-launch reservation retained");
            }
        }
    }
}
