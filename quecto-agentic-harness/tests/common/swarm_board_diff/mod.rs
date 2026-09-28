//! The swarm board's differential harness (#2270, epic #2265): the same
//! operation sequence against the Python board and the Rust board, each on
//! its own file, with the same clock and the same id draws. After every
//! step the results must be equal JSON values (an integer never equals a
//! float), refusals equal text, and the two files equal logical dumps.

pub mod dump;
pub mod python;
pub mod rust;
pub mod scenario;

/// What one board call answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The method returned this JSON value.
    Ok(serde_json::Value),
    /// The board refused with this text (`SwarmError`, `BoardError`).
    Refused(String),
    /// Python raised something other than a `SwarmError`: a harness or
    /// calling fault, never an answer to compare.
    Raised(String),
}
