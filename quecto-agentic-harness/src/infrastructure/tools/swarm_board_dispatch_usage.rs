//! The usage methods of the board dispatch (#2274): their Python
//! signatures and their serving. `usage_budget` answers the usage report,
//! `_record_request` the control receipt and `_request_admission` the run
//! and its members, each after the token budget applied. Every argument
//! reaches the use case as the JSON value passed. The decision names what
//! the budget did (`paused`, `warned`) and otherwise what the call did.
use serde_json::Value;

use super::control::{receipt, report};
use super::{Parameter, Served, float, member_row, object, required, take};
use crate::application::swarm::dto::{
    BudgetChange, BudgetEffect, ConfigureUsageBudgetRequest, RecordRequestUsageRequest,
    RequestDelivery,
};
use crate::application::swarm::use_cases::{
    ConfigureUsageBudget, ReadRequestAdmission, RecordRequestUsage,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// `usage_budget(token_limit, strict_unknown=True)`.
pub(super) const USAGE_BUDGET: [Parameter; 2] = [
    required("token_limit"),
    Parameter {
        name: "strict_unknown",
        default: Some(|| Value::Bool(true)),
    },
];
/// `_record_request(record)`.
pub(super) const RECORD_REQUEST: [Parameter; 1] = [required("record")];

/// The budget's effect as the decision, or `otherwise` when it had none.
fn decided(effect: BudgetEffect, otherwise: &'static str) -> &'static str {
    match effect {
        BudgetEffect::Paused => "paused",
        BudgetEffect::Warned => "warned",
        BudgetEffect::Unchanged => otherwise,
    }
}

pub(super) fn usage_budget(
    configure_usage_budget: &ConfigureUsageBudget,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [token_limit, strict_unknown] = take(arguments)?;
    let configured = configure_usage_budget.execute(ConfigureUsageBudgetRequest {
        actor: actor.to_owned(),
        token_limit,
        strict_unknown,
    })?;
    let otherwise = match configured.change {
        BudgetChange::Configured => "configured",
        BudgetChange::Unchanged => "unchanged",
    };
    Ok(Served {
        value: report(configured.report),
        decision: decided(configured.effect, otherwise),
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

pub(super) fn record_request(
    record_request_usage: &RecordRequestUsage,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [record] = take(arguments)?;
    let recorded = record_request_usage.execute(RecordRequestUsageRequest {
        actor: actor.to_owned(),
        record,
    })?;
    let otherwise = match recorded.delivery {
        RequestDelivery::Recorded => "recorded",
        RequestDelivery::Redelivered => "redelivered",
        RequestDelivery::Replaced => "replaced",
    };
    Ok(Served {
        value: receipt(recorded.receipt),
        decision: decided(recorded.effect, otherwise),
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

/// `_request_admission()`'s dict, in Python's key order.
pub(super) fn request_admission(
    read_request_admission: &ReadRequestAdmission,
    actor: &str,
) -> Result<Served, BoardError> {
    let admission = read_request_admission.execute(actor)?;
    let view = admission.view;
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    Ok(Served {
        value: object([
            ("status", text(view.status)),
            ("coordinator", text(view.coordinator)),
            ("deadline", float(view.deadline)?),
            (
                "members",
                Value::Array(view.members.into_iter().map(member_row).collect()),
            ),
            ("outcome", text(view.outcome)),
            ("control_generation", Value::from(view.control_generation)),
        ]),
        decision: decided(admission.effect, "read"),
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

#[cfg(test)]
#[path = "swarm_board_dispatch_usage_tests.rs"]
mod tests;
