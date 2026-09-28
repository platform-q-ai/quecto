//! The swarm board's differential harness (#2270, epic #2265): the same
//! operation sequence against the Python board and the Rust board, each on
//! its own file, with the same clock and the same id draws. Each side
//! parses the step's argument text itself (`json.loads`, `py_json::decode`).
//! After every step the results must be equal, refusals equal text, and the
//! two files equal logical dumps.
//!
//! Results are compared after a serde round trip, not as Python's own
//! text: the driver writes Python's result with `json.dumps`, the harness
//! parses that into a `serde_json::Value`, and the two sides' `Value`s are
//! compared as serde writes them (an integer never equals a float, and an
//! object's keys compare in order). A difference the round trip erases is
//! not seen here: float digits and exponent spelling (`1e+16` against
//! serde's `1e16`), an integer beyond u64 (read as a float), and
//! `NaN`/`Infinity`, which serde does not read (the harness panics on such
//! an answer). The results are rendered in Python's text only on the wire,
//! so S13's wire rendering must write them with `py_json` (`dumps`), never
//! with serde, and compare that text against Python's.

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
