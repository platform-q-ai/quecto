//! `join_process` (#2271): the second half of `Workbench._bootstrap`, a
//! harness joining its container's run. (Python returns the coordinator's
//! `summary()`, a read model S12 adds.)
//!
//! A deliberate composition (#2271 round-1 review L3): `JoinRun` runs the
//! work of the `AdmitMember` and `ActivateMember` use cases because
//! Python's `join_process` calls exactly those two `Workbench` methods,
//! `_admit` and `_activate`, as the coordinator, each in its own
//! transaction. Running the same functions keeps their gates, refusals and
//! commits (an admission that commits before its activation is refused)
//! identical to Python's by construction, rather than restating them in a
//! third transaction. It runs them over its own repository (#2303
//! reconcile), not through nested use cases: a board use case holds one
//! repository, so serving it `over` a metered call's repository measures
//! all three transactions.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use super::activate_member::activate;
use super::admit_member::admit;
use crate::application::swarm::board_membership::{
    MEMBER_MAX_BYTES, holds_reservation, live, same_process,
};
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::dto::{
    ActivateMemberRequest, AdmitMemberRequest, JoinRunRequest, Joined,
};
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::{BoardError, RefusalKind, bounded, python_truthy};

/// Reads the run's coordinator and the member's row in one plain
/// transaction, then acts **as the coordinator** (the events' actor and the
/// new member's launcher): a new identity is admitted under the given
/// reservation, or a fresh one when the given one is falsy (Python's
/// `reservation or uuid4().hex`); the member's own live process returns at
/// once, carrying the coordinator for the summary S12 adds; any other
/// process must name the member's reservation (Python's `==`). Then the
/// coordinator activates it.
pub struct JoinRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
}

impl JoinRun {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
        }
    }

    /// # Errors
    /// `coordination run missing`, `launch reservation does not match
    /// invoking process`, an admission or activation refusal, or the
    /// store's.
    pub fn execute(&self, request: JoinRunRequest) -> Result<Joined, BoardError> {
        let member = Value::from(request.member.as_str());
        let (coordinator, existing) = atomic(&*self.repository, false, |transaction| {
            let Some(coordinator) = transaction.run_coordinator()? else {
                return Err(BoardError::new(
                    RefusalKind::RunMissing,
                    "coordination run missing",
                ));
            };
            Ok((coordinator, transaction.member_row(&member, None)?))
        })?;
        let joined = match &existing {
            None => Joined::Admitted,
            Some(row) if live(row) && same_process(row, &request.launch) => {
                return Ok(Joined::AlreadyLive { coordinator });
            }
            Some(row) if holds_reservation(row, &request.reservation) => Joined::Reactivated,
            Some(_) => {
                return Err(BoardError::new(
                    RefusalKind::LaunchConflict,
                    "launch reservation does not match invoking process",
                ));
            }
        };
        // Python's `reservation or uuid.uuid4().hex`, drawn before admitting.
        let admitting = match joined {
            Joined::Admitted if python_truthy(&request.reservation) => {
                Some(request.reservation.clone())
            }
            Joined::Admitted => Some(Value::String(self.ids.hex32())),
            Joined::AlreadyLive { .. } | Joined::Reactivated => None,
        };
        let Some(coordinator) = coordinator else {
            // Python acts as a `Workbench` whose member is `None`: `_admit`
            // bounds the member first, then the gate finds no invoking
            // member, as activation's gate does.
            if admitting.is_some() {
                bounded(&member, "member", MEMBER_MAX_BYTES)?;
            }
            return Err(BoardError::new(
                RefusalKind::NotMember,
                "invoking member is unknown or death confirmed",
            ));
        };
        if let Some(reservation) = &admitting {
            admit(
                &*self.repository,
                &*self.clock,
                AdmitMemberRequest {
                    actor: coordinator.clone(),
                    member: member.clone(),
                    reservation: reservation.clone(),
                },
            )?;
        }
        activate(
            &*self.repository,
            &*self.clock,
            ActivateMemberRequest {
                actor: coordinator,
                member,
                reservation: admitting.unwrap_or(request.reservation),
                launch: request.launch,
                socket: request.socket,
            },
        )?;
        Ok(joined)
    }
}

impl OverRepository for JoinRun {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            ids: self.ids.clone(),
        }
    }
}

#[cfg(test)]
#[path = "join_run_tests.rs"]
mod tests;
