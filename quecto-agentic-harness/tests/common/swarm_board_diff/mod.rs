//! The swarm board's differential harness (#2270, epic #2265): the same
//! operation sequence against the Python board and the Rust board, each on
//! its own file, with the same clock and the same id draws. After every
//! step the results must be the same JSON text (an integer never equals a
//! float, and an object's keys compare in order), refusals equal text, and
//! the two files equal logical dumps.

pub mod dump;
pub mod python;
pub mod rust;
pub mod scenario;

/// What one board call answered.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// The method returned this JSON value.
    Ok(serde_json::Value),
    /// The board refused with this text (`SwarmError`, `BoardError`).
    Refused(String),
    /// Python raised something other than a `SwarmError`: a harness or
    /// calling fault, never an answer to compare.
    Raised(String),
}

/// Results compare as their JSON text, not as `Value`s: `Value` equality
/// ignores an object's key order (#2270 review L4), and Python's dicts
/// keep theirs in the tool's output.
impl PartialEq for Outcome {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ok(left), Self::Ok(right)) => json_text(left) == json_text(right),
            (Self::Refused(left), Self::Refused(right))
            | (Self::Raised(left), Self::Raised(right)) => left == right,
            _ => false,
        }
    }
}

fn json_text(value: &serde_json::Value) -> String {
    serde_json::to_string(value).expect("a result serializes")
}
