//! The swarm board's differential harness (#2270, epic #2265): an
//! operation sequence run on the Rust board, with an injected clock and id
//! draws, compared step by step with what the Python board answered for the
//! same sequence: results equal, refusals equal text, and the board file
//! the same logical dump. Until #2283 the Python board ran beside it, on a
//! file of its own; #2283 froze its answers into golden fixtures
//! (`golden.rs`) before deleting it, and the harness replays the Rust
//! board against them.
//!
//! Results are compared as values, not as Python's own text: the golden
//! holds the result as Python's `json.dumps` wrote it, which the harness
//! reads as Python's `json.loads` would (`py_json::decode`, whose float
//! parse is correctly rounded, #2277 review M1) into a `serde_json::Value`,
//! and the two `Value`s are compared as serde writes them (an integer never
//! equals a float, an object's keys compare in order, and serde writes
//! each float's shortest round-tripping digits, so two floats compare
//! equal only when they are the same double). The Rust side's `Value` is
//! the dispatcher's own answer, never parsed from text. What is not seen
//! here is spelling alone: exponent spelling (`1e+16` against serde's
//! `1e16`); `NaN`, `Infinity`, an integer beyond u64 or a lone surrogate,
//! which no `Value` holds, make the harness panic. The wire comparison
//! (`run_golden_wire`) compares the Rust board's wire text with Python's,
//! byte for byte.

pub mod dump;
pub mod golden;
pub mod rust;
pub mod scenario;

/// What one board call answered.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// The method returned this JSON value.
    Ok(serde_json::Value),
    /// The board refused with this text (`SwarmError`, `BoardError`).
    Refused(String),
    /// Python raised something other than a `SwarmError` (a golden's
    /// `raised`): never an answer the Rust board may match.
    Raised(String),
}

/// Results compare as serde's text of each `Value`, not by `Value`
/// equality, which ignores an object's key order (#2270 review L4):
/// Python's dicts keep theirs in the tool's output. See the module docs
/// for what the serde round trip cannot see.
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
