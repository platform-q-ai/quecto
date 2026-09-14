use super::{ActiveSessionState, RecallIdentity, RecoveryResolution};
use crate::application::sessions::use_cases::recover_message::tests::{
    MemSpillStore, NoopSpillStore, collapsed_message, spill_entry,
};
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;
use std::sync::Arc;

#[test]
fn a_new_state_carries_its_identity_and_an_empty_conversation() {
    let state = ActiveSessionState::new(SessionIdentity::from_persisted_key("cli:one"));
    assert_eq!(state.identity().runtime_key(), "cli:one");
    assert!(state.conversation().live_messages().is_empty());
    assert!(matches!(
        state.resolve_for_recovery("missing"),
        RecoveryResolution::NotFound
    ));
    assert!(format!("{state:?}").contains("cli:one"));
}

#[test]
fn resolution_is_found_for_plain_messages_and_stubs_without_a_store() {
    let mut state = ActiveSessionState::new(SessionIdentity::ephemeral());
    let plain = Message::assistant("plain", vec![]);
    let stub = collapsed_message("spill-1");
    let mut stub_without_spill = collapsed_message("spill-2");
    stub_without_spill.spill_id = None;
    state.publish(&[plain.clone(), stub.clone(), stub_without_spill.clone()]);
    assert!(matches!(
        state.resolve_for_recovery(&plain.id().to_string()),
        RecoveryResolution::Found(m) if m.content == "plain"
    ));
    // A collapsed stub with no retention store is served as it is.
    assert!(matches!(
        state.resolve_for_recovery(&stub.id().to_string()),
        RecoveryResolution::Found(m) if m.is_collapsed
    ));
    state.set_spill_store(Some(Arc::new(NoopSpillStore)));
    assert!(matches!(
        state.resolve_for_recovery(&stub_without_spill.id().to_string()),
        RecoveryResolution::Found(_)
    ));
    assert!(matches!(
        state.resolve_for_recovery(&stub.id().to_string()),
        RecoveryResolution::Recall { .. }
    ));
}

#[tokio::test]
async fn a_deferred_recall_fills_the_stub_and_carries_its_identity() {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key("cli:s"));
    let stub = collapsed_message("spill-1");
    let id = stub.id().to_string();
    let store = Arc::new(MemSpillStore::with_session_entry(
        "cli:s",
        spill_entry("spill-1", "the full body"),
    ));
    state.set_spill_store(Some(store.clone()));
    state.publish(std::slice::from_ref(&stub));
    let RecoveryResolution::Recall {
        generation,
        ref identity,
        ..
    } = state.resolve_for_recovery(&id)
    else {
        panic!("collapsed stub with a store defers");
    };
    assert_eq!(identity.runtime_key(), "cli:s");
    let expected = RecallIdentity {
        message_id: id.clone(),
        identity: identity.clone(),
        spill_id: "spill-1".into(),
        generation,
    };
    let resolved = state.resolve_for_recovery(&id).into_message().await;
    let message = resolved.message.expect("stub is returned");
    assert_eq!(message.content, "the full body");
    assert!(!message.is_collapsed);
    assert_eq!(message.spill_id, None);
    assert_eq!(resolved.recalled, Some(expected.clone()));
    assert!(state.recall_is_current(&expected));
    assert_eq!(
        store.recalled(),
        vec![("cli:s".to_string(), "spill-1".to_string())]
    );

    // A miss or an error keeps the stub, still marked as a deferred read.
    let missing = Arc::new(MemSpillStore::default());
    state.set_spill_store(Some(missing));
    let resolved = state.resolve_for_recovery(&id).into_message().await;
    assert!(resolved.message.as_ref().is_some_and(|m| m.is_collapsed));
    assert!(resolved.recalled.is_some());
    assert!(
        !state.recall_is_current(&expected),
        "a replaced store advances the generation"
    );
}

#[test]
fn switching_identity_after_clear_keeps_the_generation_of_the_clear() {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key("cli:old"));
    state.publish(&[Message::user("old")]);
    let before = state.conversation().generation();
    let advance = state.clear();
    assert!(advance.changed);
    assert_eq!(state.conversation().generation(), before + 1);
    state.switch_identity(
        SessionIdentity::from_persisted_key("cli:new"),
        Some(Arc::new(NoopSpillStore)),
    );
    assert_eq!(state.identity().runtime_key(), "cli:new");
    assert!(state.conversation().spill_store().is_some());
    assert_eq!(state.conversation().generation(), before + 1);
    let stale = RecallIdentity {
        message_id: "x".into(),
        identity: SessionIdentity::from_persisted_key("cli:old"),
        spill_id: "s".into(),
        generation: before,
    };
    assert!(!state.recall_is_current(&stale));
}
