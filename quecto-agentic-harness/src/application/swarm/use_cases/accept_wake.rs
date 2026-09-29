//! `Workbench._accept_wake(generation)` (#2276): the receiver's check that
//! a wake hint it was sent still wakes it, claiming the generation once.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_wakes::{running, woken};
use crate::application::swarm::dto::{AcceptWakeRequest, WakeAccepted};
use crate::application::swarm::ports::{BoardRepository, BoardTransaction, Clock};
use crate::domain::swarm::{Access, BoardError, NotificationEvent, RefusalKind, RunRecord};

/// The generation (Python's `int`, at least 0) is checked before the
/// operation gate, a read-only one (`operation(active=False,
/// read_only=True)`). Then, as `claim_wake_events`: `wake_cursors` is
/// created inside the transaction (so a refused claim leaves none); a
/// generation ahead of the board is refused; a run that is not running
/// claims nothing and keeps the frontier, so the generation is honoured
/// after a resume; a generation at or below the caller's wake cursor
/// claims nothing; otherwise the other members' events up to it are
/// claimed and the cursor moves to it. The claimed events are judged from
/// each event's own actor's view (the empty actor, as Python's
/// `notification_targets(..., '', ...)`): whether they still wake the
/// caller.
pub struct AcceptWake {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

/// What a claim took: the events, and whether the cursor moved.
struct Claimed {
    events: Vec<NotificationEvent>,
    moved: bool,
}

/// Python's `type(generation) is int and generation >= 0`: a JSON
/// integer of at least 0, never a boolean or a float.
///
/// # Errors
/// `wake generation must be a nonnegative integer`.
fn wake_generation(value: &Value) -> Result<u64, BoardError> {
    match value {
        Value::Number(number) if number.is_u64() => number.as_u64(),
        Value::Number(_)
        | Value::Null
        | Value::Bool(_)
        | Value::String(_)
        | Value::Array(_)
        | Value::Object(_) => None,
    }
    .ok_or_else(|| {
        BoardError::new(
            RefusalKind::Invalid,
            "wake generation must be a nonnegative integer",
        )
    })
}

impl AcceptWake {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// `wake generation must be a nonnegative integer`, an authorisation
    /// refusal, `wake generation is ahead of the board`, the policy's
    /// `TypeError` for targets that do not sort, or the store's.
    pub fn execute(&self, request: AcceptWakeRequest) -> Result<WakeAccepted, BoardError> {
        let generation = wake_generation(&request.generation)?;
        let actor = request.actor.as_str();
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, run| {
                let claimed = claim(transaction, run, actor, generation)?;
                let members = woken(transaction, run, "", &claimed.events)?;
                Ok(WakeAccepted {
                    woken: members.iter().any(|row| row.text("id") == Some(actor)),
                    cursor_moved: claimed.moved,
                })
            },
        )
    }
}

/// `Transaction.claim_wake_events(actor, generation)`.
fn claim(
    transaction: &dyn BoardTransaction,
    run: &RunRecord,
    actor: &str,
    generation: u64,
) -> Result<Claimed, BoardError> {
    // Separate receiver frontier from each producer's delivery frontier.
    transaction.create_wake_cursors()?;
    let current = transaction.event_generation()?;
    let Some(generation) = i64::try_from(generation)
        .ok()
        .filter(|generation| *generation <= current)
    else {
        return Err(BoardError::new(
            RefusalKind::Invalid,
            "wake generation is ahead of the board",
        ));
    };
    // A paused run retains its wake frontier: the same generation is
    // honoured after resume instead of being silently consumed. Only a
    // running run claims, and only a generation past the cursor.
    if running(run) {
        let previous = transaction.wake_cursor(actor)?.unwrap_or(0);
        if generation > previous {
            let events = transaction.wake_events(previous, generation, actor)?;
            transaction.set_wake_cursor(actor, generation)?;
            return Ok(Claimed {
                events,
                moved: true,
            });
        }
    }
    Ok(Claimed {
        events: Vec::new(),
        moved: false,
    })
}

impl OverRepository for AcceptWake {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "accept_wake_tests.rs"]
mod tests;
