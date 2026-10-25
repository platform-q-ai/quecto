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
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::run::Run;
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
pub(super) fn checked_now(clock: &dyn BoardClock) -> Result<When, ProjectBoardError> {
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
/// first. Every change is about one task ([`one_tasks_change`]): it
/// writes that task first ([`written_task`]), or only other artifacts of
/// it (a plan, [`written_plan`]); a run's end writes its run, after its
/// task when the end moves it ([`commit_run`]). Given
/// `facts` (epic #2481 T4), it notes the attempts it began, how many lost
/// the race and how long it paused between them; given none, it notes
/// nothing and reads no monotonic time.
pub(super) fn commit(
    store: &dyn ProjectBoardStore,
    clock: &dyn BoardClock,
    message: &str,
    facts: Option<&mut OpFacts>,
    change: impl FnMut(&BoardSnapshot, &When) -> Result<Vec<BoardArtifact>, ProjectBoardError>,
) -> Result<Vec<BoardArtifact>, ProjectBoardError> {
    shaped(Writes::OneTask, store, clock, message, facts, change)
}

/// [`commit`] of a run's end (a fold-back, an end_run): its run, after
/// its task when the end moves it (#2705 T5), and nothing else.
pub(super) fn commit_run(
    store: &dyn ProjectBoardStore,
    clock: &dyn BoardClock,
    message: &str,
    facts: Option<&mut OpFacts>,
    change: impl FnMut(&BoardSnapshot, &When) -> Result<Vec<BoardArtifact>, ProjectBoardError>,
) -> Result<Vec<BoardArtifact>, ProjectBoardError> {
    shaped(Writes::OneRun, store, clock, message, facts, change)
}

/// What a change writes, as its use case declares it.
#[derive(Debug, Clone, Copy)]
enum Writes {
    /// One task's change ([`one_tasks_change`]).
    OneTask,
    /// One run, after its task when its end moves it, and nothing else.
    OneRun,
}

impl Writes {
    /// Whether `artifacts` are written as declared (an allowlist).
    fn holds(self, artifacts: &[BoardArtifact]) -> bool {
        match (self, artifacts) {
            (Self::OneTask, _) => one_tasks_change(artifacts),
            (Self::OneRun, [BoardArtifact::Run(_)]) => true,
            (Self::OneRun, [BoardArtifact::Task(task), BoardArtifact::Run(run)]) => {
                task.fields().id == run.fields().task
            }
            (Self::OneRun, _) => false,
        }
    }

    /// What a change breaks when it writes otherwise.
    fn rule(self) -> &'static str {
        match self {
            Self::OneTask => {
                "a board change writes its task first, or only one task's other artifacts"
            }
            Self::OneRun => {
                "a board change writes as it declares: one run, after its task when it moves"
            }
        }
    }
}

fn shaped(
    writes: Writes,
    store: &dyn ProjectBoardStore,
    clock: &dyn BoardClock,
    message: &str,
    facts: Option<&mut OpFacts>,
    mut change: impl FnMut(&BoardSnapshot, &When) -> Result<Vec<BoardArtifact>, ProjectBoardError>,
) -> Result<Vec<BoardArtifact>, ProjectBoardError> {
    let mut facts = facts;
    // The store's count before this change, so its own races are told.
    let races = facts.as_ref().map(|_| store.lock_races());
    let mut written = None;
    let result = retried(clock, facts.as_deref_mut(), || {
        let snapshot = store.read()?;
        let when = checked_now(clock)?;
        let artifacts = change(&snapshot, &when)?;
        assert!(writes.holds(&artifacts), "{}", writes.rule());
        match store.write(&snapshot.head, &artifacts, message)? {
            BoardWrite::Written(head) => {
                assert_ne!(
                    head, snapshot.head,
                    "a write the board holds moves its head"
                );
                written = Some(head);
                Ok(Attempt::Done(artifacts))
            }
            BoardWrite::Lost => Ok(Attempt::Lost),
            BoardWrite::Refused(reason) => Err(ProjectBoardError::Refused(reason)),
        }
    });
    if let (Some(facts), Some(before)) = (facts, races) {
        note_lock_races(store, before, facts);
        facts.head = written;
    }
    result
}

/// Notes in `facts` the lock races `store` met since it counted `before`
/// (#2663): only when it met one, as a record tells it.
pub(super) fn note_lock_races(store: &dyn ProjectBoardStore, before: u64, facts: &mut OpFacts) {
    let met = store.lock_races().saturating_sub(before);
    facts.lock_races = (met > 0).then_some(met);
}

/// Whether `artifacts` are one task's change (an allowlist): that task
/// first, then only plans, runs, changes and reviews of it; or, writing
/// no task, only plans, runs, changes and reviews of one and the same
/// task. A second task, or a discussion (keyed by its own id, and maybe
/// about no task yet: the slice that writes discussions decides their
/// shape), is no one task's change, nor is nothing.
pub(super) fn one_tasks_change(artifacts: &[BoardArtifact]) -> bool {
    let task_of = |artifact: &BoardArtifact| match artifact {
        BoardArtifact::Plan(plan) => Some(plan.fields().task.clone()),
        BoardArtifact::Run(run) => Some(run.fields().task.clone()),
        BoardArtifact::Change(change) => Some(change.fields().task.clone()),
        BoardArtifact::Review(review) => Some(review.fields().task.clone()),
        BoardArtifact::Task(_) | BoardArtifact::Discussion(_) => None,
    };
    let (task, rest) = match artifacts.split_first() {
        Some((BoardArtifact::Task(task), rest)) => (Some(task.fields().id.clone()), rest),
        Some((first, rest)) => (task_of(first), rest),
        None => return false,
    };
    task.is_some_and(|task| {
        rest.iter()
            .all(|other| task_of(other).as_ref() == Some(&task))
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

/// What a run's end writes ([`commit_run`]): its task first, when the end
/// `moved` it, then its `run`.
pub(super) fn run_end_writes(run: Run, moved: Option<Task>) -> Vec<BoardArtifact> {
    let task = moved.map(BoardArtifact::Task);
    task.into_iter().chain([BoardArtifact::Run(run)]).collect()
}

/// The run a run's end wrote, and its task when the end moved it:
/// [`commit_run`] holds it to those.
pub(super) fn written_run_end(artifacts: &[BoardArtifact]) -> (Option<&Task>, &Run) {
    match artifacts {
        [BoardArtifact::Run(run)] => (None, run),
        [BoardArtifact::Task(task), BoardArtifact::Run(run)] => (Some(task), run),
        other => panic!("a run's end writes its run, after its task when it moves, not {other:?}"),
    }
}

/// The plan a plan's change wrote: its one artifact.
pub(super) fn written_plan(artifacts: Vec<BoardArtifact>) -> Plan {
    match artifacts.as_slice() {
        [BoardArtifact::Plan(plan)] => plan.clone(),
        other => panic!("a plan's change writes its plan alone, not {other:?}"),
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

#[cfg(test)]
#[path = "board_change_tests.rs"]
mod tests;
