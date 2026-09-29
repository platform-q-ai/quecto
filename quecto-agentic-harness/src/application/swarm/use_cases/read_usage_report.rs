//! `Workbench.usage_report()` (#2273): the request-usage report,
//! read-only. S9 adds the budget's writes.
use std::sync::Arc;

use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::UsageReport;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate for reading: the report, whose read creates
/// the usage tables when absent and writes no budget row.
pub struct ReadUsageReport {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadUsageReport {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, or the store's.
    pub fn execute(&self, actor: &str) -> Result<UsageReport, BoardError> {
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, _| transaction.usage_report(),
        )
    }
}

#[cfg(test)]
#[path = "read_usage_report_tests.rs"]
mod tests;
