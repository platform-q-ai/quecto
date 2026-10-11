//! How every board change is made (#2476): read the board, take the time,
//! build the change against what was read, and write it only if the board
//! is still at the head that was read. A write that lost the race is read
//! again and the change rebuilt against the fresh board, with a short
//! jittered pause before each new attempt, for at most
//! [`MAX_WRITE_ATTEMPTS`] attempts in all; a change no longer valid is
//! refused.
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardHead, BoardSnapshot, BoardWrite, ProjectBoardStore,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::CLOCK_AHEAD_SECONDS;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// How many attempts, in all, a change gets before the board is reported
/// contended.
pub const MAX_WRITE_ATTEMPTS: usize = 3;

/// The time a change is made at: this machine's `now`, and the latest
/// time it may build on (the outside reference plus
/// [`CLOCK_AHEAD_SECONDS`]), when the clock has a reference.
pub(super) struct When {
    pub now: Timestamp,
    pub latest: Option<Timestamp>,
}

impl When {
    /// Refuses to build on `task` when a clock ran ahead stamped it past
    /// the latest time: its claims would hold everyone up.
    pub fn may_build_on(&self, task: &Task) -> Result<(), ProjectBoardError> {
        match &self.latest {
            Some(latest) if task.fields().updated > *latest => Err(ProjectBoardError::TaskAhead {
                id: task.fields().id.clone(),
                updated: task.fields().updated.clone(),
            }),
            Some(_) | None => Ok(()),
        }
    }
}

/// This machine's time, refused when it runs more than
/// [`CLOCK_AHEAD_SECONDS`] ahead of the clock's outside reference.
fn checked_now(clock: &dyn BoardClock) -> Result<When, ProjectBoardError> {
    let now = clock.now()?;
    let Some(reference) = clock.reference() else {
        return Ok(When { now, latest: None });
    };
    let latest = reference
        .plus_seconds(CLOCK_AHEAD_SECONDS)
        .map_err(|_| ProjectBoardError::ClockOutOfRange(reference.clone()))?;
    match now <= latest {
        true => Ok(When {
            now,
            latest: Some(latest),
        }),
        false => Err(ProjectBoardError::ClockAhead { now, reference }),
    }
}

/// Writes the tasks `change` builds from the board and the time, retried
/// against a fresh board when another writer moved it first.
pub(super) fn commit(
    store: &dyn ProjectBoardStore,
    clock: &dyn BoardClock,
    message: &str,
    mut change: impl FnMut(&BoardSnapshot, &When) -> Result<Vec<Task>, ProjectBoardError>,
) -> Result<(Vec<Task>, BoardHead), ProjectBoardError> {
    for attempt in 1..=MAX_WRITE_ATTEMPTS {
        if attempt > 1 {
            clock.pause(attempt);
        }
        let snapshot = store.read()?;
        let when = checked_now(clock)?;
        let tasks = change(&snapshot, &when)?;
        assert!(!tasks.is_empty(), "a board change writes at least one task");
        match store.write(&snapshot.head, &tasks, message)? {
            BoardWrite::Written(head) => return Ok((tasks, head)),
            BoardWrite::Lost => continue,
            BoardWrite::Refused(reason) => return Err(ProjectBoardError::Refused(reason)),
        }
    }
    Err(ProjectBoardError::Contended {
        attempts: MAX_WRITE_ATTEMPTS,
    })
}

/// The task `id` on the board as read: a quarantined file of that id is
/// told apart from no task at all.
pub(super) fn task_on<'b>(
    board: &'b BoardSnapshot,
    id: &crate::domain::project_board::value_objects::slug::Slug,
) -> Result<&'b Task, ProjectBoardError> {
    if let Some(task) = board.tasks.iter().find(|task| task.fields().id == *id) {
        return Ok(task);
    }
    match board
        .quarantined
        .iter()
        .any(|file| file.id.as_ref() == Some(id))
    {
        true => Err(ProjectBoardError::Quarantined(id.clone())),
        false => Err(ProjectBoardError::NotFound(id.clone())),
    }
}
