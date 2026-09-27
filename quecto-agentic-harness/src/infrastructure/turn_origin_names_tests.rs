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
