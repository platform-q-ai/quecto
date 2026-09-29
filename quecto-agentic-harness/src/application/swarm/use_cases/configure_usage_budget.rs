//! `Workbench.usage_budget(token_limit, strict_unknown=True)` (#2274): the
//! coordinator sets, changes or lifts the run's token budget.
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_control::edited;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_usage::apply_usage_budget;
use crate::application::swarm::dto::{BudgetChange, ConfigureUsageBudgetRequest, ConfiguredBudget};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, python_equal};

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
        let ConfigureUsageBudgetRequest {
            actor,
            token_limit,
            strict_unknown,
        } = request;
        if !(token_limit_accepted(&token_limit) && strict_unknown.is_boolean()) {
            return Err(BoardError::new(BUDGET_ARGUMENTS));
        }
        let coordinating = Access {
            coordinator: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            &actor,
            coordinating,
            |transaction, _| {
                let change =
                    if unchanged(&transaction.usage_budget()?, &token_limit, &strict_unknown)? {
                        BudgetChange::Unchanged
                    } else {
                        transaction.configure_usage_budget(&detail([
                            ("token_limit", token_limit.clone()),
                            ("strict_unknown", strict_unknown.clone()),
                            ("warned", Value::Bool(false)),
                        ]))?;
                        transaction.event(
                            &actor,
                            self.clock.now_seconds(),
                            "usage-budget",
                            &detail([
                                ("token_limit", token_limit.clone()),
                                ("strict_unknown", strict_unknown.clone()),
                            ]),
                        )?;
                        BudgetChange::Configured
                    };
                let effect = apply_usage_budget(transaction, &*self.clock, &actor)?;
                Ok(ConfiguredBudget {
                    change,
                    effect,
                    report: transaction.usage_report()?,
                })
            },
        )
    }
}

/// `limit is None or (type(limit) is int and 0 < limit <= 2**63 - 1)`.
fn token_limit_accepted(limit: &Value) -> bool {
    match limit {
        Value::Null => true,
        Value::Number(number) => number
            .as_u64()
            .is_some_and(|limit| (1..=i64::MAX.unsigned_abs()).contains(&limit)),
        Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => false,
    }
}

/// `old['token_limit'] == limit and old['strict_unknown'] == strict_unknown`
/// by Python's `==`, the strictness read only when the limits are equal.
fn unchanged(old: &Value, limit: &Value, strict_unknown: &Value) -> Result<bool, BoardError> {
    let Value::Object(old) = old else {
        return Err(edited("usage budget"));
    };
    let stored = |key: &str| old.get(key).ok_or_else(|| edited("usage budget"));
    Ok(python_equal(stored("token_limit")?, limit)
        && python_equal(stored("strict_unknown")?, strict_unknown))
}

#[cfg(test)]
#[path = "configure_usage_budget_tests.rs"]
mod tests;
