//! How every board change is made (#2476): read the board, take the time,
//! build the change against what was read, and write it only if the board
//! is still at the head that was read. A write that lost the race is read
//! again and the change rebuilt against the fresh board, at most
//! [`MAX_WRITE_ATTEMPTS`] times; a change no longer valid is refused.
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardHead, BoardSnapshot, BoardWrite, ProjectBoardStore,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::CLOCK_AHEAD_SECONDS;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// How many times a change is tried before the board is reported contended.
pub const MAX_WRITE_ATTEMPTS: usize = 3;

/// This machine's time, refused when it runs more than
/// [`CLOCK_AHEAD_SECONDS`] ahead of the clock's outside reference.
pub(super) fn checked_now(clock: &dyn BoardClock) -> Result<Timestamp, ProjectBoardError> {
    let now = clock.now()?;
    match clock.reference() {
        Some(reference) => {
            let latest = reference
                .plus_seconds(CLOCK_AHEAD_SECONDS)
                .map_err(ProjectBoardError::Invalid)?;
            match now <= latest {
                true => Ok(now),
                false => Err(ProjectBoardError::ClockAhead { now, reference }),
            }
        }
        None => Ok(now),
    }
}

/// Writes the tasks `change` builds from the board and the time, retried
/// against a fresh board when another writer moved it first.
pub(super) fn commit(
    store: &dyn ProjectBoardStore,
    clock: &dyn BoardClock,
    message: &str,
    mut change: impl FnMut(&BoardSnapshot, &Timestamp) -> Result<Vec<Task>, ProjectBoardError>,
) -> Result<(Vec<Task>, BoardHead), ProjectBoardError> {
    for _ in 0..MAX_WRITE_ATTEMPTS {
        let snapshot = store.read()?;
        let now = checked_now(clock)?;
        let tasks = change(&snapshot, &now)?;
        assert!(!tasks.is_empty(), "a board change writes at least one task");
        match store.write(&snapshot.head, &tasks, message)? {
            BoardWrite::Written(head) => return Ok((tasks, head)),
            BoardWrite::Lost => break,
            BoardWrite::Refused(reason) => return Err(ProjectBoardError::Refused(reason)),
        }
    }
    Err(ProjectBoardError::Contended {
        attempts: MAX_WRITE_ATTEMPTS,
    })
}
