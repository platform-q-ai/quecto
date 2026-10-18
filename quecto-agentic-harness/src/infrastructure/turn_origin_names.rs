//! The names a transcript page, a saved session and an export give a
//! message's turn origin (#2226): the adapters' mapping of the domain's
//! [`TurnOrigin`], so the domain carries no wire or storage vocabulary;
//! likewise a user message's [`UserKind`] (#2403).
use crate::domain::conversation::services::turn_origin::TurnOrigin;
use crate::domain::conversation::value_objects::user_kind::UserKind;

const INSTRUCTION: &str = "instruction";
const PROGRESS_NUDGE: &str = "progressNudge";
const UNRECOGNISED: &str = "unrecognised";
const PROMPT: &str = "prompt";
const ARCHIVE_STUB: &str = "archiveStub";

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

/// The name a saved message gives its [`UserKind`] (#2403); an unmarked
/// message has none.
pub fn user_kind_name(kind: UserKind) -> Option<&'static str> {
    match kind {
        UserKind::Unmarked => None,
        UserKind::Prompt => Some(PROMPT),
        UserKind::ArchiveStub => Some(ARCHIVE_STUB),
    }
}

/// The kind `name` stands for: only a name this build knows marks a
/// message; none, or any other, is unmarked.
pub fn user_kind_from_name(name: Option<&str>) -> UserKind {
    match name {
        Some(PROMPT) => UserKind::Prompt,
        Some(ARCHIVE_STUB) => UserKind::ArchiveStub,
        Some(_) | None => UserKind::Unmarked,
    }
}

#[cfg(test)]
#[path = "turn_origin_names_tests.rs"]
mod tests;
