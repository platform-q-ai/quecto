//! How every board change is made (#2476): read the board, take the time,
//! build the change against what was read, and write it only if the board
//! is still at the head that was read. A write that lost the race is read
//! again and the change rebuilt against the fresh board, with a short
//! jittered pause before each new attempt, for at most
//! [`MAX_WRITE_ATTEMPTS`] attempts in all; a change no longer valid is
//! refused. The pause blocks the calling thread ([`BoardClock::pause`]),
//! as the store's calls do: callers run a change off the async workers.
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardSnapshot, BoardWrite, ProjectBoardStore,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::{CLOCK_AHEAD_SECONDS, within_slack};
use crate::domain::project_board::services::plans::Moment;
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
    /// the latest time plus [`STAMP_SLACK_SECONDS`](crate::domain::project_board::services::claims::STAMP_SLACK_SECONDS) (the room the
    /// strictly-forward stamp needs): its claims would hold everyone up.
    /// The bound is inclusive, as the clock's own is.
    pub fn may_build_on(&self, task: &Task) -> Result<(), ProjectBoardError> {
        let updated = &task.fields().updated;
        match within_slack(updated, self.latest.as_ref()) {
            true => Ok(()),
            false => Err(ProjectBoardError::TaskAhead {
                id: task.fields().id.clone(),
                updated: updated.clone(),
            }),
        }
    }
}

/// This time as the domain's plan rules read it: the one place a
/// [`Moment`] is made from a [`When`].
impl From<&When> for Moment {
    fn from(when: &When) -> Self {
        Moment {
            now: when.now.clone(),
            latest: when.latest.clone(),
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

/// Writes the artifacts `change` builds from the board and the time, in
/// one change, retried against a fresh board when another writer moved it
/// first. Every change writes its task first ([`written_task`]). Given
/// `facts` (epic #2481 T4), it notes the attempts it began, how many lost
/// the race and how long it paused between them; given none, it notes
/// nothing and reads no monotonic time.
pub(super) fn commit(
    store: &dyn ProjectBoardStore,
    clock: &dyn BoardClock,
    message: &str,
    facts: Option<&mut OpFacts>,
    mut change: impl FnMut(&BoardSnapshot, &When) -> Result<Vec<BoardArtifact>, ProjectBoardError>,
) -> Result<Vec<BoardArtifact>, ProjectBoardError> {
    retried(clock, facts, || {
        let snapshot = store.read()?;
        let when = checked_now(clock)?;
        let artifacts = change(&snapshot, &when)?;
        assert!(
            matches!(artifacts.first(), Some(BoardArtifact::Task(_))),
            "a board change writes its task first"
        );
        match store.write(&snapshot.head, &artifacts, message)? {
            BoardWrite::Written(head) => {
                assert_ne!(
                    head, snapshot.head,
                    "a write the board holds moves its head"
                );
                Ok(Attempt::Done(artifacts))
            }
            BoardWrite::Lost => Ok(Attempt::Lost),
            BoardWrite::Refused(reason) => Err(ProjectBoardError::Refused(reason)),
        }
    })
}

/// What one attempt at a write came to.
pub(super) enum Attempt<T> {
    /// The board holds it.
    Done(T),
    /// Another writer moved the board first: try again on a fresh read.
    Lost,
}

/// Runs `attempt` until it is done, for at most [`MAX_WRITE_ATTEMPTS`]
/// attempts in all, pausing before each new one (the board is reported
/// contended when every one lost). Given `facts` (epic #2481 T4), it notes
/// the attempts it began, how many lost the race and how long it paused
/// between them; given none, it notes nothing and reads no monotonic time.
pub(super) fn retried<T>(
    clock: &dyn BoardClock,
    mut facts: Option<&mut OpFacts>,
    mut attempt: impl FnMut() -> Result<Attempt<T>, ProjectBoardError>,
) -> Result<T, ProjectBoardError> {
    if let Some(facts) = facts.as_deref_mut() {
        facts.lost = Some(0);
    }
    for number in 1..=MAX_WRITE_ATTEMPTS {
        if number > 1 {
            paused(clock, number, facts.as_deref_mut());
        }
        if let Some(facts) = facts.as_deref_mut() {
            facts.attempts = Some(u64::try_from(number).unwrap_or(u64::MAX));
        }
        match attempt()? {
            Attempt::Done(done) => return Ok(done),
            Attempt::Lost => {
                if let Some(facts) = facts.as_deref_mut() {
                    facts.lost = Some(facts.lost.unwrap_or(0).saturating_add(1));
                }
            }
        }
    }
    Err(ProjectBoardError::Contended {
        attempts: MAX_WRITE_ATTEMPTS,
    })
}

/// The pause before attempt `attempt`, timed on the clock's monotonic
/// time into `facts` only when there are facts to note.
fn paused(clock: &dyn BoardClock, attempt: usize, facts: Option<&mut OpFacts>) {
    let Some(facts) = facts else {
        clock.pause(attempt);
        return;
    };
    let before = clock.elapsed();
    clock.pause(attempt);
    let pause = clock.elapsed().saturating_sub(before);
    let ms = u64::try_from(pause.as_millis()).unwrap_or(u64::MAX);
    facts.pause_ms = Some(facts.pause_ms.unwrap_or(0).saturating_add(ms));
}

/// The task a change wrote: [`commit`] holds every change to writing its
/// task first.
pub(super) fn written_task(artifacts: Vec<BoardArtifact>) -> Task {
    match artifacts.into_iter().next() {
        Some(BoardArtifact::Task(task)) => task,
        other => panic!("a board change writes its task first, not {other:?}"),
    }
}

/// `views` as a rule takes them: owned, in the view's order.
pub(super) fn owned<T: Clone>(views: Vec<&T>) -> Vec<T> {
    views.into_iter().cloned().collect()
}

/// The identity the store commits as, checked once when a use case is
/// built: an invalid one is refused before any change is tried.
pub(super) fn checked_writer(store: &dyn ProjectBoardStore) -> Result<Identity, ProjectBoardError> {
    let writer = store.writer();
    writer
        .validate("writer")
        .map_err(ProjectBoardError::InvalidWriter)?;
    Ok(writer)
}

/// The task `id` on the board as read: a quarantined file of that id is
/// told apart from no task at all ([`super::board_view::task`]).
pub(super) fn task_on<'b>(
    board: &'b BoardSnapshot,
    id: &crate::domain::project_board::value_objects::slug::Slug,
) -> Result<&'b Task, ProjectBoardError> {
    super::board_view::task(board, id)
}
