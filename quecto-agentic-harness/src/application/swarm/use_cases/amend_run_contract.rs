//! `Workbench.amend(goal, constraints, criteria, reason)` (#2273): the
//! coordinator replaces the run's contract, and every piece of criterion
//! evidence with it.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{AmendRunContractRequest, AmendedContract, StoredContract};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{Access, BoardError, RefusalKind, bounded, bounded_text, criteria};

/// The goal, the reason and the encoded constraints are bounded before the
/// operation gate, which admits only the coordinator (a running run); the
/// criteria are checked inside it, after authorisation (so a worker's
/// amend with bad criteria is refused as not the coordinator's). The
/// constraints are not required to be strings: only their encoding is
/// bounded, as Python bounds it. The contract is replaced, every evidence
/// row is deleted, and the event `amended` carries the previous and the
/// new goal, the reason, and the whole contract before and after.
pub struct AmendRunContract {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl AmendRunContract {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// A bound, an authorisation or budget refusal, a criteria refusal
    /// with Python's text, a stored contract that is not JSON, or the
    /// store's.
    ///
    /// Answers the contract as it now stands (#2394).
    pub fn execute(&self, request: AmendRunContractRequest) -> Result<StoredContract, BoardError> {
        let goal = bounded(&request.goal, "goal", TEXT_MAX_BYTES)?;
        let reason = bounded(&request.reason, "amendment reason", TEXT_MAX_BYTES)?;
        bounded_text(
            &self.encoding.encode(&request.constraints)?,
            "constraints",
            TEXT_MAX_BYTES,
        )?;
        let actor = request.actor.as_str();
        let coordinating = Access {
            active: true,
            coordinator: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            coordinating,
            |transaction, _| {
                criteria(
                    &request.criteria,
                    self.encoding.encode(&request.criteria)?.len(),
                )?;
                let Some(before) = transaction.run_contract()? else {
                    return Err(BoardError::new(
                        RefusalKind::RunMissing,
                        "coordination run missing",
                    ));
                };
                transaction.amend_contract(&AmendedContract {
                    goal: goal.to_owned(),
                    constraints: request.constraints.clone(),
                    criteria: request.criteria.clone(),
                })?;
                transaction.delete_all_evidence()?;
                let after = StoredContract {
                    goal: text(goal),
                    constraints: request.constraints.clone(),
                    criteria: request.criteria.clone(),
                };
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "amended",
                    &detail([
                        ("previous_goal", before.goal.clone()),
                        ("goal", text(goal)),
                        ("reason", text(reason)),
                        ("before", contract(before)),
                        ("after", contract(after.clone())),
                    ]),
                )?;
                Ok(after)
            },
        )
    }
}

/// `{'goal', 'constraints', 'criteria'}`.
fn contract(contract: StoredContract) -> Value {
    detail([
        ("goal", contract.goal),
        ("constraints", contract.constraints),
        ("criteria", contract.criteria),
    ])
}

impl OverRepository for AmendRunContract {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "amend_run_contract_tests.rs"]
mod tests;
