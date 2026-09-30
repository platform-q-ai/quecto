//! The token budget's application (#2274): the port of
//! `Coordination._apply_usage_budget` and the repository's
//! `mark_usage_warning`, which `usage_budget`, `_record_request` and
//! `_request_admission` each run inside their transaction. A
//! capability-internal helper, not a use case and not a port.
use serde_json::Value;

use super::board_control::{budget_decision, current, edited};
use super::board_operation::{detail, end};
use super::dto::{BudgetEffect, UsageStanding};
use super::ports::{BoardEvents, BoardRuns, BoardUsage, Clock};
use crate::domain::swarm::{BoardError, RunState, UsageDecision};

/// The reason a budget pause holds.
const BUDGET_PAUSE_REASON: &str = "observed usage budget or unavailable measurement";

/// The budget decides on the usage standing: the budget and the two
/// totals of the usage report, one aggregate over the ledger, never the
/// whole report (#2340). A warning or a pause warns
/// once: the stored budget's `warned` becomes true (the payload written
/// again in its own key order) and the event `usage-warning` records the
/// decision and the totals. A pause then ends a running run as a pause
/// holding `budget-exhausted`, as `actor`.
///
/// # Errors
/// An edited budget (not an object, or without `warned`), or the store's.
pub(crate) fn apply_usage_budget(
    transaction: &(impl BoardRuns + BoardEvents + BoardUsage + ?Sized),
    clock: &dyn Clock,
    actor: &str,
) -> Result<BudgetEffect, BoardError> {
    budget_applied(transaction, clock, actor).map(|(effect, _)| effect)
}

/// [`apply_usage_budget`], answering also the standing after it: the
/// totals it decided on, which nothing it writes changes, and the budget
/// as stored now (read again after a warning rewrote it), for the
/// caller's control receipt (#2340).
///
/// # Errors
/// As [`apply_usage_budget`].
pub(crate) fn budget_applied(
    transaction: &(impl BoardRuns + BoardEvents + BoardUsage + ?Sized),
    clock: &dyn Clock,
    actor: &str,
) -> Result<(BudgetEffect, UsageStanding), BoardError> {
    let standing = transaction.usage_standing()?;
    let decision = budget_decision(&standing)?;
    let mut effect = BudgetEffect::Unchanged;
    if matches!(decision, UsageDecision::Warn | UsageDecision::Pause) {
        let Value::Object(mut budget) = standing.budget.clone() else {
            return Err(edited("usage budget"));
        };
        match budget.get("warned") {
            Some(Value::Bool(false)) => {
                let token_limit = budget.get("token_limit").cloned().unwrap_or(Value::Null);
                budget.insert("warned".to_owned(), Value::Bool(true));
                transaction.configure_usage_budget(&Value::Object(budget))?;
                transaction.event(
                    actor,
                    clock.now_seconds(),
                    "usage-warning",
                    &detail([
                        ("decision", Value::from(decision.as_str())),
                        ("observed_tokens", standing.observed_tokens.clone()),
                        ("token_limit", token_limit),
                        (
                            "unknown_usage_requests",
                            standing.unknown_usage_requests.clone(),
                        ),
                    ]),
                )?;
                effect = BudgetEffect::Warned;
            }
            Some(_) => {}
            None => return Err(edited("usage budget")),
        }
    }
    // A receipt read after a warning reports the stored payload as
    // `json.loads` reads it back; otherwise the budget is as it was read.
    let standing = match effect {
        BudgetEffect::Warned => UsageStanding {
            budget: transaction.usage_budget()?,
            ..standing
        },
        BudgetEffect::Unchanged | BudgetEffect::Paused => standing,
    };
    if decision == UsageDecision::Pause && running(transaction)? {
        end(
            transaction,
            clock,
            actor,
            "budget-exhausted",
            BUDGET_PAUSE_REASON,
        )?;
        effect = BudgetEffect::Paused;
    }
    Ok((effect, standing))
}

/// `tx.run()['status'] == 'running'`, read again in the transaction.
fn running(transaction: &(impl BoardRuns + ?Sized)) -> Result<bool, BoardError> {
    let run = current(transaction)?;
    Ok(run.status.as_ref() == Some(&RunState::RUNNING))
}

#[cfg(test)]
#[path = "board_usage_tests.rs"]
mod tests;
