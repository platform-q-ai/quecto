use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::application::project_board::dto::{
    BoardOp, ProjectBoardError, RunEnd, RunEndOutcome, SwarmBoardSight, SwarmSight, SwarmStage,
};
use crate::application::project_board::fold_back::tests::{plan, swarm_item};
use crate::application::project_board::fold_back::{SwarmEnd, SwarmItemState, SwarmRunView};
use crate::application::project_board::keeper::{
    KEEPER_PERIOD, KeeperEnd, KeeperNotice, KeptRun, Look, PAUSED_PERIOD, Renewal, Watch,
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
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;

/// A swarm read as scripted: each look takes the next answer, the last
/// answer standing once the script runs out.
struct Script(Mutex<VecDeque<SwarmBoardSight>>);

impl Script {
    fn of(answers: Vec<SwarmBoardSight>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(answers.into())))
    }

    fn then(&self, answers: Vec<SwarmBoardSight>) {
        *self.0.lock().unwrap() = answers.into();
    }
}

impl StartedSwarm for Script {
    fn board(&self, environment_ref: &str) -> SwarmBoardSight {
        assert_eq!(environment_ref, "C1", "the run's own swarm is read");
        let mut answers = self.0.lock().unwrap();
        match answers.len() {
            0 => SwarmBoardSight::Unreadable("no answer scripted".into()),
            1 => answers[0].clone(),
            _ => answers.pop_front().expect("an answer"),
        }
    }
}

fn view(end: SwarmEnd) -> SwarmRunView {
    SwarmRunView {
        end,
        items: vec![swarm_item(
            "api: Build the api",
            SwarmItemState::Completed,
            Some("worker-1"),
            &[("src/api.rs", "4f2a9c1")],
        )],
        observed_tokens: 52_000,
    }
}

fn seen(run: &str, alive: bool, stage: SwarmStage) -> SwarmBoardSight {
    SwarmBoardSight::Board(SwarmSight {
        run_id: Some(run.into()),
        coordinator_alive: alive,
        truncated: false,
        stage,
    })
}

fn going() -> SwarmBoardSight {
    seen("r1", true, SwarmStage::Going)
}

fn dead() -> SwarmBoardSight {
    seen("r1", false, SwarmStage::Going)
}

fn paused(proposed: Option<SwarmEnd>) -> SwarmBoardSight {
    seen(
        "r1",
        true,
        SwarmStage::Paused {
            proposed: proposed.map(view),
        },
    )
}

fn ended(end: SwarmEnd) -> SwarmBoardSight {
    seen("r1", true, SwarmStage::Ended(view(end)))
}

/// A board with no run yet, its coordinator alive or not.
fn unstarted(alive: bool) -> SwarmBoardSight {
    SwarmBoardSight::Board(SwarmSight {
        run_id: None,
        coordinator_alive: alive,
        truncated: false,
        stage: SwarmStage::Going,
    })
}

fn unreadable() -> SwarmBoardSight {
    SwarmBoardSight::Unreadable("contended".into())
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

/// The board store, counting its reads; once `hide` is set, it reads with
/// no task on the board.
struct Counted {
    store: MemoryBoardStore,
    reads: AtomicUsize,
    hide: AtomicBool,
    unread: AtomicBool,
}

impl ProjectBoardStore for Counted {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.unread.swap(false, Ordering::SeqCst) {
            return Err(BoardStoreError::Unavailable("the remote is down".into()));
        }
        let mut snapshot = self.store.read()?;
        if self.hide.load(Ordering::SeqCst) {
            snapshot.tasks.clear();
        }
        Ok(snapshot)
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

/// Ada's keeper of run 1, its clock at 10:00, reading `swarm`.
struct Keeping {
    store: MemoryBoardStore,
    counted: Arc<Counted>,
    clock: Arc<SetClock>,
    keeper: KeepStartedRun,
    watch: Watch,
    kept: KeptRun,
}

impl Keeping {
    fn of(store: MemoryBoardStore, swarm: Arc<Script>) -> Self {
        let clock = SetClock::at("10:00:00");
        let counted = Arc::new(Counted {
            store: store.client(),
            reads: AtomicUsize::new(0),
            hide: AtomicBool::new(false),
            unread: AtomicBool::new(false),
        });
        let keeper = KeepStartedRun::new(
            Arc::clone(&counted) as _,
            Arc::clone(&clock) as _,
            Arc::new(MarkdownTextCells),
            swarm,
        )
        .unwrap();
        Self {
            store,
            counted,
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
        let seconds = u64::try_from(now.seconds_since(&at("10:00:00"))).unwrap();
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

    fn reads(&self) -> usize {
        self.counted.reads.load(Ordering::SeqCst)
    }
}

fn renewed(left_minutes: i64) -> Look {
    Look {
        renewal: Some(Renewal::Renewed {
            left_s: left_minutes * 60,
        }),
        ..Look::default()
    }
}

fn paused_look() -> Look {
    Look {
        next: PAUSED_PERIOD,
        ..Look::default()
    }
}

/// A swarm seen running, then paused (looked at more often then: a close
/// comes next), then succeeded is folded back once it has ended. The first
/// look names the run's environment on the board.
#[test]
fn a_swarm_running_then_paused_then_succeeded_is_folded_back_once() {
    let script = Script::of(vec![going(), paused(None), ended(SwarmEnd::Succeeded)]);
    let mut keeping = Keeping::of(started(), script);
    assert_eq!(keeping.look_at("10:01:00"), Look::default());
    assert_eq!(keeping.run().fields().environment.as_deref(), Some("C1"));
    let writes = keeping.store.writes();
    assert_eq!(keeping.look_at("10:02:00"), paused_look());
    assert_eq!(keeping.store.writes(), writes, "named once");
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

/// A swarm paused with a proposed end, then torn down by its close before
/// the next look (gone), is folded back with the end it proposed.
#[test]
fn a_paused_swarm_torn_down_by_its_close_is_folded_back_as_it_proposed() {
    let script = Script::of(vec![
        paused(Some(SwarmEnd::Succeeded)),
        SwarmBoardSight::Gone,
    ]);
    let mut keeping = Keeping::of(started(), script);
    assert_eq!(keeping.look_at("10:01:00"), paused_look());
    assert_eq!(keeping.look_at("10:01:05"), Look::ending(KeeperEnd::Folded));
    let run = keeping.run();
    assert_eq!(run.fields().outcome, Some(RunOutcome::Succeeded));
    assert_eq!(run.fields().items[0].state, ItemState::Done);
}

/// A swarm resumed after a pause no longer holds what it proposed: gone
/// then, nothing is folded back.
#[test]
fn a_swarm_resumed_forgets_what_it_proposed() {
    let script = Script::of(vec![
        paused(Some(SwarmEnd::Succeeded)),
        going(),
        SwarmBoardSight::Gone,
    ]);
    let mut keeping = Keeping::of(started(), script);
    keeping.look_at("10:01:00");
    assert_eq!(keeping.look_at("10:01:05"), Look::default());
    assert_eq!(
        keeping.look_at("10:02:05"),
        Look::ending(KeeperEnd::SwarmLost)
    );
    assert!(keeping.run().is_open());
}

/// A swarm gone with nothing proposed is given up at once, writing
/// nothing: the run is left for `end_run`, and the session is told.
#[test]
fn a_swarm_gone_with_nothing_proposed_is_given_up_at_once() {
    let mut keeping = Keeping::of(started(), Script::of(vec![going(), SwarmBoardSight::Gone]));
    keeping.look_at("10:01:00");
    let writes = keeping.store.writes();
    let lost = keeping.look_at("10:02:00");
    assert_eq!(lost, Look::ending(KeeperEnd::SwarmLost));
    assert_eq!(
        lost.notice,
        Some(KeeperNotice::LeftOpen(KeeperEnd::SwarmLost))
    );
    assert_eq!(keeping.store.writes(), writes);
    assert!(keeping.run().is_open());
}

/// Another run on the board than the one first sighted is the swarm gone
/// (its board was made again).
#[test]
fn another_run_on_the_board_is_the_swarm_gone() {
    let script = Script::of(vec![going(), seen("r2", true, SwarmStage::Going)]);
    let mut keeping = Keeping::of(started(), script);
    keeping.look_at("10:01:00");
    assert_eq!(
        keeping.look_at("10:02:00"),
        Look::ending(KeeperEnd::SwarmLost)
    );
}

/// The claim is renewed when less than 45 minutes are left of it, from its
/// expiry as last read: routine looks read the board not at all, only
/// before a renewal.
#[test]
fn the_claim_is_renewed_under_45_minutes_and_routine_looks_read_no_board() {
    let mut keeping = Keeping::of(started(), Script::of(vec![going()]));
    assert_eq!(keeping.look_at("10:01:00"), Look::default());
    let reads = keeping.reads();
    assert_eq!(keeping.look_at("10:30:00"), Look::default());
    assert_eq!(
        keeping.look_at("11:15:00"),
        Look::default(),
        "45 minutes left"
    );
    assert_eq!(keeping.reads(), reads, "routine looks read no board");
    assert_eq!(keeping.look_at("11:16:00"), renewed(44));
    assert_eq!(keeping.claim_expires(), at("13:16:00"));
    assert_eq!(keeping.look_at("11:17:00"), Look::default());
    assert_eq!(keeping.look_at("12:32:00"), renewed(44));
}

/// Bob took the task over once Ada's claim lapsed: the renewal is refused
/// as no holder, renewing stops, and the run is still folded back.
#[test]
fn a_take_over_stops_renewing_and_the_run_is_still_folded_back() {
    let store = started();
    let script = Script::of(vec![going()]);
    let mut keeping = Keeping::of(store.clone(), Arc::clone(&script));
    let bob = ChangeBoardTask::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("12:01:00"),
    )
    .unwrap();
    bob.execute(&slug("board-store"), &BoardOp::TakeOver)
        .unwrap();
    assert_eq!(
        keeping.look_at("13:20:00").renewal,
        Some(Renewal::Refused {
            left_s: 41 * 60,
            error: ProjectBoardError::Rule(BoardRuleError::NotHolder),
        })
    );
    assert!(!keeping.watch.renewing);
    let (writes, reads) = (keeping.store.writes(), keeping.reads());
    assert_eq!(
        keeping.look_at("13:50:00"),
        Look::default(),
        "no renewal tried"
    );
    assert_eq!((keeping.store.writes(), keeping.reads()), (writes, reads));
    script.then(vec![ended(SwarmEnd::Cancelled)]);
    assert_eq!(keeping.look_at("13:51:00"), Look::ending(KeeperEnd::Folded));
    assert_eq!(keeping.run().fields().outcome, Some(RunOutcome::Stopped));
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

/// A renewal refused as contended, or by a store that could not be
/// reached, is tried again at the next look; any other refusal stops the
/// renewing, refused once.
#[test]
fn only_a_contended_or_unreachable_renewal_is_tried_again() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![going()]));
    keeping.look_at("10:01:00");
    moved_under(
        &store,
        crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS,
    );
    assert_eq!(
        keeping.look_at("11:30:00").renewal,
        Some(Renewal::Refused {
            left_s: 30 * 60,
            error: ProjectBoardError::Contended { attempts: 3 },
        })
    );
    store.fail_next_write(BoardStoreError::Unavailable("the remote is down".into()));
    assert!(matches!(
        keeping.look_at("11:31:00").renewal,
        Some(Renewal::Refused {
            error: ProjectBoardError::Store(BoardStoreError::Unavailable(_)),
            ..
        })
    ));
    assert!(keeping.watch.renewing, "tried again");
    store.refuse_next_write("the remote declined it");
    assert!(matches!(
        keeping.look_at("11:32:00").renewal,
        Some(Renewal::Refused {
            error: ProjectBoardError::Refused(_),
            ..
        })
    ));
    assert!(
        !keeping.watch.renewing,
        "any other refusal stops the renewing"
    );
    assert_eq!(keeping.look_at("11:33:00").renewal, None);
}

/// A merge that completed the task ended its run as lost: the keeping
/// ends at its first board read, writing nothing more.
#[test]
fn a_merge_completion_ends_the_keeping() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![going()]));
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

/// A run its holder ended by hand while it was kept is found ended at the
/// next board read (before a renewal), and the keeping ends cleanly.
#[test]
fn an_end_run_while_kept_ends_the_keeping_at_the_next_board_read() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![going()]));
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
    assert_eq!(keeping.look_at("10:07:00"), Look::default(), "not read yet");
    assert_eq!(keeping.look_at("11:16:00"), Look::ending(KeeperEnd::Ended));
    assert_eq!(keeping.store.writes(), writes, "no renewal written");
}

/// A task no longer on the board (archived) ends the keeping.
#[test]
fn a_task_gone_from_the_board_ends_the_keeping() {
    let mut keeping = Keeping::of(board(), Script::of(vec![ended(SwarmEnd::Succeeded)]));
    assert_eq!(keeping.look_at("10:01:00"), Look::ending(KeeperEnd::Gone));
}

/// A task gone between its claim's read and the fold-back ends the keeping
/// as gone.
#[test]
fn a_task_gone_at_the_fold_back_ends_the_keeping() {
    let script = Script::of(vec![going()]);
    let mut keeping = Keeping::of(started(), Arc::clone(&script));
    keeping.look_at("10:01:00");
    keeping.counted.hide.store(true, Ordering::SeqCst);
    script.then(vec![ended(SwarmEnd::Succeeded)]);
    assert_eq!(keeping.look_at("10:02:00"), Look::ending(KeeperEnd::Gone));
}

/// A sighted swarm stalled (unreadable, or its coordinator dead) for 30
/// minutes in a row is given up, writing nothing; a sighting of its live
/// coordinator in between starts the count again.
#[test]
fn a_sighted_swarm_stalled_for_30_minutes_is_given_up() {
    let script = Script::of(vec![going(), unreadable(), dead(), going(), dead()]);
    let mut keeping = Keeping::of(started(), script);
    keeping.look_at("10:01:00");
    assert_eq!(keeping.look_at("10:02:00"), Look::default());
    assert_eq!(keeping.look_at("10:31:00"), Look::default(), "29 minutes");
    assert_eq!(keeping.look_at("10:40:00"), Look::default(), "alive again");
    assert_eq!(keeping.watch.stalled_since, None);
    assert_eq!(keeping.look_at("10:41:00"), Look::default());
    let writes = keeping.store.writes();
    assert_eq!(keeping.look_at("11:10:00"), Look::default(), "29 minutes");
    assert_eq!(
        keeping.look_at("11:11:00"),
        Look::ending(KeeperEnd::SwarmLost)
    );
    assert_eq!(keeping.store.writes(), writes, "nothing written");
    assert!(keeping.run().is_open());
}

/// Before its run is first sighted a swarm has the start's grace, an hour
/// from the keeping's first look, not the stall limit: a container to make
/// and a coordinator to boot.
#[test]
fn an_unsighted_swarm_has_the_starts_grace() {
    let script = Script::of(vec![unstarted(false), unreadable()]);
    let mut keeping = Keeping::of(started(), script);
    assert_eq!(keeping.look_at("10:00:00"), Look::default());
    assert_eq!(
        keeping.look_at("10:31:00"),
        Look::default(),
        "past the stall limit"
    );
    assert_eq!(keeping.look_at("10:59:00"), Look::default());
    assert_eq!(
        keeping.look_at("11:00:00"),
        Look::ending(KeeperEnd::SwarmLost)
    );
}

/// An ended swarm in a view its board's limits cut short is never folded
/// back: the run is left for `end_run`. A paused one keeps no proposal.
#[test]
fn a_truncated_view_is_never_folded_back() {
    let cut = |stage| {
        SwarmBoardSight::Board(SwarmSight {
            run_id: Some("r1".into()),
            coordinator_alive: true,
            truncated: true,
            stage,
        })
    };
    let script = Script::of(vec![
        cut(SwarmStage::Paused {
            proposed: Some(view(SwarmEnd::Succeeded)),
        }),
        cut(SwarmStage::Ended(view(SwarmEnd::Succeeded))),
    ]);
    let mut keeping = Keeping::of(started(), script);
    keeping.look_at("10:01:00");
    assert_eq!(keeping.watch.proposed, None);
    assert_eq!(
        keeping.look_at("10:01:05"),
        Look::ending(KeeperEnd::Truncated)
    );
    assert!(keeping.run().is_open());
}

/// A swarm paused for six hours, never closed, stops the renewing (the
/// claim lapses), and the session is told once; the keeper still watches.
#[test]
fn a_swarm_paused_for_six_hours_stops_the_renewing() {
    let script = Script::of(vec![paused(None)]);
    let mut keeping = Keeping::of(started(), Arc::clone(&script));
    keeping.look_at("10:01:00");
    keeping.look_at("11:16:00");
    let told = keeping.look_at("16:01:00");
    assert_eq!(told.notice, Some(KeeperNotice::PausedTooLong));
    assert!(!keeping.watch.renewing);
    assert_eq!(keeping.look_at("16:02:00").notice, None, "told once");
    script.then(vec![ended(SwarmEnd::Succeeded)]);
    assert_eq!(keeping.look_at("16:03:00"), Look::ending(KeeperEnd::Folded));
}

/// A fold-back the board could not take this time is tried again at the
/// next look; one refused for good ends the keeping, the run left open.
#[test]
fn a_fold_back_refused_for_good_ends_the_keeping_and_one_unavailable_is_retried() {
    let store = started();
    let mut keeping = Keeping::of(store.clone(), Script::of(vec![going()]));
    keeping.look_at("10:01:00");
    let mut keeping = Keeping {
        keeper: KeepStartedRun::new(
            Arc::clone(&keeping.counted) as _,
            Arc::clone(&keeping.clock) as _,
            Arc::new(MarkdownTextCells),
            Script::of(vec![ended(SwarmEnd::Succeeded)]),
        )
        .unwrap(),
        ..keeping
    };
    store.fail_next_write(BoardStoreError::Unavailable("the remote is down".into()));
    assert_eq!(keeping.look_at("10:02:00"), Look::default());
    moved_under(
        &store,
        crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS,
    );
    assert_eq!(keeping.look_at("10:03:00"), Look::default(), "contended");
    assert!(keeping.run().is_open());
    store.fail_next_write(BoardStoreError::InvalidWrite("refused".into()));
    assert_eq!(
        keeping.look_at("10:04:00"),
        Look::ending(KeeperEnd::FoldBackRefused)
    );
    assert!(keeping.run().is_open());
}

/// A board that cannot be read before a renewal renews nothing: the next
/// look reads it again.
#[test]
fn a_board_unreadable_before_a_renewal_renews_at_the_next_look() {
    let mut keeping = Keeping::of(started(), Script::of(vec![going()]));
    keeping.look_at("10:01:00");
    keeping.counted.unread.store(true, Ordering::SeqCst);
    assert_eq!(keeping.look_at("11:30:00"), Look::default());
    assert_eq!(keeping.look_at("11:31:00"), renewed(29));
    let _ = KEEPER_PERIOD;
}
