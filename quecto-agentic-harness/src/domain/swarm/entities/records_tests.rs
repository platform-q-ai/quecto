use super::*;

#[test]
fn criterion_kinds_round_trip_their_stored_text() {
    for (kind, text) in [
        (CriterionKind::Command, "command"),
        (CriterionKind::Review, "review"),
    ] {
        assert_eq!(kind.as_str(), text);
        assert_eq!(CriterionKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(CriterionKind::parse("Command"), None);
    assert_eq!(CriterionKind::parse(""), None);
}
