//! `Workbench.create` (#2270, #2277): a member creates the run and becomes
//! its coordinator, and answers with the summary.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{atomic, detail, text};
use crate::application::swarm::board_read_models::summary;
use crate::application::swarm::dto::{
    CreateBranch, CreateRunRequest, CreatedRun, NewMember, NewRun, RunContract, RunOwnerRow,
};
use crate::application::swarm::ports::{
    BoardEncoding, BoardMembers, BoardRepository, BoardRuns, Clock, IdSource,
};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{
    BoardError, MemberState, RefusalKind, RunState, bounded, bounded_text, criteria,
};

/// The most members a run may hold, the coordinator included.
const MEMBER_LIMIT_MAX: i64 = 25;
/// How far ahead a deadline may be: seven days.
const DEADLINE_HORIZON_SECONDS: f64 = 604_800.0;

/// Validates the contract in Python's order, then in one creating
/// transaction either takes over the setup placeholder (only its
/// coordinator may, and only while the live and reserved members fit the
/// new limit) or inserts a fresh run and the creator's live member row,
/// drawing the run id before the member's reservation. Either way the event
/// `created` records the contract as given. Once that transaction has
/// committed, the creator's summary is read (`board_read_models`), which
/// can still refuse: a created run answered with a refusal, as Python's
/// `create` answers (#2307).
pub struct CreateRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
    encoding: Arc<dyn BoardEncoding>,
}

impl CreateRun {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
            encoding,
        }
    }

    /// # Errors
    /// A validation refusal with Python's text, the refusal to create over
    /// an existing run, or the store's.
    pub fn execute(&self, request: CreateRunRequest) -> Result<CreatedRun, BoardError> {
        let contract = self.validated(&request)?;
        let member = request.member.as_str();
        let branch = atomic(&*self.repository, true, |transaction| {
            let branch = match transaction.run_owner()? {
                Some(existing) => {
                    take_over_setup(transaction, member, &existing, &contract)?;
                    CreateBranch::OverSetup
                }
                None => {
                    self.insert_fresh(transaction, member, &contract)?;
                    CreateBranch::Fresh
                }
            };
            transaction.event(
                member,
                self.clock.now_seconds(),
                "created",
                &detail([
                    ("goal", text(&contract.goal)),
                    ("deadline", request.deadline.clone()),
                    (
                        "contract",
                        detail([
                            ("goal", text(&contract.goal)),
                            ("constraints", request.constraints.clone()),
                            ("criteria", request.criteria.clone()),
                        ]),
                    ),
                ]),
            )?;
            Ok(branch)
        })?;
        let summary = summary(&*self.repository, &*self.clock, Some(member), None);
        Ok(CreatedRun { branch, summary })
    }

    /// `create`'s checks, in Python's order: the goal, the encoded
    /// constraints, the criteria, the constraints' shape, the member limit
    /// and the deadline window (read on the clock, as Python reads
    /// `time.time()`: the upper bound only once the lower one holds).
    fn validated(&self, request: &CreateRunRequest) -> Result<RunContract, BoardError> {
        let goal = bounded(&request.goal, "goal", TEXT_MAX_BYTES)?;
        bounded_text(
            &self.encoding.encode(&request.constraints)?,
            "constraints",
            TEXT_MAX_BYTES,
        )?;
        criteria(
            &request.criteria,
            self.encoding.encode(&request.criteria)?.len(),
        )?;
        let strings = match &request.constraints {
            Value::Array(items) => items.iter().all(Value::is_string),
            _ => false,
        };
        if !strings {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "constraints must be a list of strings",
            ));
        }
        let member_limit = match request.member_limit.as_i64() {
            Some(limit @ 1..=MEMBER_LIMIT_MAX) => limit,
            _ => {
                return Err(BoardError::new(
                    RefusalKind::Invalid,
                    "member limit must be 1 through 25 including coordinator",
                ));
            }
        };
        let deadline = self.deadline(&request.deadline)?;
        Ok(RunContract {
            goal: goal.to_owned(),
            constraints: request.constraints.clone(),
            criteria: request.criteria.clone(),
            member_limit,
            deadline,
        })
    }

    /// `isinstance(deadline, (int, float))` (a boolean is an int) and
    /// `time.time() < deadline <= time.time() + 604800`.
    fn deadline(&self, deadline: &Value) -> Result<f64, BoardError> {
        let number = match deadline {
            Value::Number(number) => number.as_f64(),
            Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
            _ => None,
        };
        let within = number.filter(|&deadline| {
            self.clock.now_seconds() < deadline
                && deadline <= self.clock.now_seconds() + DEADLINE_HORIZON_SECONDS
        });
        within.ok_or_else(|| {
            BoardError::new(
                RefusalKind::Invalid,
                "deadline must be in the next seven days",
            )
        })
    }

    /// No run yet: the run under a fresh id, then the creator's live row
    /// under a fresh reservation, drawn in that order.
    fn insert_fresh(
        &self,
        transaction: &(impl BoardRuns + BoardMembers + ?Sized),
        member: &str,
        contract: &RunContract,
    ) -> Result<(), BoardError> {
        transaction.insert_run(&NewRun {
            id: self.ids.hex32(),
            contract: contract.clone(),
            coordinator: member.to_owned(),
            integrator: member.to_owned(),
            status: RunState::RUNNING,
        })?;
        transaction.insert_member(&NewMember {
            id: member.to_owned(),
            reservation: self.ids.hex32(),
            status: MemberState::LIVE,
            pid: Value::Null,
            started: Value::Null,
            socket: Value::Null,
            launcher: None,
        })
    }
}

/// An existing run: only the setup placeholder's coordinator creates over
/// it, and only when the members not confirmed dead fit the new limit.
fn take_over_setup(
    transaction: &(impl BoardRuns + BoardMembers + ?Sized),
    member: &str,
    existing: &RunOwnerRow,
    contract: &RunContract,
) -> Result<(), BoardError> {
    let placeholder = existing.status.as_deref() == Some(RunState::SETUP.as_str())
        && existing.coordinator.as_deref() == Some(member);
    if !placeholder {
        return Err(BoardError::new(
            RefusalKind::RunExists,
            "only the setup coordinator can create this run; existing runs cannot be reset",
        ));
    }
    if transaction.not_dead()? > contract.member_limit {
        return Err(BoardError::new(
            RefusalKind::MemberLimit,
            "existing live/reserved members exceed requested limit; terminate and reconcile first",
        ));
    }
    transaction.update_run_contract(contract)
}

impl OverRepository for CreateRun {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            ids: self.ids.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "create_run_tests.rs"]
mod tests;
