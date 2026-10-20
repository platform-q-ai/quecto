//! The runs a session leaves open as it ends (#2655): a one-shot parent
//! whose reply ended while its board run was open left the run `running`
//! with no swarm behind it, its task claimed until the lease ran out.
use std::sync::Arc;

use crate::application::project_board::dto::{ProjectBoardError, RunEnd};
use crate::application::project_board::ports::{ProjectBoardStore, StartedRuns};

pub struct ListLeftRuns {
    store: Arc<dyn ProjectBoardStore>,
    started: Arc<dyn StartedRuns>,
}

impl ListLeftRuns {
    pub fn new(store: Arc<dyn ProjectBoardStore>, started: Arc<dyn StartedRuns>) -> Self {
        Self { store, started }
    }

    /// # Errors
    /// The store's.
    pub fn execute(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        let _ = (&self.store, &self.started);
        Ok(vec![])
    }
}

#[cfg(test)]
#[path = "list_left_runs_tests.rs"]
mod tests;
