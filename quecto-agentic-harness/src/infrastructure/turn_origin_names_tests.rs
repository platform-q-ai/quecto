use super::{origin_from_name, origin_name};
use crate::domain::turn_origin::TurnOrigin;

#[test]
fn every_origin_round_trips_through_its_name() {
    for origin in [
        TurnOrigin::Unknown,
        TurnOrigin::Instruction,
        TurnOrigin::ProgressNudge,
        TurnOrigin::Unrecognised,
    ] {
        assert_eq!(origin_from_name(origin_name(origin)), origin);
    }
    assert_eq!(origin_name(TurnOrigin::Unknown), None);
    assert_eq!(origin_name(TurnOrigin::Instruction), Some("instruction"));
    assert_eq!(
        origin_name(TurnOrigin::ProgressNudge),
        Some("progressNudge")
    );
}

/// Review 3 L2: a name this build does not know is unrecognised, never an
/// unmarked (legacy) message.
#[test]
fn an_unknown_name_is_unrecognised_never_unmarked() {
    assert_eq!(
        origin_from_name(Some("someNewKind")),
        TurnOrigin::Unrecognised
    );
    assert_eq!(origin_from_name(None), TurnOrigin::Unknown);
}

/// #2403: a prompt and a stub round-trip through their names; only a name
/// this build knows marks a message.
#[test]
fn every_user_kind_round_trips_and_an_unknown_name_is_unmarked() {
    use super::{user_kind_from_name, user_kind_name};
    use crate::domain::conversation::UserKind;
    for kind in [UserKind::Unmarked, UserKind::Prompt, UserKind::ArchiveStub] {
        assert_eq!(user_kind_from_name(user_kind_name(kind)), kind);
    }
    assert_eq!(user_kind_name(UserKind::Unmarked), None);
    assert_eq!(user_kind_name(UserKind::Prompt), Some("prompt"));
    assert_eq!(user_kind_name(UserKind::ArchiveStub), Some("archiveStub"));
    assert_eq!(user_kind_from_name(Some("someNewKind")), UserKind::Unmarked);
}
