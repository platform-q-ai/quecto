//! `Workbench._confirmed_dead` (#2277, #1961): the harness that launched a
//! member reaped its owned process.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_loss::{ABRUPT_BLOCKER, ORDERLY_BLOCKER, end_by_loss};
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{ConfirmMemberDeadRequest, DeathConfirmation, DeathConfirmed};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{
    Access, BoardError, MemberExit, RefusalKind, python_equal, status_is_alive,
};

/// The reason a confirmed death that retains reservations records.
const ORPHANS: &str = "abrupt harness exit; orphaned tool processes may still write reserved paths";

/// The exit kind is checked before the operation gate (`active=False`),
/// as Python checks it before its operation. The member is dead and its
/// active tasks block for the coordinator's `recover`; a member unknown,
/// or not alive, is left as it is. An orderly exit (the member's own
/// teardown ended its tool process groups) releases its reservations; an
/// abrupt one retains them, and says so, for the coordinator to free.
/// Recorded as `death_confirmed`; the coordinator's death ends the run by
/// loss, any other member's keeps it going.
pub struct ConfirmMemberDead {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ConfirmMemberDead {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// `exit kind must be orderly or abrupt`, an authorisation refusal, or
    /// the store's (a member it cannot bind included).
    pub fn execute(&self, request: ConfirmMemberDeadRequest) -> Result<DeathConfirmed, BoardError> {
        let exit = exit_kind(&request.exit)?;
        let actor = request.actor.as_str();
        let member = &request.member;
        let clock = &*self.clock;
        operation(
            &*self.repository,
            clock,
            actor,
            Access::default(),
            |transaction, run| {
                let decided = |decision| DeathConfirmed {
                    decision,
                    exit,
                    run_status: run.status.clone(),
                };
                let alive = transaction
                    .member_status(member)?
                    .is_some_and(|row| status_is_alive(row.status.as_deref()));
                if !alive {
                    return Ok(decided(DeathConfirmation::AlreadyDead));
                }
                transaction.mark_member_dead(member)?;
                let blocker = match exit {
                    MemberExit::Orderly => ORDERLY_BLOCKER,
                    MemberExit::Abrupt => ABRUPT_BLOCKER,
                };
                transaction.block_owned_tasks(member, blocker)?;
                let mut retained = transaction.owner_file_count(member)?;
                debug_assert!(retained >= 0, "a count is never negative");
                if exit == MemberExit::Orderly {
                    transaction.delete_owner_files(member)?;
                    retained = 0;
                }
                let mut recorded = detail([
                    ("member", member.clone()),
                    ("exit", text(exit.as_str())),
                    ("reservations_retained", Value::from(retained)),
                ]);
                if retained != 0 {
                    recorded["reason"] = text(ORPHANS);
                }
                transaction.event(actor, clock.now_seconds(), "death_confirmed", &recorded)?;
                let named = run.coordinator.as_deref().map_or(Value::Null, text);
                let coordinator = python_equal(member, &named);
                let ended_by_loss = coordinator
                    && end_by_loss(
                        transaction,
                        clock,
                        actor,
                        run,
                        "coordinator death confirmed",
                    )?;
                Ok(decided(DeathConfirmation::Confirmed {
                    coordinator,
                    reservations_retained: u64::try_from(retained).unwrap_or(0),
                    ended_by_loss,
                }))
            },
        )
    }
}

/// Python's `exit not in ('orderly', 'abrupt')`: only that text is a kind.
fn exit_kind(exit: &Value) -> Result<MemberExit, BoardError> {
    match exit.as_str() {
        Some("orderly") => Ok(MemberExit::Orderly),
        Some("abrupt") => Ok(MemberExit::Abrupt),
        _ => Err(BoardError::new(
            RefusalKind::Invalid,
            "exit kind must be orderly or abrupt",
        )),
    }
}

impl OverRepository for ConfirmMemberDead {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "confirm_member_dead_tests.rs"]
mod tests;
