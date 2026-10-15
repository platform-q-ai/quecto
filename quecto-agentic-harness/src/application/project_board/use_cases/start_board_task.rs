//! Starts a task (S4c): the task claimed if it is ready (or its claim
//! renewed when the starter holds it), moved to in progress, and run 1 of
//! its accepted plan begun, written together in one change, task first;
//! then the run's swarm coordinator launched with the plan's brief.
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
//! - **A failed launch is undone.** The run is folded back as failed, and
//!   only then (a held task leaves its holder only once its run ended),
//!   when the start took the claim, the task is moved back to ready,
//!   releasing it (one held in review or blocked goes back there). The
//!   refusal says how far that got; a run left open is for `end_run`. A
//!   call cancelled after the write launches nothing, and one cancelled
//!   during the launch stops it; either is undone the same way, as is a
//!   launch that panicked. The undo writes through a store that ignores
//!   the call's cancel, so a cancel never strands a run.
//!
//! Blocking: the store's calls, a pause before a retried write and the
//! launch itself hold the calling thread.
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::board_change::{checked_writer, commit, owned, task_on};
use super::{ChangeBoardTask, FoldBackRun, board_view};
use crate::application::project_board::dto::{
    BoardOp, FoldBackFacts, FoldedBack, ProjectBoardError, Restored, RunAfterUndo, TaskStart,
};
use crate::application::project_board::fold_back::{SwarmEnd, SwarmRunView};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardTextCells, CoordinatorLaunch, CoordinatorRequest,
    LaunchRefused, ProjectBoardStore, RunLiveness,
};
use crate::application::project_board::start::{StartFacts, StartedTask, UndoFoldBack};
use crate::application::project_board::swarm_brief::{
    BriefRefusal, MAX_BRIEF_BYTES, SwarmBrief, brief_of, render,
};
use crate::application::project_board::telemetry::{OpFacts, counted};
use crate::application::tool_panic_scope::catch_in_call_site;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::services::plans::as_accepted;
use crate::domain::project_board::services::starts::{Started, start};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::StartClaim;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// Why a start launched nothing when its call was cancelled after its
/// write.
const CANCELLED: &str = "the call was cancelled before the coordinator launched";

/// Why a start whose launch panicked launched nothing.
const PANICKED: &str = "the coordinator's launch failed unexpectedly (it panicked)";

pub struct StartBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who starts.
    actor: Identity,
    /// `None`: this session may not spawn, so nothing is started.
    launcher: Option<Arc<dyn CoordinatorLaunch>>,
    /// Set when the call this start serves was cancelled.
    cancelled: Option<Arc<AtomicBool>>,
    /// The undo's, over a store that ignores the call's cancel.
    fold_back: FoldBackRun,
    change: ChangeBoardTask,
    /// Whether the run's coordinator still runs here, asked before an
    /// undo (`None`: this front end can see none).
    liveness: Option<Arc<dyn RunLiveness>>,
}

/// A start as its write left it: the start, its brief and the brief's
/// text, as the last attempt built them.
struct Written {
    started: Started,
    brief: SwarmBrief,
    text: String,
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
        let actor = checked_writer(&*store)?;
        assert_eq!(
            checked_writer(&*undo_store)?,
            actor,
            "a start and its undo write as one identity"
        );
        let fold_back = FoldBackRun::new(Arc::clone(&undo_store), Arc::clone(&clock), cells)?;
        let change = ChangeBoardTask::new(undo_store, Arc::clone(&clock))?;
        Ok(Self {
            store,
            clock,
            actor,
            launcher,
            cancelled,
            fold_back,
            change,
            liveness: None,
        })
    }

    /// This start, asking `liveness` whether a coordinator still runs
    /// before it undoes anything (`None`: it can see none).
    #[must_use]
    pub fn seeing(self, liveness: Option<Arc<dyn RunLiveness>>) -> Self {
        Self { liveness, ..self }
    }

    /// The task `request` names, started and its coordinator launched.
    ///
    /// # Errors
    /// No launcher, or one that may not launch now; no such task, or its
    /// file, one of its plans' or runs' set aside; a task stamped ahead;
    /// another plan accepted than the one confirmed; the rules' (no
    /// accepted plan, a task not startable or held by another, a run still
    /// going); a plan that cannot brief a swarm; the clock's; a board that
    /// kept moving; the store's; a launch that failed or a call cancelled
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
        mut facts: Option<&mut StartFacts>,
    ) -> Result<StartedTask, ProjectBoardError> {
        let launcher = self
            .launcher
            .as_ref()
            .filter(|launcher| launcher.may_launch())
            .ok_or(ProjectBoardError::NoLauncher)?;
        let Written {
            started,
            brief,
            text,
        } = self.write(request, op)?;
        let run = started.run().fields().number;
        let claim = match started.took_claim() {
            true => StartClaim::Fresh,
            false => StartClaim::Held,
        };
        if let Some(facts) = facts.as_deref_mut() {
            facts.run = Some(u64::from(run));
            facts.members = brief.member_limit.map(u64::from);
            let roles = started.plan().fields().team.as_ref();
            facts.roles = Some(counted(roles.map_or(0, |team| team.roles.len())));
            facts.items = Some(counted(brief.items.len()));
            facts.not_applied = Some(counted(brief.not_applied.len()));
            facts.claim = Some(claim);
        }
        // A call cancelled after its write launches nothing: it is undone.
        let never = AtomicBool::new(false);
        let cancel = self.cancelled.as_deref().unwrap_or(&never);
        if cancel.load(Ordering::SeqCst) {
            let refused = LaunchRefused::because(CANCELLED);
            return Err(self.undone(&request.task, &started, refused, facts));
        }
        let coordinator = brief.coordinator.as_ref();
        let launch = CoordinatorRequest {
            task: request.task.clone(),
            run,
            plan_version: brief.plan_version,
            brief: text,
            model: coordinator.map(|choice| choice.model.clone()),
            effort: coordinator.map(|choice| choice.effort),
        };
        let before = facts.as_ref().map(|_| self.clock.elapsed());
        // A launch that panics is undone as one that failed: the write it
        // follows is never left behind. The catch holds while the call's
        // scope is open; a panic after a cancelled call's scope closed (its
        // work left running) is outside any scope, and the hook ends the
        // process, as for any carried work (`call_work`'s limit).
        let launched = catch_in_call_site(AssertUnwindSafe(|| launcher.launch(&launch, cancel)))
            .unwrap_or_else(|_| Err(LaunchRefused::because(PANICKED)));
        if let (Some(facts), Some(before)) = (facts.as_mut(), before) {
            let took = self.clock.elapsed().saturating_sub(before);
            facts.launch_us = Some(u64::try_from(took.as_micros()).unwrap_or(u64::MAX));
        }
        match launched {
            Ok(coordinator) => {
                let (task, run) = started.into_writes();
                Ok(StartedTask {
                    task,
                    run,
                    coordinator,
                    not_applied: brief.not_applied,
                    claim,
                })
            }
            Err(refused) => Err(self.undone(&request.task, &started, refused, facts)),
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
                let started = start(task, &runs, plan, actor, &when.now)?;
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

    /// The refusal of a start whose coordinator did not launch, once the
    /// start is undone as far as it can be: its run folded back as failed
    /// (or found ended already), then, only once the run has ended, the
    /// task moved back to ready when the start took the claim, or to the
    /// review or blocked status it was held in. A coordinator that may
    /// still be running (one that came up wrong and could not be stopped)
    /// leaves the start as it is: its run open, for `end_run` once it is
    /// stopped, and the claim held. Noted in `facts`, when given, with the
    /// undo's fold-back.
    fn undone(
        &self,
        id: &Slug,
        started: &Started,
        refused: LaunchRefused,
        facts: Option<&mut StartFacts>,
    ) -> ProjectBoardError {
        let run = started.run().fields().number;
        let LaunchRefused {
            reason,
            still_running,
        } = refused;
        // A coordinator that came up anyway (a launch that panicked, was
        // cancelled or timed out after spawn began) may still be running
        // where this front end can see it: undone the same as one the
        // launch said it could not stop.
        let still_running = still_running.or_else(|| {
            let liveness = self.liveness.as_ref();
            liveness.and_then(|liveness| liveness.running_coordinator(id, run))
        });
        let (run_state, undo) = match &still_running {
            // Never fold back, nor release, under a swarm that may run.
            Some(_) => (RunAfterUndo::Open, None),
            None => {
                let (run_state, undo) = self.folded_back(id, run, facts.is_some());
                (run_state, Some(undo))
            }
        };
        // A held task leaves progress only once its run has ended.
        let ended = matches!(run_state, RunAfterUndo::Folded | RunAfterUndo::EndedAlready);
        let moved_to = |status| ended && self.change.execute(id, &BoardOp::Move(status)).is_ok();
        let released = started.took_claim().then(|| moved_to(TaskStatus::Ready));
        // A task held in review or blocked goes back there; one claimed or
        // in progress stays in progress (no row of the table moves it back).
        let restored = match started.from() {
            status @ (TaskStatus::Review | TaskStatus::Blocked) => Some(Restored {
                status,
                moved: moved_to(status),
            }),
            TaskStatus::Draft
            | TaskStatus::Ready
            | TaskStatus::Claimed
            | TaskStatus::InProgress
            | TaskStatus::Done
            | TaskStatus::Archived => None,
        };
        if let Some(facts) = facts {
            facts.run_state = Some(run_state);
            facts.released = released;
            facts.fold_back = undo;
        }
        ProjectBoardError::LaunchFailed {
            reason,
            run,
            run_state,
            released,
            restored,
            still_running,
        }
    }

    /// Run `run` of task `id` folded back as failed, as a swarm that
    /// reported nothing: where that left it, and the fold-back, timed and
    /// with its facts when `noting`.
    fn folded_back(&self, id: &Slug, run: u32, noting: bool) -> (RunAfterUndo, UndoFoldBack) {
        let failed = SwarmRunView {
            end: SwarmEnd::Failed,
            items: vec![],
            observed_tokens: 0,
        };
        let before = self.clock.elapsed();
        let mut facts = FoldBackFacts::default();
        let folded = self
            .fold_back
            .execute_noted(id, run, &failed, noting.then_some(&mut facts));
        let run_state = match &folded {
            Ok(FoldedBack::Folded(_)) => RunAfterUndo::Folded,
            Ok(FoldedBack::Ended(_)) => RunAfterUndo::EndedAlready,
            Err(_) => RunAfterUndo::Open,
        };
        let undo = UndoFoldBack {
            task: id.clone(),
            run,
            result: folded,
            facts,
            elapsed: self.clock.elapsed().saturating_sub(before),
        };
        (run_state, undo)
    }
}

#[cfg(test)]
#[path = "start_board_task_tests.rs"]
mod tests;
