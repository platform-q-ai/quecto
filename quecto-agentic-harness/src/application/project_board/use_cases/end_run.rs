//! Records how a task's started run ended (S4c): nothing else does, as
//! the swarm never writes the board. Once the swarm has ended (its parent
//! closes it), the claim's holder records the outcome; the same ends a run
//! no swarm will finish, as when a start's coordinator never launched and
//! its undo could not end the run, or the process died between the start's
//! write and its launch. Without it a run would hold its task for good.
//!
//! The claim's holder ends it as `succeeded`, `stopped` or `failed`, folded
//! back from its swarm's board as this front end kept it when the run's
//! coordinator left its container ([`SwarmRunReports`], dropped once
//! recorded): where each task item stood,
//! with owners and evidence, and what it cost, mapped as a fold-back maps
//! them ([`run_items`], [`cost_of`]). With no board kept (the CLI, a run
//! another session launched, a board that could not be read whole, a
//! process restarted between the teardown and this end), it is folded
//! back as a swarm that ended so with nothing reported is: every task item
//! as todo, its cost unknown. A run ends as `succeeded` only when every
//! task item is done there (`items_not_done` otherwise, from the domain),
//! so no success is recorded over items the board shows undone. A run
//! that would leave the board past its bytes is ended once more without
//! its evidence, as a fold-back is. Never while its swarm is still running:
//! while this front end sees it running ([`RunLiveness`]: its session
//! launched the coordinator, and it still runs) the end is refused
//! (`run_live`); a coordinator another session launched is not seen. As
//! `lost`
//! it is ended by whoever the domain lets ([`end_lost`]: the holder of a
//! live claim, or anyone when nobody claims the task; one who finds the
//! claim expired takes the task over first). With `expected_updated`, the
//! task must still be as the caller read it. Only the run is written: the
//! task is left for its holder to move. Blocking: a lost write pauses the
//! calling thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit_run, task_on};
use super::board_view;
use crate::application::project_board::dto::{
    EndedRun, LivenessCheck, ProjectBoardError, RunEnd, RunEndOutcome, RunReport,
};
use crate::application::project_board::dto::{FoldBackFacts, FoldedBack};
use crate::application::project_board::dto::{SwarmEnd, SwarmRunView};
use crate::application::project_board::fold_back::{Evidence, FoldBackItems, cost_of, run_items};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardStoreError, BoardTextCells, ProjectBoardStore, RunLiveness,
    SwarmRunReports,
};
use crate::application::project_board::start::NotedFoldBack;
use crate::application::project_board::telemetry::{OpFacts, ReportSeen, counted, summed};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::claims::claim_facts;
use crate::domain::project_board::services::runs::{
    HeldEnd, Reported, end_held, end_lost, run_and_plan,
};
use crate::domain::project_board::value_objects::code_ref::CommitId;
use crate::domain::project_board::value_objects::rule_error::SwarmReport;
use crate::domain::project_board::value_objects::telemetry::RunOutcomeKind;

pub struct EndRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    cells: Arc<dyn BoardTextCells>,
    /// Whether a run's swarm still runs here (`None`: this front end can
    /// see none).
    liveness: Option<Arc<dyn RunLiveness>>,
    /// How a run's swarm ended, as this front end kept its board (`None`:
    /// it keeps none).
    reports: Option<Arc<dyn SwarmRunReports>>,
    /// The store's committer, checked valid when built: who ends the run.
    actor: Identity,
}

impl EndRun {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
        liveness: Option<Arc<dyn RunLiveness>>,
        reports: Option<Arc<dyn SwarmRunReports>>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            cells,
            liveness,
            reports,
            actor,
        })
    }

    /// The run `request` names, ended as it says, as written.
    ///
    /// # Errors
    /// A run whose swarm still runs here; no such task, run or plan
    /// version, or one of their files set aside; a task stamped ahead, or
    /// changed since it was read; the rules' (an
    /// actor who may not end it, a run that has ended); the clock's; a
    /// board that kept moving; the store's.
    pub fn execute(&self, request: &RunEnd) -> Result<EndedRun, ProjectBoardError> {
        self.execute_noted(request, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the run and the outcome it names, the write's attempts, and the
    /// task's status and claim as last read.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        request: &RunEnd,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<EndedRun, ProjectBoardError> {
        let started = facts.as_ref().map(|_| self.clock.elapsed());
        if let Some(facts) = facts.as_deref_mut() {
            facts.run = Some(u64::from(request.run));
            facts.run_outcome = Some(request.outcome.outcome());
        }
        let (id, number) = (&request.task, request.run);
        // A swarm still running here is stopped before its run is ended.
        let running = self
            .liveness
            .as_ref()
            .and_then(|liveness| liveness.running_coordinator(id, number));
        if let Some(agent) = running {
            return Err(ProjectBoardError::RunLive {
                task: id.clone(),
                run: number,
                agent,
            });
        }
        // Read once, before any write: the board its swarm left, if kept.
        let report = self
            .reports
            .as_ref()
            .and_then(|reports| reports.ended_run(id, number));
        let message = format!(
            "board: run {number} of {} ended, {}",
            id.as_str(),
            request.outcome.as_str()
        );
        let mut items = None;
        let mut written = self.write(
            request,
            &message,
            (report.as_ref(), Evidence::Kept),
            &mut items,
            &mut facts,
        );
        // Only a run past the board's bytes is ended again, smaller (an
        // allowlist), as a fold-back is: any other refusal is the caller's.
        if let Err(ProjectBoardError::Store(BoardStoreError::WriteTooLarge(_))) = &written {
            let first = facts
                .as_deref()
                .map(|facts| (facts.attempts, facts.lost, facts.lock_races));
            written = self.write(
                request,
                &message,
                (report.as_ref(), Evidence::Left),
                &mut items,
                &mut facts,
            );
            if let (Some(facts), Some((attempts, lost, races))) = (facts.as_deref_mut(), first) {
                facts.attempts = summed(attempts, facts.attempts);
                facts.lost = summed(lost, facts.lost);
                facts.lock_races = summed(races, facts.lock_races);
                assert!(facts.attempts.is_some(), "a noted write notes its attempts");
            }
        }
        // An end that wrote says what it found of the board its swarm left
        // (#2663), whatever its outcome: kept, none kept, or no keeper to
        // ask.
        if let (Some(facts), Ok(_)) = (facts.as_deref_mut(), &written) {
            facts.swarm_report = Some(match (&self.reports, &report) {
                (None, _) => ReportSeen::NotChecked,
                (Some(_), Some(_)) => ReportSeen::Kept,
                (Some(_), None) => ReportSeen::NoneKept,
            });
        }
        // A kept swarm board folded onto the run: its own fold-back record
        // (#2667), as a fold-back's, whatever the outcome, once the run
        // written holds the items folded from it.
        if let (Some(facts), Some(started), Some(items), Some(_), Ok([BoardArtifact::Run(run)])) =
            (facts, started, items, &report, written.as_deref())
            && run.fields().items == items.items
        {
            facts.fold_back = Some(NotedFoldBack {
                task: id.clone(),
                run: number,
                result: Ok(FoldedBack::Folded(run.clone())),
                facts: folded_facts(run, &items, facts),
                elapsed: self.clock.elapsed().saturating_sub(started),
            });
        }
        match written?.as_slice() {
            [BoardArtifact::Run(run)] => {
                assert_eq!(
                    run.fields().number,
                    number,
                    "the run ended is the one named"
                );
                // Whether this front end looked for the swarm still running.
                let liveness = match self.liveness {
                    Some(_) => LivenessCheck::Checked,
                    None => LivenessCheck::NotChecked,
                };
                // A board recorded need not be kept: the run has ended.
                let report = match (&report, &self.reports, request.outcome) {
                    (
                        Some(_),
                        Some(reports),
                        RunEndOutcome::Succeeded | RunEndOutcome::Stopped | RunEndOutcome::Failed,
                    ) => {
                        reports.recorded(id, number);
                        RunReport::Kept
                    }
                    (_, _, RunEndOutcome::Lost) | (None, _, _) | (_, None, _) => {
                        RunReport::Unreported
                    }
                };
                Ok(EndedRun {
                    run: run.clone(),
                    liveness,
                    report,
                })
            }
            other => panic!("an end_run writes its run alone, not {other:?}"),
        }
    }

    /// One pass at ending the run, as `request` says, with `report`'s
    /// items, owners and cost when there is one (its evidence kept or left
    /// out as `evidence` says), noting in `facts`, when given, the write's
    /// attempts and the task as last read.
    fn write(
        &self,
        request: &RunEnd,
        message: &str,
        (report, evidence): (Option<&SwarmRunView>, Evidence),
        kept: &mut Option<FoldBackItems>,
        facts: &mut Option<&mut OpFacts>,
    ) -> Result<Vec<BoardArtifact>, ProjectBoardError> {
        let (id, number) = (&request.task, request.run);
        let noting = facts.is_some();
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let mut read = None;
        let written = commit_run(
            store,
            clock,
            message,
            facts.as_deref_mut(),
            |board, when| {
                // This attempt's read, or none: never an earlier one's.
                read = None;
                let task = task_on(board, id)?;
                if noting {
                    read = Some((task.fields().status, claim_facts(task, actor, &when.now)));
                }
                when.may_build_on(task)?;
                match &request.expected_updated {
                    Some(expected) if *expected != task.fields().updated => {
                        return Err(ProjectBoardError::Stale {
                            id: id.clone(),
                            expected: expected.clone(),
                            updated: task.fields().updated.clone(),
                        });
                    }
                    Some(_) | None => {}
                }
                let runs = board_view::runs_of(board, id)?;
                let plans = board_view::plans_of(board, id)?;
                let (run, plan) = run_and_plan(&runs, &plans, number)?;
                // This attempt's items from the board its swarm left, for
                // every outcome (#2663): never an earlier attempt's.
                *kept = report.map(|view| run_items(plan, view, &*self.cells, evidence));
                let mapped = kept.as_ref();
                let held = |end: HeldEnd, swarm: SwarmEnd| {
                    let reported = match (report, mapped) {
                        // What the swarm's board showed as it ended.
                        (Some(view), Some(items)) => Reported {
                            items: items.items.clone(),
                            cost: Some(cost_of(view)),
                            report: SwarmReport::Kept,
                        },
                        // Nothing reported: every item as a fold-back maps
                        // a swarm that reported nothing (a start's undo's
                        // included), its cost unknown.
                        (None, _) | (Some(_), None) => {
                            let nothing = SwarmRunView {
                                end: swarm,
                                items: vec![],
                                observed_tokens: 0,
                            };
                            Reported {
                                items: run_items(plan, &nothing, &*self.cells, Evidence::Left)
                                    .items,
                                cost: None,
                                report: SwarmReport::NoneKept,
                            }
                        }
                    };
                    end_held(task, (run, plan), end, reported, actor, &when.now)
                };
                let ended = match request.outcome {
                    RunEndOutcome::Succeeded => held(HeldEnd::Succeeded, SwarmEnd::Succeeded)?,
                    RunEndOutcome::Stopped => held(HeldEnd::Stopped, SwarmEnd::Cancelled)?,
                    RunEndOutcome::Failed => held(HeldEnd::Failed, SwarmEnd::Failed)?,
                    RunEndOutcome::Lost => end_lost(task, run, actor, &when.now)?,
                };
                Ok(vec![BoardArtifact::Run(ended)])
            },
        );
        if let Some(facts) = facts.as_deref_mut() {
            facts.status_from = read.map(|(status, _)| status);
            facts.claim = read.and_then(|(_, claim)| claim);
        }
        written
    }
}

/// What an end's fold-back of a kept swarm board saw, for its record:
/// the run as written, the `items` it mapped, and the write's own `facts`.
fn folded_facts(run: &Run, items: &FoldBackItems, facts: &OpFacts) -> FoldBackFacts {
    let fields = run.fields();
    FoldBackFacts {
        run_outcome: fields.outcome.map(RunOutcomeKind::of),
        items: Some(items.counts()),
        unmatched: Some(counted(items.unmatched)),
        extra: Some(counted(items.extra)),
        evidence: Some(counted(items.evidence)),
        evidence_dropped: Some(counted(items.dropped)),
        cost_tokens: fields.cost,
        attempts: facts.attempts,
        lost: facts.lost,
        lock_races: facts.lock_races,
        bytes: Some(counted(fields.text_bytes())),
        head: facts
            .head
            .as_ref()
            .and_then(|head| CommitId::parse(&head.0).ok()),
    }
}

#[cfg(test)]
#[path = "end_run_tests.rs"]
mod tests;
