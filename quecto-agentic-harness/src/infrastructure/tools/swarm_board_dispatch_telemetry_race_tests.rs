//! #2277 final review L2: `_bootstrap` (placeholder, then the join) and
//! `_join` (admission, then activation) commit before their closing
//! summary, so a refusal met after those commits is recorded as
//! committed, with the branch taken so far and, for `_bootstrap`, whether
//! it wrote the placeholder. The race is another connection's write
//! landing between the op's transactions.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{ActorRefs, BoardTelemetry, Recorded, SwarmBoardHandles, call, create_args};
use super::{location, only, plain};
use crate::application::swarm::dto::CallMeasure;
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository, BoardWork, MeteredCall};
use crate::domain::swarm::{BoardError, BoardOpDetail, BoardOpOutcome, RefusalKind};
use crate::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;

/// A call's repository that runs `race` on the board file once, before
/// its `nth` transaction (from 1), as another connection's commit landing
/// between the op's own.
struct Racing {
    repository: Arc<dyn BoardRepository>,
    database: PathBuf,
    nth: usize,
    race: &'static str,
    begun: Mutex<usize>,
}

impl BoardRepository for Racing {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        let begun = {
            let mut begun = self.begun.lock().unwrap();
            *begun += 1;
            *begun
        };
        if begun == self.nth {
            rusqlite::Connection::open(&self.database)
                .unwrap()
                .execute_batch(self.race)
                .unwrap();
        }
        self.repository.atomic(create, work)
    }
}

impl MeteredCall for Racing {
    fn measure(&self) -> Option<CallMeasure> {
        None
    }
}

/// Opens a [`Racing`] repository for each call.
struct RacingMeter {
    repository: Arc<dyn BoardRepository>,
    database: PathBuf,
    nth: usize,
    race: &'static str,
}

impl BoardCallMeter for RacingMeter {
    fn open(&self) -> Arc<dyn MeteredCall> {
        Arc::new(Racing {
            repository: self.repository.clone(),
            database: self.database.clone(),
            nth: self.nth,
            race: self.race,
            begun: Mutex::new(0),
        })
    }
}

/// The record of `member`'s `method` with `args` on the board in `dir`
/// (set up by `setup` over plain handles), with `race` run before the
/// op's `nth` transaction; the call must refuse.
fn raced(
    setup: fn(&SwarmBoardHandles),
    member: &str,
    method: &str,
    args: Value,
    (nth, race): (usize, &'static str),
) -> crate::domain::swarm::BoardOpObservation {
    let dir = tempfile::TempDir::new().unwrap();
    let repository: Arc<dyn BoardRepository> =
        Arc::new(SqliteBoardRepository::new(&location(&dir)));
    setup(&plain(repository.clone()));
    let log = Arc::new(Recorded::default());
    let handles = SwarmBoardHandles {
        telemetry: Some(BoardTelemetry {
            log: log.clone(),
            meter: Arc::new(RacingMeter {
                repository: repository.clone(),
                database: location(&dir).database,
                nth,
                race,
            }),
            actors: Arc::new(ActorRefs::default()),
        }),
        ..plain(repository)
    };
    let refusal = call(&handles, member, method, args).unwrap_err();
    let recorded = only(&log);
    assert_eq!(
        recorded.result_bytes, 0,
        "{method} before {nth}: {refusal:?}"
    );
    recorded
}

/// A running run coordinated by `parent`.
fn running(handles: &SwarmBoardHandles) {
    call(handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    call(handles, "parent", "create_run", create_args()).unwrap();
}

/// A running run with `worker` joined in process 7 started at `t`.
fn joined(handles: &SwarmBoardHandles) {
    running(handles);
    call(handles, "worker", "_join", json!(["res-w", 7, "t", null])).unwrap();
}

fn nothing(_: &SwarmBoardHandles) {}

const PAUSE: &str = "UPDATE run SET status='paused'";
const COORDINATOR_GONE: &str = "DELETE FROM members WHERE id='parent'";
const WORKER: [&str; 2] = ["worker", "_join"];

fn refused(kind: RefusalKind, committed: bool) -> BoardOpOutcome {
    BoardOpOutcome::Refused { kind, committed }
}

/// `_join`'s admission commits before its activation: a pause landing
/// before the admission is a plain refusal; landing between the two, it
/// refuses the activation after the admission; and a coordinator gone
/// after the activation refuses the summary after both.
#[test]
fn a_join_refused_after_its_admission_is_recorded_committed() {
    let [member, method] = WORKER;
    let join = json!(["res-w", 7, "t", null]);
    for (nth, race, kind, committed) in [
        (3, PAUSE, RefusalKind::NotRunning, false),
        (4, PAUSE, RefusalKind::NotRunning, true),
        (5, COORDINATOR_GONE, RefusalKind::NotMember, true),
    ] {
        let recorded = raced(running, member, method, join.clone(), (nth, race));
        assert_eq!(
            (recorded.outcome, recorded.decision.as_deref()),
            (refused(kind, committed), committed.then_some("admitted")),
            "{race} before {nth}"
        );
    }
}

/// The join's already-live branch writes nothing, so a summary refused
/// after it is a plain refusal.
#[test]
fn an_already_live_join_refused_is_not_committed() {
    let [member, method] = WORKER;
    let recorded = raced(
        joined,
        member,
        method,
        json!(["res-w", 7, "t", null]),
        (2, COORDINATOR_GONE),
    );
    assert_eq!(
        (recorded.outcome, recorded.decision),
        (refused(RefusalKind::NotMember, false), None)
    );
}

/// `_bootstrap` of a fresh board commits the placeholder before the
/// join: a refusal after it is committed, with the branch the join took
/// (`placeholder` when it took none) and the placeholder it wrote; on a
/// board that holds a run, it writes none, and the already-live join
/// none, so the summary's refusal is plain.
#[test]
fn a_bootstrap_refused_after_its_placeholder_is_recorded_committed() {
    let bootstrap = json!([1, "s", null]);
    let wrote = BoardOpDetail {
        placeholder_created: Some(true),
        ..BoardOpDetail::NONE
    };
    for (nth, race, kind, decision) in [
        (3, COORDINATOR_GONE, RefusalKind::NotMember, "already_live"),
        (
            2,
            "UPDATE members SET pid=99, reservation='other'",
            RefusalKind::LaunchConflict,
            "placeholder",
        ),
    ] {
        let recorded = raced(
            nothing,
            "parent",
            "_bootstrap",
            bootstrap.clone(),
            (nth, race),
        );
        assert_eq!(
            (
                recorded.outcome,
                recorded.decision.as_deref(),
                recorded.detail
            ),
            (refused(kind, true), Some(decision), wrote.clone()),
            "{race}"
        );
    }
    let recorded = raced(
        |handles| {
            call(handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
        },
        "parent",
        "_bootstrap",
        bootstrap,
        (3, COORDINATOR_GONE),
    );
    assert_eq!(
        (recorded.outcome, recorded.decision, recorded.detail),
        (
            refused(RefusalKind::NotMember, false),
            None,
            BoardOpDetail::NONE
        )
    );
}
