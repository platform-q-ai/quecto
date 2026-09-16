use super::RewindFields;
use crate::application::sessions::dto::RewindConversationError;
use crate::domain::conversation_edit::RewindTarget;
use crate::domain::ids::MessageId;

fn fields(message_id: Option<&str>, message_index: Option<usize>) -> RewindFields {
    RewindFields {
        message_id: message_id.map(str::to_string),
        message_index,
    }
}

#[test]
fn the_stable_id_is_preferred_over_a_legacy_index() {
    let request = fields(Some("abc"), Some(0)).into_request(64).unwrap();
    assert_eq!(
        request.target,
        RewindTarget::MessageId(MessageId::from("abc"))
    );
    assert_eq!(request.legacy_index_window, 64);
}

#[test]
fn a_legacy_index_alone_maps_onto_the_restricted_variant() {
    let request = fields(None, Some(3)).into_request(64).unwrap();
    assert_eq!(request.target, RewindTarget::LegacyIndex(3));
    assert_eq!(request.legacy_index_window, 64);
}

#[test]
fn a_request_naming_neither_is_the_missing_target_refusal() {
    let err = fields(None, None)
        .into_request(64)
        .expect_err("nothing to rewind to");
    assert!(matches!(err, RewindConversationError::MissingTarget));
    assert_eq!(err.to_string(), "rewind requires messageId or messageIndex");
    assert_eq!(fields(None, None).clone(), fields(None, None));
}
