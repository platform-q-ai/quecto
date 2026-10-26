//! The runs a session leaves open as it ends (#2655): a one-shot parent
//! whose reply ended while its board run was open left the run `running`
//! with no swarm behind it, its task claimed until the lease ran out, and
//! nothing said so. A session that ends in an orderly way ends each run
//! its own starts launched ([`StartedRuns`]) that the board still shows
//! open, as `stopped` ([`LEFT_RUN_END`]): its swarm stopped with the
//! session that ran it, and the session can account for it, so it is not
//! `lost`. A run another session launched is never this session's to end,
//! nor is one the board shows ended or does not have.
//!
//! This only lists the ends; the front end makes each through the same
//! path a caller's `end_run` takes (its rules, its liveness check and its
//! record), once the session's children have settled.
use std::sync::Arc;

use crate::application::project_board::dto::{ProjectBoardError, RunEnd, RunEndOutcome};
use crate::application::project_board::ports::{ProjectBoardStore, StartedRuns};

/// How a run its session left open is ended.
pub const LEFT_RUN_END: RunEndOutcome = RunEndOutcome::Stopped;

pub struct ListLeftRuns {
    store: Arc<dyn ProjectBoardStore>,
    started: Arc<dyn StartedRuns>,
}

impl ListLeftRuns {
    pub fn new(store: Arc<dyn ProjectBoardStore>, started: Arc<dyn StartedRuns>) -> Self {
        Self { store, started }
    }

    /// The ends the session owes: each run its starts launched that the
    /// board shows open, in the order they were launched, once each. A
    /// session that launched none reads nothing.
    ///
    /// # Errors
    /// The store's.
    pub fn execute(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        let started = self.started.started();
        if started.is_empty() {
            return Ok(vec![]);
        }
        let board = self.store.read()?;
        let mut ends: Vec<RunEnd> = Vec::new();
        let mut first = std::collections::BTreeSet::new();
        for (task, number) in started {
            let open = board.runs.iter().any(|run| {
                let fields = run.fields();
                fields.task == task && fields.number == number && run.is_open()
            });
            // Once each: a run first named here.
            if open && first.insert((task.clone(), number)) {
                ends.push(RunEnd {
                    task,
                    run: number,
                    outcome: LEFT_RUN_END,
                    expected: None,
                    stop: None,
                });
            }
        }
        assert!(
            ends.iter().all(|end| end.outcome == LEFT_RUN_END),
            "a left run is ended one way"
        );
        Ok(ends)
    }
}

#[cfg(test)]
#[path = "list_left_runs_tests.rs"]
mod tests;
