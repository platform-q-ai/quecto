use super::*;

#[test]
fn a_known_position_follows_the_failure() {
    assert_eq!(
        placed(DocumentTextFailure::Malformed.words(), 3, 14),
        "malformed JSON at line 3 column 14"
    );
}

#[test]
fn an_unknown_position_leaves_the_failure_alone() {
    assert_eq!(
        placed(DocumentTextFailure::EndsEarly.words(), 0, 0),
        "the file ends early"
    );
}
