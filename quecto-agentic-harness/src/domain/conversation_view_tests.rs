use super::{is_injected_system_prompt, position_by_id, user_visible_messages};
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

#[test]
fn injected_prompt_matches_only_the_exact_non_manifest_system_message() {
    let prompt = Message::system("be brief");
    assert!(is_injected_system_prompt(&prompt, "be brief"));
    assert!(!is_injected_system_prompt(&prompt, ""));
    assert!(!is_injected_system_prompt(&prompt, "be verbose"));
    let mut manifest = Message::system("be brief");
    manifest.is_manifest = true;
    assert!(!is_injected_system_prompt(&manifest, "be brief"));
    assert!(!is_injected_system_prompt(
        &Message::user("be brief"),
        "be brief"
    ));
}

#[test]
fn user_visible_messages_drop_the_injected_prompt_and_keep_order() {
    let messages = vec![
        Message::system("be brief"),
        Message::user("hi"),
        Message::assistant("hello", vec![]),
    ];
    let visible = user_visible_messages(&messages, "be brief");
    assert_eq!(visible.len(), 2);
    assert_eq!(visible[0].content, "hi");
    assert_eq!(visible[1].content, "hello");
    assert_eq!(user_visible_messages(&messages, "").len(), 3);
}

#[test]
fn position_by_id_parses_once_and_rejects_non_uuids() {
    let messages = vec![Message::user("a"), Message::user("b")];
    let second = MessageId::from(messages[1].id().to_string());
    assert_eq!(position_by_id(&messages, &second), Some(1));
    assert_eq!(
        position_by_id(&messages, &MessageId::from("not-a-uuid")),
        None
    );
    let stranger = MessageId::from(uuid::Uuid::new_v4().to_string());
    assert_eq!(position_by_id(&messages, &stranger), None);
}
