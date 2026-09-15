use super::super::sessions::{SessionLoopInputs, build_session_handles};
use crate::domain::message::Message;

#[tokio::test]
async fn the_handles_share_one_state_between_the_handle_and_the_read_use_cases() {
    let tmp = tempfile::tempdir().unwrap();
    let handles = build_session_handles(SessionLoopInputs {
        base_dir: tmp.path().to_path_buf(),
        store: None,
        session_key: "cli:graph".into(),
        spill_store: None,
    });
    assert_eq!(
        handles.active_session.read().await.identity().runtime_key(),
        "cli:graph"
    );
    let message = Message::user("published once");
    handles
        .active_session
        .write()
        .await
        .publish(std::slice::from_ref(&message));
    let page = handles.read_history.newest_live_page(8).await;
    assert_eq!(page.messages.len(), 1);
    let recovered = handles
        .recover_message
        .recover(
            crate::interface::uds::sessions::recover_message_controller::GetMessageFields {
                message_id: &message.id().to_string(),
                tool_call_id: None,
                offset: None,
                thinking_offset: None,
                limit: None,
            },
            &[],
        )
        .await;
    assert!(
        recovered.is_ok(),
        "the recovery use case reads the same state"
    );
    let reads = handles.read_handles();
    assert!(std::sync::Arc::ptr_eq(
        &reads.active_session,
        &handles.active_session
    ));
    assert!(format!("{reads:?}").starts_with("SessionReadHandles"));
}
