use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::application::project_board::dto::{BoardOp, ProjectBoardError, RunEnd, RunEndOutcome};
use crate::application::project_board::fold_back::tests::plan;
use crate::application::project_board::keeper::tests::projection;
use crate::application::project_board::keeper::{
    KeeperEnd, KeptRun, Look, Renewal, STALL_LIMIT, Watch,
};
use crate::application::project_board::ports::{
    BoardArtifact, BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite,
    ProjectBoardStore, PullRequestState, SetAsideClear, StartedSwarm,
};
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::application::project_board::use_cases::{ChangeBoardTask, CompleteMergedTask, EndRun};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;

/// A swarm board read as scripted: each look takes the next answer, the
/// last answer standing once the script runs out.
struct Script(Mutex<VecDeque<Result<Value, String>>>);

impl Script {
    fn of(answers: Vec<Result<Value, String>>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(answers.into())))
    }
}

impl StartedSwarm for Script {
    fn board(&self, environment_ref: &str) -> Result<Value, String> {
        assert_eq!(environment_ref, "C1", "the run's own swarm is read");
        let mut answers = self.0.lock().unwrap();
        match answers.len() {
            0 => Err("no answer scripted".into()),
            1 => answers[0].clone(),
            _ => answers.pop_front().expect("an answer"),
        }
    }
}

fn swarm(status: &str) -> Result<Value, String> {
    let items = json!([
        {"title": "api: Build the api", "state": "completed", "owner": "worker-1",
         "evidence": [{"artifact": "src/api.rs", "revision": "4f2a9c1"}]},
    ]);
    Ok(projection(Some(status), "live", items))
}

fn dead(status: &str) -> Result<Value, String> {
    Ok(projection(Some(status), "dead", json!([])))
}

/// Task `board-store`, started by Ada at 10:00 on its accepted plan: in
/// progress, held until 12:00, run 1 going.
fn started() -> MemoryBoardStore {
    let store = board();
    let mut ready = fields("board-store");
    ready.accepted_plan = Some(1);
    let ready = Task::new(ready).unwrap();
    let accepted = plan();
    let (task, run) = start(
        &ready,
        &[],
        Some(&accepted),
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap()
    .into_writes();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task),
        BoardArtifact::Plan(accepted),
        BoardArtifact::Run(run),
    ];
    let written = store.write(&head, &artifacts, "board: board-store started");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store
}

/// Ada's keeper of run 1, its clock at 10:00, reading `swarm`.
struct Keeping {
    store: MemoryBoardStore,
    clock: Arc<SetClock>,
    keeper: KeepStartedRun,
    watch: Watch,
    kept: KeptRun,
}

impl Keeping {
    fn of(store: MemoryBoardStore, swarm: Arc<Script>) -> Self {
        let clock = SetClock::at("10:00:00");
        let keeper = KeepStartedRun::new(
            Arc::new(store.client()),
            Arc::clone(&clock) as _,
            Arc::new(MarkdownTextCells),
            swarm,
        )
        .unwrap();
        Self {
            store,
            clock,
            keeper,
            watch: Watch::default(),
            kept: KeptRun {
                task: slug("board-store"),
                run: 1,
                environment_ref: "C1".into(),
            },
        }
    }

    /// One look at `time`, on a monotonic clock that moved on as much.
    fn look_at(&mut self, time: &str) -> Look {
        let now = at(time);
        let from = at("10:00:00");
        let seconds = u64::try_from(now.seconds_since(&from)).unwrap();
        *self.clock.now.lock().unwrap() = now;
        *self.clock.elapsed.lock().unwrap() = Duration::from_secs(seconds);
        self.keeper.look(&self.kept, &mut self.watch, None)
    }

    fn run(&self) -> Run {
        self.store.read().unwrap().runs.into_iter().next().unwrap()
    }

    fn claim_expires(&self) -> Timestamp {
        let task = self.store.read().unwrap().tasks.into_iter().next().unwrap();
        task.fields().claim.clone().unwrap().expires
    }
}

/// A swarm seen running, then paused (it waits for its parent to close
/// it), then succeeded is folded back once it has ended, and the keeping
/// ends there.
#[test]
fn a_swarm_running_then_paused_then_succeeded_is_folded_back_once() {
    let script = Script::of(vec![swarm("running"), swarm("paused"), swarm("succeeded")]);
    let mut keeping = Keeping::of(started(), script);
    assert_eq!(keeping.look_at("10:01:00"), Look::default());
    assert_eq!(keeping.look_at("10:02:00"), Look::default());
    assert!(
        keeping.run().is_open(),
        "no fold-back before the swarm ended"
    );
    assert_eq!(keeping.look_at("10:03:00"), Look::ending(KeeperEnd::Folded));
    assert_eq!(keeping.run().fields().outcome, Some(RunOutcome::Succeeded));
    let writes = keeping.store.writes();
    assert_eq!(keeping.look_at("10:04:00"), Look::ending(KeeperEnd::Ended));
    assert_eq!(keeping.store.writes(), writes, "it folds back once");
}

/// The claim is renewed once less than 45 minutes are left, and then not
/// again until less than 45 minutes are left of the renewed one.
#[test]
fn the_claim_is_renewed_when_less_than_45_minutes_are_left() {
    let mut keeping = Keeping::of(started(), Script::of(vec![swarm("running")]));
    assert_eq!(
        keeping.look_at("11:15:00"),
        Look::default(),
        "45 minutes left"
    );
    assert_eq!(
        keeping.look_at("11:16:00"),
        Look {
            renewal: Some(Renewal::Renewed { left_s: 44 * 60 }),
            end: None
        }
    );
    assert_eq!(keeping.claim_expires(), at("13:16:00"));
    assert_eq!(keeping.look_at("11:17:00"), Look::default());
    assert_eq!(
        keeping.look_at("12:32:00").renewal,
        Some(Renewal::Renewed { left_s: 44 * 60 })
    );
}

/// Bob took the task over once Ada's claim had expired: Ada's keeper is
/// refused as no holder, stops renewing, and still folds the run back.
#[test]
fn a_take_over_stops_renewing_and_the_run_is_still_folded_back() {
    let store = started();
    let script = Script::of(vec![swarm("running")]);
    let mut keeping = Keeping::of(store.clone(), Arc::clone(&script));
    let bob = ChangeBoardTask::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("12:01:00"),
    )
    .unwrap();
    bob.execute(&slug("board-store"), &BoardOp::TakeOver)
        .unwrap();
    let refused = keeping.look_at("13:20:00");
    assert_eq!(
        refused.renewal,
        Some(Renewal::Refused {
            left_s: 41 * 60,
            error: ProjectBoardError::Rule(BoardRuleError::NotHolder),
        })
    );
    assert!(!keeping.watch.renewing);
    let writes = keeping.store.writes();
    assert_eq!(
        keeping.look_at("13:21:00"),
        Look::default(),
        "no renewal tried"
    );
    assert_eq!(keeping.store.writes(), writes);
    *script.0.lock().unwrap() = vec![swarm("cancelled")].into();
    assert_eq!(keeping.look_at("13:22:00"), Look::ending(KeeperEnd::Folded));
    assert_eq!(keeping.run().fields().outcome, Some(RunOutcome::Stopped));
}

/// A renewal the board kept moving under is refused as contended, and
/// tried again at the next look.
#[test]
fn a_contended_renewal_is_tried_again_at_the_next_look() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![swarm("running")]));
    moved_under(
        &store,
        crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS,
    );
    let refused = keeping.look_at("11:30:00");
    assert_eq!(
        refused.renewal,
        Some(Renewal::Refused {
            left_s: 30 * 60,
            error: ProjectBoardError::Contended { attempts: 3 },
        })
    );
    assert!(keeping.watch.renewing, "a contended renewal is tried again");
    assert_eq!(
        keeping.look_at("11:31:00").renewal,
        Some(Renewal::Renewed { left_s: 29 * 60 })
    );
}

/// The board moved by another write of its task, `times` writes in a row.
fn moved_under(store: &MemoryBoardStore, times: usize) {
    if times == 0 {
        return;
    }
    let other = store.client();
    store.before_next_write(move |board| {
        let snapshot = board.read().unwrap();
        let task = snapshot.tasks[0].clone();
        let written = other.write(&snapshot.head, &[BoardArtifact::Task(task)], "board: other");
        assert!(matches!(written, Ok(BoardWrite::Written(_))));
        moved_under(&other, times - 1);
    });
}

/// A merge that completed the task ended its run as lost: the keeping
/// ends, writing nothing more.
#[test]
fn a_merge_completion_ends_the_keeping() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![swarm("running")]));
    let ada = ChangeBoardTask::new(Arc::new(store.client()), SetClock::at("10:30:00")).unwrap();
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Open);
    ada.execute(&slug("board-store"), &BoardOp::AddPr(7))
        .unwrap();
    host.set(7, PullRequestState::Merged);
    let completion = CompleteMergedTask::new(
        Arc::new(store.client()),
        SetClock::at("10:40:00"),
        Arc::new(host),
    )
    .unwrap();
    completion.execute(&slug("board-store")).unwrap();
    assert_eq!(keeping.run().fields().outcome, Some(RunOutcome::Lost));
    let writes = keeping.store.writes();
    assert_eq!(keeping.look_at("10:41:00"), Look::ending(KeeperEnd::Ended));
    assert_eq!(keeping.store.writes(), writes);
}

/// A run its holder ended by hand (`end_run`) while it was kept ends the
/// keeping cleanly, writing nothing more.
#[test]
fn an_end_run_while_kept_ends_the_keeping() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![swarm("running")]));
    assert_eq!(keeping.look_at("10:05:00"), Look::default());
    let end_run = EndRun::new(
        Arc::new(store.client()),
        SetClock::at("10:06:00"),
        Arc::new(MarkdownTextCells),
    )
    .unwrap();
    let ended = RunEnd {
        task: slug("board-store"),
        run: 1,
        outcome: RunEndOutcome::Stopped,
        expected_updated: None,
    };
    end_run.execute(&ended).unwrap();
    let writes = keeping.store.writes();
    assert_eq!(keeping.look_at("10:07:00"), Look::ending(KeeperEnd::Ended));
    assert_eq!(keeping.store.writes(), writes);
}

/// A task no longer on the board (archived) ends the keeping.
#[test]
fn a_task_gone_from_the_board_ends_the_keeping() {
    let store = board();
    let mut keeping = Keeping::of(store, Script::of(vec![swarm("succeeded")]));
    assert_eq!(keeping.look_at("10:01:00"), Look::ending(KeeperEnd::Gone));
}

/// A swarm whose board cannot be read, or whose coordinator is dead, while
/// no end was seen, stalls; stalled for 30 minutes in a row the keeper
/// gives up, writing nothing: the run is left open for `end_run`. A sight
/// of a live coordinator in between starts the count again.
#[test]
fn a_coordinator_lost_for_30_minutes_is_given_up_writing_nothing() {
    let script = Script::of(vec![
        Err("swarm C1 is neither running nor retained".into()),
        dead("running"),
        swarm("running"),
        dead("paused"),
    ]);
    let mut keeping = Keeping::of(started(), script);
    assert_eq!(keeping.look_at("10:01:00"), Look::default());
    assert_eq!(keeping.look_at("10:20:00"), Look::default());
    assert_eq!(keeping.look_at("10:40:00"), Look::default(), "alive again");
    assert_eq!(keeping.watch.stalled_since, None);
    assert_eq!(keeping.look_at("10:41:00"), Look::default());
    let stall = STALL_LIMIT.as_secs() / 60;
    assert_eq!(stall, 30);
    let writes = keeping.store.writes();
    assert_eq!(keeping.look_at("11:10:00"), Look::default(), "29 minutes");
    assert_eq!(
        keeping.look_at("11:11:00"),
        Look::ending(KeeperEnd::SwarmLost)
    );
    assert_eq!(keeping.store.writes(), writes, "nothing written");
    assert!(keeping.run().is_open());
}

/// The board store, its next read failing once `unread` is set.
struct Flaky {
    store: MemoryBoardStore,
    unread: Mutex<bool>,
}

impl ProjectBoardStore for Flaky {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        match std::mem::take(&mut *self.unread.lock().unwrap()) {
            true => Err(BoardStoreError::Unavailable("the remote is down".into())),
            false => self.store.read(),
        }
    }

    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.store.write(expected, artifacts, message)
    }

    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.store.clear_set_aside(expected, clear, message)
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        self.store.bootstrap()
    }

    fn writer(&self) -> Identity {
        self.store.writer()
    }
}

/// A board that cannot be read this time neither ends the keeping nor
/// renews: the next look reads it again.
#[test]
fn a_board_unreadable_for_a_look_skips_its_renewal() {
    let flaky = Arc::new(Flaky {
        store: started(),
        unread: Mutex::new(true),
    });
    let clock = SetClock::at("11:30:00");
    let swarm = Script::of(vec![swarm("running")]);
    let keeper = KeepStartedRun::new(
        Arc::clone(&flaky) as _,
        Arc::clone(&clock) as _,
        Arc::new(MarkdownTextCells),
        swarm,
    )
    .unwrap();
    let kept = KeptRun {
        task: slug("board-store"),
        run: 1,
        environment_ref: "C1".into(),
    };
    let mut watch = Watch::default();
    assert_eq!(keeper.look(&kept, &mut watch, None), Look::default());
    assert_eq!(
        keeper.look(&kept, &mut watch, None).renewal,
        Some(Renewal::Renewed { left_s: 30 * 60 })
    );
}

/// A fold-back the store refuses for good ends the keeping, the run left
/// open; one it could not make this time is tried again at the next look.
#[test]
fn a_fold_back_refused_for_good_ends_the_keeping_and_one_unavailable_is_retried() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![swarm("succeeded")]));
    store.fail_next_write(BoardStoreError::Unavailable("the remote is down".into()));
    assert_eq!(keeping.look_at("10:01:00"), Look::default());
    assert!(keeping.run().is_open());
    store.fail_next_write(BoardStoreError::InvalidWrite("refused".into()));
    assert_eq!(
        keeping.look_at("10:02:00"),
        Look::ending(KeeperEnd::FoldBackRefused)
    );
    assert!(keeping.run().is_open());
}
