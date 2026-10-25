//! Starts a task (S4c), in two halves (epic #2705 B8). [`BeginBoardRun`]
//! writes it: the task claimed if it is ready (or its claim renewed when
//! the starter holds it), moved to in progress, and run 1 of its accepted
//! plan begun, written together in one change, task first; and it undoes
//! a begun start whose coordinator did not launch. The coordinator is
//! launched where the session is ([`super::LaunchBegunRun`]):
//! [`StartBoardTask`] does both on this machine's board, and a board
//! server begins and undoes for the client that launches.
//!
//! - **Confirmed plan only.** The caller names the accepted version it
//!   confirmed (`plan_version`); a task that names another by the time the
//!   change is written is refused (`accepted_plan_changed`), so no plan the
//!   caller did not confirm is launched.
//! - **Nothing written that cannot launch.** The brief is built on each
//!   attempt before the write: a plan that cannot brief a swarm is refused
//!   with nothing written. With no launcher (this session may not spawn)
//!   nothing is read or written at all.
//! - **The brief is rendered before the write.** Each attempt renders the
//!   coordinator's text and checks its size, so nothing after the write
//!   can refuse it.
//! - **The undo** folds the run back as failed, as a swarm that reported
//!   nothing, which moves the task back to ready, releasing it, or to the
//!   review or blocked status the start took it from, in the same write
//!   (#2705 T5). It writes through a store that ignores the call's cancel,
//!   so a cancel never strands a run.
//!
//! Blocking: the store's calls, a pause before a retried write and the
//! launch itself hold the calling thread.
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::board_change::{checked_now, checked_writer, commit, owned, task_on};
use super::{FoldBackRun, LaunchBegunRun, board_view};
use crate::application::project_board::dto::{
    FoldBackFacts, FoldedBack, ProjectBoardError, RunAfterUndo, TaskStart,
};
use crate::application::project_board::fold_back::{SwarmEnd, SwarmRunView};
use crate::application::project_board::ports::{
    BegunStarts, BoardArtifact, BoardClock, BoardTextCells, CoordinatorLaunch, CoordinatorRequest,
    ProjectBoardStore, RunLiveness, Undone,
};
use crate::application::project_board::start::{
    Begun, NotedFoldBack, StartFacts, StartUndo, StartWrite, StartedTask,
};
use crate::application::project_board::swarm_brief::{
    BriefRefusal, MAX_BRIEF_BYTES, SwarmBrief, brief_of, render,
};
use crate::application::project_board::telemetry::{OpFacts, counted};
use crate::domain::project_board::entities::claim::Identity;

use crate::domain::project_board::services::claims::is_live;
use crate::domain::project_board::services::plans::as_accepted;
use crate::domain::project_board::services::starts::{Started, start};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::telemetry::StartClaim;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct BeginBoardRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who starts.
    actor: Identity,
    /// The undo's, over a store that ignores the call's cancel.
    fold_back: FoldBackRun,
}

/// A start as its write left it: the start, its brief and the brief's
/// text, as the last attempt built them.
struct Written {
    started: Started,
    brief: SwarmBrief,
    text: String,
}

impl BeginBoardRun {
    /// A begin writing to `store`, its undo writing to `undo_store` (the
    /// same board, never cancelled).
    ///
    /// # Errors
    /// Either store writes as an invalid identity, or the two write as
    /// different ones.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        undo_store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        assert_eq!(
            checked_writer(&*undo_store)?,
            actor,
            "a start and its undo write as one identity"
        );
        let fold_back = FoldBackRun::new(undo_store, Arc::clone(&clock), cells)?;
        Ok(Self {
            store,
            clock,
            actor,
            fold_back,
        })
    }

    /// The task `request` names started and run 1 of its plan begun, as
    /// written, with what its coordinator is launched with; noting the
    /// write in `facts`, when given.
    ///
    /// # Errors
    /// No such task, or its file, one of its plans' or runs' set aside; a
    /// task stamped ahead; another plan accepted than the one confirmed;
    /// the rules' (no accepted plan, a task not startable or held by
    /// another, a run still going); a plan that cannot brief a swarm; the
    /// clock's; a board that kept moving; the store's.
    pub fn begin(
        &self,
        request: &TaskStart,
        facts: Option<&mut OpFacts>,
    ) -> Result<Begun<StartWrite>, ProjectBoardError> {
        let Written {
            started,
            brief,
            text,
        } = self.write(request, facts)?;
        let run = started.run().fields().number;
        let claim = match started.took_claim() {
            true => StartClaim::Fresh,
            false => StartClaim::Held,
        };
        let roles = started.plan().fields().team.as_ref();
        let noted = StartFacts {
            run: Some(u64::from(run)),
            plan_version: Some(u64::from(request.plan_version)),
            members: brief.member_limit.map(u64::from),
            roles: Some(counted(roles.map_or(0, |team| team.roles.len()))),
            items: Some(counted(brief.items.len())),
            not_applied: Some(counted(brief.not_applied.len())),
            claim: Some(claim),
            ..StartFacts::default()
        };
        let coordinator = brief.coordinator.as_ref();
        let launch = CoordinatorRequest {
            task: request.task.clone(),
            run,
            plan_version: brief.plan_version,
            brief: text,
            model: coordinator.map(|choice| choice.model.clone()),
            effort: coordinator.map(|choice| choice.effort),
        };
        Ok(Begun {
            launch,
            from: started.from(),
            claim,
            noted,
            written: StartWrite {
                started,
                not_applied: brief.not_applied,
            },
        })
    }

    /// A board server's undo of the start `request` names (B8), as
    /// `undo` says: the refusal its client's launch came to, the run folded
    /// back as failed; or why it may not be made ([`Self::undoable`]).
    /// Nothing is seen running here: the client asks for an undo only once
    /// no coordinator of the run runs there.
    pub fn undone(&self, request: &TaskStart, undo: &StartUndo) -> ProjectBoardError {
        if let Err(refused) = self.undoable(request, undo) {
            return refused;
        }
        let (run_state, _, _) = self.undo(undo, false);
        launch_failed(undo, run_state, None)
    }

    /// Whether `undo` may be made, read in the server's own write: it is
    /// of the start it is sent with, from a status a start takes a task
    /// from; and while its run is open, that is the task's latest run, the
    /// task is in progress and the writer holds its live claim. A run that
    /// has ended is left for the fold-back to find ended.
    fn undoable(&self, request: &TaskStart, undo: &StartUndo) -> Result<(), ProjectBoardError> {
        let invalid = |field, why| ProjectBoardError::Invalid(SchemaError::new(field, why));
        let rule = ProjectBoardError::Rule;
        match (&undo.task, undo.plan_version) == (&request.task, request.plan_version) {
            true => {}
            false => return Err(invalid("undo", "is not of the start it is sent with")),
        }
        match undo.from {
            TaskStatus::Ready | TaskStatus::Claimed | TaskStatus::Review | TaskStatus::Blocked => {}
            TaskStatus::InProgress
            | TaskStatus::Draft
            | TaskStatus::Done
            | TaskStatus::Archived => {
                return Err(invalid("from", "is not a status a start takes a task from"));
            }
        }
        let board = self.store.read()?;
        let task = task_on(&board, &undo.task)?;
        let runs = board_view::runs_of(&board, &undo.task)?;
        let run = runs
            .iter()
            .find(|run| run.fields().number == undo.run)
            .ok_or(rule(BoardRuleError::NoSuchRun(undo.run)))?;
        let latest = runs.iter().map(|run| run.fields().number).max();
        let now = checked_now(&*self.clock)?.now;
        let claim = task.fields().claim.as_ref();
        let holds = claim.is_some_and(|claim| claim.holder.is(&self.actor));
        let live = claim.is_some_and(|claim| is_live(claim, &now));
        let status = task.fields().status;
        match (run.is_open(), latest == Some(undo.run), status, holds, live) {
            (false, _, _, _, _) | (true, true, TaskStatus::InProgress, true, true) => Ok(()),
            (true, _, _, false, _) => Err(rule(BoardRuleError::NotHolder)),
            (true, _, _, true, false) => Err(rule(BoardRuleError::ClaimExpired)),
            (true, false, _, true, true) => Err(rule(BoardRuleError::RunsInconsistent(format!(
                "run {} is not the task's latest run",
                undo.run
            )))),
            (true, true, from, true, true) => Err(rule(BoardRuleError::NotAllowed {
                from,
                to: undo.from,
            })),
        }
    }

    /// The task and run 1 written in one change, with the start, its brief
    /// and its text as the last attempt built them.
    fn write(
        &self,
        request: &TaskStart,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Written, ProjectBoardError> {
        let id = &request.task;
        let message = format!("board: start {}", id.as_str());
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let noting = facts.is_some();
        let mut read = None;
        let mut built = None;
        let written = commit(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                // This attempt's read and build, or none: never an earlier one's.
                (read, built) = (None, None);
                let task = task_on(board, id)?;
                if noting {
                    read = Some(task.fields().status);
                }
                when.may_build_on(task)?;
                match task.fields().accepted_plan {
                    // The domain's start refuses a task with no accepted plan.
                    None => {}
                    Some(_) => as_accepted(task, Some(request.plan_version))?,
                }
                let versions = board_view::plans_of(board, id)?;
                let runs = owned(board_view::runs_of(board, id)?);
                let plan = task.fields().accepted_plan.and_then(|accepted| {
                    versions
                        .into_iter()
                        .find(|plan| plan.fields().version == accepted)
                });
                let started = start(task, &board.tasks, &runs, plan, actor, &when.now)?;
                let brief = brief_of(&started)?;
                // The text the coordinator gets, measured before the write.
                let text = render(&brief);
                if text.len() > MAX_BRIEF_BYTES {
                    let bytes = text.len();
                    return Err(BriefRefusal::BriefTooLarge { bytes }.into());
                }
                let (task, run) = started.clone().into_writes();
                built = Some(Written {
                    started,
                    brief,
                    text,
                });
                Ok(vec![BoardArtifact::Task(task), BoardArtifact::Run(run)])
            },
        );
        if let Some(facts) = facts {
            facts.status_from = read;
            facts.status_to = written.as_ref().ok().map(|_| TaskStatus::InProgress);
        }
        let written = written?;
        let built = built.expect("a written start was built");
        let (task, run) = built.started.clone().into_writes();
        assert_eq!(
            written,
            [BoardArtifact::Task(task), BoardArtifact::Run(run)],
            "the start written is the last one built"
        );
        Ok(built)
    }
}

impl BegunStarts for BeginBoardRun {
    /// The run folded back as failed, as a swarm that reported nothing:
    /// where that left it, and the fold-back, timed and with its facts
    /// when `noting`.
    fn undo(&self, undo: &StartUndo, noting: bool) -> Undone {
        let failed = SwarmRunView {
            end: SwarmEnd::Failed,
            items: vec![],
            observed_tokens: 0,
        };
        let (id, run) = (&undo.task, undo.run);
        let before = self.clock.elapsed();
        let mut facts = FoldBackFacts::default();
        let folded =
            self.fold_back
                .undo_noted(id, run, (&failed, undo.from), noting.then_some(&mut facts));
        let run_state = match &folded {
            Ok(FoldedBack::Folded(_)) => RunAfterUndo::Folded,
            Ok(FoldedBack::Ended(_)) => RunAfterUndo::EndedAlready,
            Err(_) => RunAfterUndo::Open,
        };
        let noted = NotedFoldBack {
            task: id.clone(),
            run,
            result: folded,
            facts,
            elapsed: self.clock.elapsed().saturating_sub(before),
        };
        (run_state, Some(noted), None)
    }

    fn elapsed(&self) -> Option<std::time::Duration> {
        Some(self.clock.elapsed())
    }
}

/// The refusal of a start whose coordinator did not launch, as `undo`
/// says, once its undo left the run as `run_state`; `still_running` names
/// a coordinator that may still run (the start then left as it is).
pub(super) fn launch_failed(
    undo: &StartUndo,
    run_state: RunAfterUndo,
    still_running: Option<String>,
) -> ProjectBoardError {
    let (released, restored) = undone_task(run_state, undo.from);
    ProjectBoardError::LaunchFailed {
        reason: undo.reason.clone(),
        run: undo.run,
        run_state,
        released,
        restored,
        still_running,
    }
}

/// Where a start's undo left its task, from where it left the run
/// (`run_state`) and the status the start took the task `from` (#2705 T5):
/// whether it was released (back to ready), and the review or blocked
/// status it went back to, its claim kept. A run left open keeps the task
/// held; one another writer ended moved it as that end says.
pub(super) fn undone_task(
    run_state: RunAfterUndo,
    from: TaskStatus,
) -> (Option<bool>, Option<TaskStatus>) {
    match (run_state, from) {
        (RunAfterUndo::Folded, TaskStatus::Review | TaskStatus::Blocked) => {
            (Some(false), Some(from))
        }
        (
            RunAfterUndo::Folded,
            TaskStatus::Draft
            | TaskStatus::Ready
            | TaskStatus::Claimed
            | TaskStatus::InProgress
            | TaskStatus::Done
            | TaskStatus::Archived,
        ) => (Some(true), None),
        (RunAfterUndo::Open, _) => (Some(false), None),
        (RunAfterUndo::Refused, _) => (None, None),
        (RunAfterUndo::EndedAlready, _) => (None, None),
    }
}

pub struct StartBoardTask {
    begin: Arc<BeginBoardRun>,
    launch: LaunchBegunRun,
}

impl StartBoardTask {
    /// A start writing to `store` (which stops writing once `cancelled` is
    /// set), its undo writing to `undo_store` (the same board, never
    /// cancelled), launching with `launcher`.
    ///
    /// # Errors
    /// Either store writes as an invalid identity, or the two write as
    /// different ones.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        undo_store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
        launcher: Option<Arc<dyn CoordinatorLaunch>>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Result<Self, ProjectBoardError> {
        let begin = Arc::new(BeginBoardRun::new(store, undo_store, clock, cells)?);
        let launch = LaunchBegunRun::new(Arc::clone(&begin) as _, launcher, cancelled);
        Ok(Self { begin, launch })
    }

    /// This start, asking `liveness` whether a coordinator still runs
    /// before it undoes anything (`None`: it can see none).
    #[must_use]
    pub fn seeing(self, liveness: Option<Arc<dyn RunLiveness>>) -> Self {
        let launch = self.launch.seeing(liveness);
        Self { launch, ..self }
    }

    /// The task `request` names, started and its coordinator launched.
    ///
    /// # Errors
    /// No launcher, or one that may not launch now; the begin's refusals
    /// ([`BeginBoardRun::begin`]); a launch that failed or a call cancelled
    /// after the write, undone as far as it could be.
    pub fn execute(&self, request: &TaskStart) -> Result<StartedTask, ProjectBoardError> {
        self.execute_noted(request, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the start's write (its attempts, and the task's status as last read
    /// and as written), the version it names, and, as `start`, what it
    /// started and undid, as far as it got: counts and kinds only.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        request: &TaskStart,
        facts: Option<&mut OpFacts>,
    ) -> Result<StartedTask, ProjectBoardError> {
        let Some(facts) = facts else {
            return self.started(request, None, None);
        };
        facts.plan_version = Some(u64::from(request.plan_version));
        let mut start = StartFacts {
            plan_version: facts.plan_version,
            ..StartFacts::default()
        };
        let result = self.started(request, Some(&mut *facts), Some(&mut start));
        facts.start = Some(start);
        result
    }

    /// [`Self::execute_noted`]'s work, its write noted in `op` and what it
    /// started in `facts`, when given.
    fn started(
        &self,
        request: &TaskStart,
        op: Option<&mut OpFacts>,
        facts: Option<&mut StartFacts>,
    ) -> Result<StartedTask, ProjectBoardError> {
        self.launch.may_launch()?;
        let begun = self.begin.begin(request, op)?;
        let (begun, coordinator) = self.launch.launch(begun, facts)?;
        let (task, run) = begun.written.started.into_writes();
        Ok(StartedTask {
            task,
            run,
            coordinator,
            not_applied: begun.written.not_applied,
            claim: begun.claim,
        })
    }
}

#[cfg(test)]
#[path = "start_board_task_tests.rs"]
mod tests;
