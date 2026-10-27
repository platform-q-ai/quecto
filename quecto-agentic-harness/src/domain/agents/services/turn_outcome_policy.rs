//! The deterministic turn-outcome policy of a trusted top-level parent
//! (#2708): what a finished turn settles on, which launches are trusted
//! roots, and which external identities are admissible.
use crate::domain::agents::value_objects::turn_declaration::{TurnDeclaration, TurnIntent};
use crate::domain::agents::value_objects::turn_outcome::{
    LIVE_REASON, MAX_EPISODE_ID_BYTES, STALLED_REASON, TurnOutcome, UNKNOWN_REASON, WorkLiveness,
};

/// What a finished parent turn settles on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnVerdict {
    /// Wake the parent once with the correction note.
    Correct,
    /// Publish this outcome with this exact reason.
    Settle(TurnOutcome, String),
}

/// Settle a turn that completed: its declaration (if any, for this turn),
/// whether its work runs, and whether its one correction is spent.
pub fn settle_turn(
    declaration: Option<&TurnDeclaration>,
    liveness: WorkLiveness,
    correction_spent: bool,
) -> TurnVerdict {
    let intent = declaration.map(|declared| (declared.intent, declared.reason.as_str()));
    match (intent, liveness, correction_spent) {
        (Some((TurnIntent::Blocked | TurnIntent::Asking, reason)), _, _) => {
            TurnVerdict::Settle(TurnOutcome::NeedsUser, reason.to_owned())
        }
        (Some((TurnIntent::Done, reason)), _, _) => {
            TurnVerdict::Settle(TurnOutcome::Done, reason.to_owned())
        }
        (Some((TurnIntent::Waiting, reason)), WorkLiveness::Live, _) => {
            TurnVerdict::Settle(TurnOutcome::Waiting, reason.to_owned())
        }
        (None, WorkLiveness::Live, _) => {
            TurnVerdict::Settle(TurnOutcome::Waiting, LIVE_REASON.to_owned())
        }
        (Some((TurnIntent::Waiting, _)) | None, WorkLiveness::Unknown, _) => {
            TurnVerdict::Settle(TurnOutcome::NeedsUser, UNKNOWN_REASON.to_owned())
        }
        (Some((TurnIntent::Waiting, _)) | None, WorkLiveness::Empty, false) => TurnVerdict::Correct,
        (Some((TurnIntent::Waiting, _)) | None, WorkLiveness::Empty, true) => {
            TurnVerdict::Settle(TurnOutcome::Stalled, STALLED_REASON.to_owned())
        }
    }
}

/// The facts of a launch that decide whether it is a trusted root parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchFacts {
    pub spawned: bool,
    pub coordinator: bool,
    pub has_parent_id: bool,
    pub has_parent_control: bool,
    pub swarm_member: bool,
}

/// Whether a launch is a trusted top-level parent.
/// An affirmative allowlist of exactly one launch shape: not spawned, not a
/// coordinator, no parent and no swarm. Every other shape is delegated.
pub fn is_trusted_root(facts: LaunchFacts) -> bool {
    matches!(
        facts,
        LaunchFacts {
            spawned: false,
            coordinator: false,
            has_parent_id: false,
            has_parent_control: false,
            swarm_member: false,
        }
    )
}

/// Whether the launcher switched the guard on: exactly `1`, nothing else.
pub fn turn_outcomes_switched_on(value: Option<&str>) -> bool {
    matches!(value, Some("1"))
}

/// Whether an external instruction's identity may open an episode.
/// Bytes, not scalars, and never trimmed: the identity is echoed exactly.
pub fn admissible_episode_id(episode_id: &str) -> bool {
    (1..=MAX_EPISODE_ID_BYTES).contains(&episode_id.len())
}

#[cfg(test)]
#[path = "turn_outcome_policy_tests.rs"]
mod tests;
