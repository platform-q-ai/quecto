use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

pub struct SyncInput {
    pub epoch: u64,
    pub since_rev: u64,
    pub agent_id: Option<String>,
}

pub async fn execute(gateway: &dyn AgentGateway, input: SyncInput) -> Result<AgentEvent, ApiError> {
    gateway
        .send(AgentCommand::Sync {
            epoch: input.epoch,
            since_rev: input.since_rev,
            agent_id: input.agent_id,
        })
        .await
}

#[cfg(test)]
#[path = "sync_ledger_tests.rs"]
mod tests;
