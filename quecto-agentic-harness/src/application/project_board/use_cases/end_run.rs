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
//! as todo, its cost unknown. A stop, a failure or a loss is recorded
//! from the board a run had reached as its owner ended its coordinator
//! while it still ran or was paused, when that is what was kept (#2674);
//! a success never is. A run ends as `succeeded` only when every
//! task item is done there (`items_not_done` otherwise, from the domain),
//! so no success is recorded over items the board shows undone; nor while
//! a revision the recorded items' evidence names is on no branch or tag of
//! the board's remote ([`RemoteRevisions`], `not_pushed` from the domain,
//! #2666), so no success is recorded over work that never left the swarm's
//! container (a remote that cannot be read refuses it too). A run
//! that would leave the board past its bytes is ended once more without
//! its evidence, as a fold-back is. Never while its swarm is still running:
//! while this front end sees it running ([`RunLiveness`]: its session
//! launched the coordinator, and it still runs) the end is refused
//! (`run_live`); a coordinator another session launched is not seen. As
//! `lost`
//! it is ended by whoever the domain lets ([`end_lost`]: the holder of a
//! live claim, or anyone when nobody claims the task; one who finds the
//! claim expired takes the task over first), recording what is known of
//! it as a held end does: its swarm's kept board, or every item as todo
//! (#2657). With `expected_updated`, the
//! task must still be as the caller read it. Only the run is written: the
//! task is left for its holder to move. Blocking: a lost write pauses the
//! calling thread before it is tried again.
use std::sync::Arc;

use super::board_change::{When, checked_now, checked_writer, commit_run, task_on};
use super::board_view;
use crate::application::project_board::dto::{
    EndedRun, LivenessCheck, ProjectBoardError, RunEnd, RunEndOutcome, RunReport,
};
use crate::application::project_board::dto::{SwarmEnd, SwarmRunView};
use crate::application::project_board::fold_back::{
    Evidence, EvidenceRevisions, checks_commit, cost_of, evidence_revisions, run_items,
};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardStoreError, BoardTextCells, ProjectBoardStore, RemoteRevisions,
    RunLiveness, SwarmRunReports,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::claims::claim_facts;
use crate::domain::project_board::services::runs::{
    HeldEnd, Reported, end_held, end_lost, run_and_plan,
};
use crate::domain::project_board::value_objects::rule_error::SwarmReport;

pub struct EndRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    cells: Arc<dyn BoardTextCells>,
    /// Which revisions the board's remote holds: a success is recorded
    /// only over evidence that left the swarm's container (#2666).
    revisions: Arc<dyn RemoteRevisions>,
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
        revisions: Arc<dyn RemoteRevisions>,
        liveness: Option<Arc<dyn RunLiveness>>,
        reports: Option<Arc<dyn SwarmRunReports>>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            cells,
            revisions,
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
        // A success only from a run that ended there; any other end from
        // the board a run had reached as its owner stopped it too (#2674).
        let report = self.reports.as_ref().and_then(|reports| {
            let ended = reports.ended_run(id, number);
            match request.outcome {
                RunEndOutcome::Succeeded => ended,
                RunEndOutcome::Stopped | RunEndOutcome::Failed | RunEndOutcome::Lost => {
                    ended.or_else(|| reports.left_run(id, number))
                }
            }
        });
        let message = format!(
            "board: run {number} of {} ended, {}",
            id.as_str(),
            request.outcome.as_str()
        );
        // What the remote answered, asked at most once for every pass, and
        // asked before the write begins (#2666 round 1): never between an
        // attempt's read and its write, so the change is judged at the
        // time it is written.
        let mut asked = None;
        self.asked_ahead(request, report.as_ref(), &mut asked);
        let mut written = self.write(
            request,
            &message,
            report.as_ref(),
            (Evidence::Kept, &mut asked),
            &mut facts,
        );
        // Only a run past the board's bytes is ended again, smaller (an
        // allowlist), as a fold-back is: any other refusal is the caller's.
        if let Err(ProjectBoardError::Store(BoardStoreError::WriteTooLarge(_))) = &written {
            let first = facts.as_deref().map(|facts| (facts.attempts, facts.lost));
            written = self.write(
                request,
                &message,
                report.as_ref(),
                (Evidence::Left, &mut asked),
                &mut facts,
            );
            if let (Some(facts), Some((attempts, lost))) = (facts, first) {
                let sum = |one: Option<u64>, two: Option<u64>| {
                    Some(one.unwrap_or(0).saturating_add(two.unwrap_or(0)))
                };
                facts.attempts = sum(attempts, facts.attempts);
                facts.lost = sum(lost, facts.lost);
            }
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
                // A loss records the kept board too (#2657).
                let report = match (&report, &self.reports) {
                    (Some(_), Some(reports)) => {
                        reports.recorded(id, number);
                        RunReport::Kept
                    }
                    (None, _) | (_, None) => RunReport::Unreported,
                };
                // How many evidence revisions a success left unchecked, so a
                // silent pass shows (#2666 round 2).
                let unchecked = match (request.outcome, &asked) {
                    (RunEndOutcome::Succeeded, Some((_, Ok(judged)))) => {
                        Some(u64::try_from(judged.unchecked).unwrap_or(u64::MAX))
                    }
                    (RunEndOutcome::Succeeded, Some((_, Err(_))) | None)
                    | (RunEndOutcome::Stopped | RunEndOutcome::Failed | RunEndOutcome::Lost, _) => {
                        None
                    }
                };
                Ok(EndedRun {
                    run: run.clone(),
                    liveness,
                    report,
                    unchecked,
                })
            }
            other => panic!("an end_run writes its run alone, not {other:?}"),
        }
    }

    /// One pass at ending the run, as `request` says, with `report`'s
    /// items, owners and cost when there is one (its evidence kept or left
    /// out as `evidence` says), noting in `facts`, when given, the write's
    /// attempts and the task as last read. A success over a kept report
    /// counts the revisions its evidence names that the remote does not
    /// hold, asking the remote only for revisions `asked` holds no answer
    /// for.
    fn write(
        &self,
        request: &RunEnd,
        message: &str,
        report: Option<&SwarmRunView>,
        (evidence, asked): (Evidence, &mut Asked),
        facts: &mut Option<&mut OpFacts>,
    ) -> Result<Vec<BoardArtifact>, ProjectBoardError> {
        let id = &request.task;
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
                let ended = self.ended(request, report, (evidence, &mut *asked), board, when)?;
                Ok(vec![BoardArtifact::Run(ended)])
            },
        );
        if let Some(facts) = facts.as_deref_mut() {
            facts.status_from = read.map(|(status, _)| status);
            facts.claim = read.and_then(|(_, claim)| claim);
        }
        written
    }

    /// The run `request` names as `board` and `when` end it, with
    /// `report`'s items, owners and cost when there is one (its evidence
    /// kept or left out as `evidence` says). A success over a kept report
    /// is first judged by every other rule (its claim, its run, its
    /// items); only one that passes them asks the remote, once, which of
    /// the commits its evidence names it holds (#2666), the answer kept in
    /// `asked` for a later attempt or pass.
    fn ended(
        &self,
        request: &RunEnd,
        report: Option<&SwarmRunView>,
        (evidence, asked): (Evidence, &mut Asked),
        board: &crate::application::project_board::ports::BoardSnapshot,
        when: &When,
    ) -> Result<Run, ProjectBoardError> {
        let (id, number, actor) = (&request.task, request.run, &self.actor);
        let task = task_on(board, id)?;
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
        // What is known of the run: its swarm's board as kept, or every
        // item as a swarm that reported nothing (#2657: a loss records it
        // too).
        let reported = |swarm: SwarmEnd, unpushed: usize| {
            match report {
                // What the swarm's board showed as it ended.
                Some(view) => Reported {
                    items: run_items(plan, view, &*self.cells, evidence).items,
                    cost: Some(cost_of(view)),
                    report: SwarmReport::Kept,
                    unpushed,
                },
                // Nothing reported: every item as a fold-back maps a swarm
                // that reported nothing (a start's undo's included), its
                // cost unknown.
                None => {
                    let nothing = SwarmRunView {
                        end: swarm,
                        items: vec![],
                        observed_tokens: 0,
                    };
                    Reported {
                        items: run_items(plan, &nothing, &*self.cells, Evidence::Left).items,
                        cost: None,
                        report: SwarmReport::NoneKept,
                        unpushed: 0,
                    }
                }
            }
        };
        let held = |end: HeldEnd, swarm: SwarmEnd, unpushed: usize| {
            end_held(
                task,
                (run, plan),
                end,
                reported(swarm, unpushed),
                actor,
                &when.now,
            )
        };
        Ok(match (request.outcome, report) {
            (RunEndOutcome::Succeeded, Some(view)) => {
                // Every other rule first: the remote is asked only for an
                // end that passes them.
                held(HeldEnd::Succeeded, SwarmEnd::Succeeded, 0)?;
                let unpushed = self.unpushed(evidence_revisions(plan, view), asked)?;
                held(HeldEnd::Succeeded, SwarmEnd::Succeeded, unpushed)?
            }
            (RunEndOutcome::Succeeded, None) => held(HeldEnd::Succeeded, SwarmEnd::Succeeded, 0)?,
            (RunEndOutcome::Stopped, _) => held(HeldEnd::Stopped, SwarmEnd::Cancelled, 0)?,
            (RunEndOutcome::Failed, _) => held(HeldEnd::Failed, SwarmEnd::Failed, 0)?,
            // Nobody accounts for it: as a swarm that was lost.
            (RunEndOutcome::Lost, _) => end_lost(
                task,
                (run, plan),
                reported(SwarmEnd::Failed, 0),
                actor,
                &when.now,
            )?,
        })
    }

    /// For a success over a kept report, the remote's answer asked ahead
    /// of the write, against the board and the time as they are now, so
    /// the write's attempts find it in `asked`: best effort, as anything
    /// it finds wrong the write finds again, and refuses, noting it.
    fn asked_ahead(&self, request: &RunEnd, report: Option<&SwarmRunView>, asked: &mut Asked) {
        match (request.outcome, report) {
            (RunEndOutcome::Succeeded, Some(_)) => {}
            (RunEndOutcome::Succeeded, None)
            | (RunEndOutcome::Stopped | RunEndOutcome::Failed | RunEndOutcome::Lost, _) => return,
        }
        let (Ok(board), Ok(when)) = (self.store.read(), checked_now(&*self.clock)) else {
            return;
        };
        let _ahead = self.ended(request, report, (Evidence::Kept, asked), &board, &when);
    }

    /// How many of the commits `evidence` names the board's remote does
    /// not hold, and how many of its revisions went unchecked (naming no
    /// commit, or one too short or too long to check: [`checks_commit`]):
    /// asked once (the remote not at all when no commit is named), the
    /// answer (or why there is none) kept in `asked` for a later attempt
    /// or pass at the same evidence.
    fn unpushed(
        &self,
        evidence: EvidenceRevisions,
        asked: &mut Asked,
    ) -> Result<usize, ProjectBoardError> {
        if let Some((kept, answer)) = asked.as_ref() {
            if *kept == evidence {
                return answer.clone().map(|judged| judged.unpushed);
            }
        }
        let answer = self.judged(&evidence);
        *asked = Some((evidence, answer.clone()));
        answer.map(|judged| judged.unpushed)
    }

    /// `evidence` as the remote judges it.
    fn judged(&self, evidence: &EvidenceRevisions) -> Result<Judged, ProjectBoardError> {
        let id_len = match evidence.ids.as_slice() {
            [] => {
                return Ok(Judged {
                    unpushed: 0,
                    unchecked: evidence.entries.len(),
                });
            }
            [_, ..] => self.revisions.commit_id_hex_len()?,
        };
        let (checked, _): (Vec<String>, Vec<String>) = evidence
            .ids
            .iter()
            .cloned()
            .partition(|id| checks_commit(id, id_len));
        let unchecked = evidence.unchecked(id_len);
        let unpushed = match checked.as_slice() {
            [] => 0,
            [_, ..] => {
                let missing = self.revisions.not_on_remote(&checked)?;
                assert!(
                    missing.iter().all(|revision| checked.contains(revision)),
                    "the remote answers only for the revisions asked"
                );
                missing.len()
            }
        };
        Ok(Judged {
            unpushed,
            unchecked,
        })
    }
}

/// How the remote judged a success's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Judged {
    unpushed: usize,
    unchecked: usize,
}

/// The evidence the remote was asked about, and how it judged it or why
/// it could not.
type Asked = Option<(EvidenceRevisions, Result<Judged, ProjectBoardError>)>;

#[cfg(test)]
#[path = "end_run_tests.rs"]
mod tests;
