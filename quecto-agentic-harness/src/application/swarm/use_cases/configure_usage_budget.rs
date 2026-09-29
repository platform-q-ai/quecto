//! `Workbench.usage_budget(token_limit, strict_unknown=True)` (#2274): the
//! coordinator sets, changes or lifts the run's token budget.
use std::sync::Arc;

use crate::application::swarm::dto::{ConfigureUsageBudgetRequest, ConfiguredBudget};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

/// The refusal of a token limit or a strictness Python does not take.
pub const BUDGET_ARGUMENTS: &str =
    "token limit must be positive or None, strict_unknown must be boolean";

/// The arguments are checked before the operation gate: the limit is
/// `None` or an integer of 1 through `2**63 - 1` (never a boolean or a
/// float), the strictness a boolean. Through the gate (the coordinator, a
/// run in any state) a budget equal to the stored one by Python's `==` is
/// left as it is; any other is written, `warned` false, with the event
/// `usage-budget{token_limit,strict_unknown}`. The budget then applies,
/// and the answer is the report.
pub struct ConfigureUsageBudget {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ConfigureUsageBudget {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// [`BUDGET_ARGUMENTS`], an authorisation refusal, an edited budget,
    /// or the store's.
    pub fn execute(
        &self,
        request: ConfigureUsageBudgetRequest,
    ) -> Result<ConfiguredBudget, BoardError> {
        let _ = (&self.repository, &self.clock, request, BUDGET_ARGUMENTS);
        Err(BoardError::new("pending #2274"))
    }
}

#[cfg(test)]
#[path = "configure_usage_budget_tests.rs"]
mod tests;
