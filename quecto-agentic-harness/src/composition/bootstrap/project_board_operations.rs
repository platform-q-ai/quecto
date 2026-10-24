//! The board's operations on this machine (epic #2705 B6): the
//! [`BoardOperations`] port the front ends hand their requests to,
//! answered by the [`RunBoardOperation`] use case over the board
//! composition built, exactly as the front ends ran it before the port.
use crate::application::project_board::dto::{
    BoardAnswered, BoardExecution, BoardRequest, ListSummaries, OpNoting, ProjectBoardError, RunEnd,
};
use crate::application::project_board::ports::BoardOperations;
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::RunBoardOperation;

pub struct LocalBoardOperations {
    operations: RunBoardOperation,
}

impl LocalBoardOperations {
    /// The port over `operations`, as composition built them.
    pub fn new(operations: RunBoardOperation) -> Self {
        Self { operations }
    }
}

impl BoardOperations for LocalBoardOperations {
    fn execute(
        &self,
        request: &BoardRequest,
        summaries: ListSummaries,
        noting: OpNoting,
    ) -> BoardExecution {
        let mut facts = match noting {
            OpNoting::Noted => Some(OpFacts::default()),
            OpNoting::Unnoted => None,
        };
        let result = self
            .operations
            .execute_noted_with(request, facts.as_mut(), summaries)
            // Read after the request ran, and only of one answered, as the
            // front ends always read it.
            .map(|outcome| BoardAnswered {
                outcome,
                clock_note: self.operations.clock_note(request),
            });
        BoardExecution { result, facts }
    }

    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        self.operations.left_runs()
    }
}

#[cfg(test)]
#[path = "project_board_operations_fixtures_tests.rs"]
pub mod fixtures;

#[cfg(test)]
#[path = "project_board_operations_tests.rs"]
mod tests;
