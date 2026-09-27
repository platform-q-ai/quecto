//! The names a transcript page, a saved session and an export give a
//! message's turn origin (#2226): the adapters' mapping of the domain's
//! [`TurnOrigin`], so the domain carries no wire or storage vocabulary.
use crate::domain::turn_origin::TurnOrigin;

const INSTRUCTION: &str = "instruction";
const PROGRESS_NUDGE: &str = "progressNudge";
const UNRECOGNISED: &str = "unrecognised";

/// The name of `origin`; an unmarked message has none.
pub fn origin_name(origin: TurnOrigin) -> Option<&'static str> {
    match origin {
        TurnOrigin::Unknown => None,
        TurnOrigin::Instruction => Some(INSTRUCTION),
        TurnOrigin::ProgressNudge => Some(PROGRESS_NUDGE),
        TurnOrigin::Unrecognised => Some(UNRECOGNISED),
    }
}

/// The origin `name` stands for: none is an unmarked message, a name this
/// build does not know is [`TurnOrigin::Unrecognised`] (ranked last, and
/// kept unrecognised when saved again).
pub fn origin_from_name(name: Option<&str>) -> TurnOrigin {
    match name {
        None => TurnOrigin::Unknown,
        Some(INSTRUCTION) => TurnOrigin::Instruction,
        Some(PROGRESS_NUDGE) => TurnOrigin::ProgressNudge,
        Some(_) => TurnOrigin::Unrecognised,
    }
}

#[cfg(test)]
#[path = "turn_origin_names_tests.rs"]
mod tests;
