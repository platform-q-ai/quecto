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
//! - **A failed launch is undone.** The run is folded back as failed, and
//!   only then (a held task leaves its holder only once its run ended),
//!   when the start took the claim, the task is moved back to ready,
//!   releasing it. The refusal says how far that got.
//!
//! Blocking: the store's calls, a pause before a retried write and the
//! launch itself hold the calling thread.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, owned, task_on};
use super::{ChangeBoardTask, FoldBackRun, board_view};
use crate::application::project_board::dto::{BoardOp, ProjectBoardError, TaskStart};
use crate::application::project_board::fold_back::{SwarmEnd, SwarmRunView};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardTextCells, CoordinatorLaunch, CoordinatorRequest,
    LaunchRefused, ProjectBoardStore,
};
use crate::application::project_board::start::{StartFacts, StartedTask};
use crate::application::project_board::swarm_brief::{SwarmBrief, brief_of, render};
use crate::application::project_board::telemetry::counted;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::services::plans::as_accepted;
use crate::domain::project_board::services::starts::{Started, start};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::StartClaim;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct StartBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who starts.
    actor: Identity,
    /// `None`: this session may not spawn, so nothing is started.
    launcher: Option<Arc<dyn CoordinatorLaunch>>,
    fold_back: FoldBackRun,
    change: ChangeBoardTask,
}

impl StartBoardTask {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
        launcher: Option<Arc<dyn CoordinatorLaunch>>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        let fold_back = FoldBackRun::new(Arc::clone(&store), Arc::clone(&clock), cells)?;
        let change = ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?;
        Ok(Self {
            store,
            clock,
            actor,
            launcher,
            fold_back,
            change,
        })
    }

    /// The task `request` names, started and its coordinator launched.
    ///
    /// # Errors
    /// No launcher; no such task, or its file, one of its plans' or runs'
    /// set aside; a task stamped ahead; another plan accepted than the one
    /// confirmed; the rules' (no accepted plan, a task not startable or
    /// held by another, a run still going); a plan that cannot brief a
    /// swarm; the clock's; a board that kept moving; the store's; a launch
    /// that failed, undone as far as it could be.
    pub fn execute(&self, request: &TaskStart) -> Result<StartedTask, ProjectBoardError> {
        self.execute_noted(request, None)
    }

    /// [`Self::execute`], noting in `facts`, when given, the start's write
    /// and what it started: counts and kinds only.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        request: &TaskStart,
        mut facts: Option<&mut StartFacts>,
    ) -> Result<StartedTask, ProjectBoardError> {
        let launcher = self
            .launcher
            .as_ref()
            .ok_or(ProjectBoardError::NoLauncher)?;
        if let Some(facts) = facts.as_deref_mut() {
            facts.plan_version = Some(u64::from(request.plan_version));
        }
        let (started, brief) = self.write(request, facts.as_deref_mut())?;
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
        let coordinator = brief.coordinator.as_ref();
        let launch = CoordinatorRequest {
            task: request.task.clone(),
            run,
            plan_version: brief.plan_version,
            brief: render(&brief),
            model: coordinator.map(|choice| choice.model.clone()),
            effort: coordinator.map(|choice| choice.effort),
        };
        let before = facts.as_ref().map(|_| self.clock.elapsed());
        let launched = launcher.launch(&launch);
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
            Err(refused) => Err(self.undone(&request.task, &started, refused)),
        }
    }

    /// The task and run 1 written in one change, with the start and its
    /// brief as the last attempt built them.
    fn write(
        &self,
        request: &TaskStart,
        facts: Option<&mut StartFacts>,
    ) -> Result<(Started, SwarmBrief), ProjectBoardError> {
        let id = &request.task;
        let message = format!("board: start {}", id.as_str());
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let noting = facts.is_some();
        let mut read = None;
        let mut built = None;
        let mut op = facts.as_ref().map(|_| Default::default());
        let written = commit(store, clock, &message, op.as_mut(), |board, when| {
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
            let brief = brief_of(&started, &when.now).map_err(ProjectBoardError::from)?;
            let (task, run) = started.clone().into_writes();
            built = Some((started, brief));
            Ok(vec![BoardArtifact::Task(task), BoardArtifact::Run(run)])
        });
        if let Some(facts) = facts {
            let mut op = op.unwrap_or_default();
            op.status_from = read;
            op.status_to = written.as_ref().ok().map(|_| TaskStatus::InProgress);
            facts.op = op;
        }
        let written = written?;
        let (started, brief) = built.expect("a written start was built");
        let (task, run) = started.clone().into_writes();
        assert_eq!(
            written,
            [BoardArtifact::Task(task), BoardArtifact::Run(run)],
            "the start written is the last one built"
        );
        Ok((started, brief))
    }

    /// The refusal of a start whose coordinator did not launch, once the
    /// start is undone as far as it can be: its run folded back as failed
    /// (or found ended already), then, only when the start took the claim
    /// and the run is ended, the task moved back to ready.
    fn undone(&self, id: &Slug, started: &Started, refused: LaunchRefused) -> ProjectBoardError {
        let run = started.run().fields().number;
        let failed = SwarmRunView {
            end: SwarmEnd::Failed,
            items: vec![],
            observed_tokens: 0,
        };
        let folded = self.fold_back.execute(id, run, &failed).is_ok();
        let released = started.took_claim().then(|| {
            folded
                && self
                    .change
                    .execute(id, &BoardOp::Move(TaskStatus::Ready))
                    .is_ok()
        });
        ProjectBoardError::LaunchFailed {
            reason: refused.0,
            run,
            folded,
            released,
        }
    }
}

#[cfg(test)]
#[path = "start_board_task_tests.rs"]
mod tests;
