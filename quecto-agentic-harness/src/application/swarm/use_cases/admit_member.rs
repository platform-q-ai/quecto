//! `Workbench._admit` (#2271): the harness reserves a member's place in the
//! run before launching it (a nested launch included). Never a board user's
//! call.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_membership::MEMBER_MAX_BYTES;
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{AdmissionDecision, AdmitMemberRequest, AdmittedMember};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind, admission, bounded};

/// Bounds the member id, then runs `Coordination.reserve_member` through
/// the operation gate (`active=False`): `admission` decides on the run, the
/// member's prior row, the reservation and the live-or-reserved usage, all
/// read in the one transaction that inserts the row, so concurrent
/// admissions never pass the member limit. A new admission is a `reserved`
/// row launched by the actor and the event `reserved`; a retry under the
/// same reservation (Python's `==`) writes nothing. Either way the answer
/// is the member's row as `dict(row)`. The reservation is the caller's
/// value, bound as given: a number is stored as its text, and a value
/// `sqlite3` cannot bind is refused only after admission decided.
pub struct AdmitMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl AdmitMember {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The member-id bound, an authorisation or admission refusal (Python's
    /// text), or the store's.
    pub fn execute(&self, request: AdmitMemberRequest) -> Result<AdmittedMember, BoardError> {
        admit(&*self.repository, &*self.clock, request)
    }
}

impl OverRepository for AdmitMember {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

/// `Workbench._admit` over `repository`: [`AdmitMember`]'s work, which
/// [`JoinRun`](super::JoinRun) runs too (#2303 reconcile): a use case
/// holds one repository and nests no other, so `over` meters the whole
/// call.
///
/// # Errors
/// As [`AdmitMember::execute`].
pub(super) fn admit(
    repository: &dyn BoardRepository,
    clock: &(dyn Clock + Send + Sync),
    request: AdmitMemberRequest,
) -> Result<AdmittedMember, BoardError> {
    let member = bounded(&request.member, "member", MEMBER_MAX_BYTES)?;
    let actor = request.actor.as_str();
    let reservation = &request.reservation;
    operation(
        repository,
        clock,
        actor,
        Access::default(),
        |transaction, run| {
            let prior = transaction.member(member)?;
            let usage = transaction.usage()?;
            let now = clock.now_seconds();
            let decision = if admission(run, prior.as_ref(), reservation, usage, now)? {
                transaction.reserve_member(member, reservation, actor)?;
                transaction.event(
                    actor,
                    clock.now_seconds(),
                    "reserved",
                    &detail([("member", text(member))]),
                )?;
                AdmissionDecision::Reserved
            } else {
                AdmissionDecision::Retry
            };
            // Admission answered for a row it either found or wrote.
            let row = transaction
                .member_row(&text(member), None)?
                .ok_or_else(|| {
                    BoardError::new(
                        RefusalKind::Internal,
                        "coordination store lost the member it admitted",
                    )
                })?;
            debug_assert_eq!(row.text("id"), Some(member), "the admitted member's row");
            Ok(AdmittedMember { row, decision })
        },
    )
}

#[cfg(test)]
#[path = "admit_member_tests.rs"]
mod tests;
