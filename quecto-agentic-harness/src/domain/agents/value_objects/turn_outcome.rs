//! How a trusted top-level parent turn ends (#2708): the outcome the
//! harness settles on, the identity it is published under and the exact
//! words the guard uses. Transport-neutral: the UDS wire shape is this
//! record's serialization, the CLI reads it as an exit status.
use serde::{Deserialize, Serialize};

/// The version of the turn-outcome contract this harness advertises.
pub const TURN_OUTCOME_VERSION: u8 = 1;

/// The one system note that wakes a parent that ended with nothing running.
pub const CORRECTION_NOTE: &str = "Nothing is running. If your reply says you'll start something you haven't launched, you're not done: launch it now, or call declare_turn_outcome with blocked or asking. Otherwise call declare_turn_outcome: asking if you asked the user something, blocked if you can't proceed, done if you've finished.";
/// The line a guarded root's system prompt carries (#2708).
pub const DECLARE_GUIDANCE: &str = "End every turn by calling declare_turn_outcome: done when you've answered or finished, asking when you need the user's answer, blocked when you can't proceed, waiting only while work you launched is running.";
/// The reason a parent stalls on: still nothing running after the correction.
pub const STALLED_REASON: &str = "Nothing is running after one correction.";
/// The reason a parent waits on when it declared nothing but work runs.
pub const LIVE_REASON: &str = "Work is already running.";
/// The reason a parent needs its human when its work cannot be read.
pub const UNKNOWN_REASON: &str =
    "Unable to determine whether work is running. Inspect the harness state before continuing.";
/// The refusal of an external instruction whose identity is out of bounds.
pub const EPISODE_ID_REFUSAL: &str =
    "Turn identity metadata must be nonempty and at most 512 bytes.";
/// The most bytes an external episode identity may carry.
pub const MAX_EPISODE_ID_BYTES: usize = 512;

/// The settled outcome of a parent turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    /// Work the parent owns is still running.
    Waiting,
    /// The parent is blocked or asked its human a question.
    NeedsUser,
    /// The parent ended with nothing running after its one correction.
    Stalled,
    /// The parent declared the task complete.
    Done,
}

impl TurnOutcome {
    /// Whether this outcome ends the episode: automatic turns stop until a
    /// fresh external instruction.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::NeedsUser | Self::Stalled | Self::Done)
    }
}

/// Whether anything the parent owns is still running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkLiveness {
    Live,
    Empty,
    Unknown,
}

impl WorkLiveness {
    /// A source read without doubt: `Live` when it runs, else `Empty`.
    pub fn of(running: bool) -> Self {
        match running {
            true => Self::Live,
            false => Self::Empty,
        }
    }

    /// Both sources together: live work anywhere wins, an unreadable source
    /// is never read as empty.
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Live, _) | (_, Self::Live) => Self::Live,
            (Self::Empty, Self::Empty) => Self::Empty,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
        }
    }
}

/// One published outcome: the turn it settles, a sequence that advances
/// once per outcome, and the exact reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnOutcomeRecord {
    pub session_key: String,
    pub outcome_epoch: String,
    pub episode_id: String,
    pub turn_id: String,
    pub sequence: u64,
    pub outcome: TurnOutcome,
    pub reason: String,
}

/// The admission of a guarded turn, published before it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnAdmissionRecord {
    pub session_key: String,
    pub outcome_epoch: String,
    pub episode_id: String,
    pub turn_id: String,
}

/// A published outcome as an event of its own: the latest one a state
/// snapshot replays is the same event the turn emitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnOutcomeEvent {
    TurnOutcome(TurnOutcomeRecord),
}

#[cfg(test)]
#[path = "turn_outcome_tests.rs"]
mod tests;
