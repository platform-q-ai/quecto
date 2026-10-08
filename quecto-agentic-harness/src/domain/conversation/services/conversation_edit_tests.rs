use super::*;

fn id_of(message: &Message) -> MessageId {
    MessageId::from(message.id().to_string())
}

#[test]
fn clear_keeps_a_leading_non_manifest_system_prompt() {
    let mut messages = vec![
        Message::system("Be helpful."),
        Message::user("hello"),
        Message::assistant("hi", vec![]),
    ];
    clear_conversation(&mut messages);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::System);
    assert_eq!(messages[0].content, "Be helpful.");
}

#[test]
fn clear_without_a_system_prompt_empties_the_conversation() {
    let mut messages = vec![Message::user("hello"), Message::assistant("hi", vec![])];
    clear_conversation(&mut messages);
    assert!(messages.is_empty());
}

#[test]
fn clear_drops_a_leading_retention_manifest() {
    let mut manifest = Message::system("[Session memory: 5 spilled entries]");
    manifest.is_manifest = true;
    let mut messages = vec![manifest, Message::user("hello")];
    clear_conversation(&mut messages);
    assert!(messages.is_empty(), "a manifest is history, not the prompt");
}

#[test]
fn select_prefers_the_stable_id_over_a_legacy_index() {
    let id = MessageId::from("11111111-1111-1111-1111-111111111111");
    assert_eq!(
        RewindTarget::select(Some(id.clone()), Some(3)),
        Some(RewindTarget::MessageId(id.clone()))
    );
    assert_eq!(
        RewindTarget::select(Some(id.clone()), None),
        Some(RewindTarget::MessageId(id))
    );
    assert_eq!(
        RewindTarget::select(None, Some(3)),
        Some(RewindTarget::LegacyIndex(3))
    );
    assert_eq!(RewindTarget::select(None, None), None);
}

#[test]
fn a_stable_id_resolves_to_its_absolute_index() {
    let messages: Vec<Message> = (0..5).map(|i| Message::user(format!("m{i}"))).collect();
    let target = RewindTarget::MessageId(id_of(&messages[3]));
    assert_eq!(resolve_rewind_target(&messages, &target, 2), Ok(3));
}

#[test]
fn an_unknown_stable_id_is_not_found_and_never_falls_back() {
    let messages: Vec<Message> = (0..3).map(|i| Message::user(format!("m{i}"))).collect();
    let target = RewindTarget::MessageId(MessageId::from(uuid::Uuid::new_v4().to_string()));
    assert_eq!(
        resolve_rewind_target(&messages, &target, 64),
        Err(RewindTargetError::NotFound)
    );
    let not_a_uuid = RewindTarget::MessageId(MessageId::from("not-a-uuid"));
    assert_eq!(
        resolve_rewind_target(&messages, &not_a_uuid, 64),
        Err(RewindTargetError::NotFound)
    );
}

#[test]
fn a_legacy_index_passes_through_within_one_page() {
    let messages: Vec<Message> = (0..4).map(|i| Message::user(format!("m{i}"))).collect();
    assert_eq!(
        resolve_rewind_target(&messages, &RewindTarget::LegacyIndex(2), 4),
        Ok(2)
    );
    // Out of range is still resolved; the boundary check refuses it later.
    assert_eq!(
        resolve_rewind_target(&messages, &RewindTarget::LegacyIndex(99), 4),
        Ok(99)
    );
}

#[test]
fn a_legacy_index_is_ambiguous_beyond_one_page() {
    let messages: Vec<Message> = (0..5).map(|i| Message::user(format!("m{i}"))).collect();
    assert_eq!(
        resolve_rewind_target(&messages, &RewindTarget::LegacyIndex(2), 4),
        Err(RewindTargetError::AmbiguousLegacyIndex)
    );
}

#[test]
fn truncation_removes_the_user_message_and_everything_after() {
    let mut messages = vec![
        Message::system("prompt"),
        Message::user("first"),
        Message::assistant("answer", vec![]),
        Message::user("second"),
        Message::assistant("later", vec![]),
    ];
    assert!(truncate_at_user_message(&mut messages, 3));
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].content, "answer");
}

#[test]
fn truncation_refuses_a_non_user_boundary_and_out_of_range() {
    let mut messages = vec![Message::user("first"), Message::assistant("answer", vec![])];
    assert!(!truncate_at_user_message(&mut messages, 1));
    assert!(!truncate_at_user_message(&mut messages, 2));
    assert_eq!(messages.len(), 2, "a refused rewind changes nothing");
}
