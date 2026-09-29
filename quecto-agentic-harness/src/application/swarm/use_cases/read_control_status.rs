//! `Workbench._control_status()` (#2273): the control receipt, read-only.
use std::sync::Arc;

use crate::application::swarm::board_control::receipt;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::ControlReceipt;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate for reading (a dead member may still read):
/// the receipt. Reading the usage report creates its tables when absent.
pub struct ReadControlStatus {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadControlStatus {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, an edited control record, or the store's.
    pub fn execute(&self, actor: &str) -> Result<ControlReceipt, BoardError> {
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, _| receipt(transaction, &*self.clock),
        )
    }
}

#[cfg(test)]
#[path = "read_control_status_tests.rs"]
mod tests;
