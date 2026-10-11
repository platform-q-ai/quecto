//! The board operations as their front ends send and answer them (#2477),
//! shared by the `quecto board` CLI and the agent `board` tool (S2b): a
//! request is one JSON object read by [`request`] (the command line builds
//! the same object, [`command_line`]), run by the application's
//! [`BoardOperations`], and answered in one stable JSON shape by
//! [`answer`]. One path, so the front ends cannot drift apart.
pub mod answer;
pub mod command_line;
pub mod json;
pub mod request;

use serde_json::Value;

use crate::application::project_board::operations::BoardOperations;

/// The answer to the request `value`.
pub fn run(operations: &BoardOperations, value: &Value) -> Value {
    let op = request::op_of(value);
    let request = match request::parse(value) {
        Ok(request) => request,
        Err(error) => return answer::request_refused(op.as_deref(), &error),
    };
    let op = op.expect("a parsed request names an op");
    match operations.execute(&request) {
        Ok(outcome) => answer::outcome(&op, &outcome, operations.clock_note(&request)),
        Err(error) => answer::board_refused(&op, &error),
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
