use super::{GetMessageFields, RecoverMessageController};
use crate::application::sessions::active_session::ActiveSessionState;
use crate::application::sessions::dto::{RecoveredContent, RecoveryError};
use crate::application::sessions::use_cases::RecoverMessage;
use crate::domain::message::{Message, ToolCall};
use crate::domain::session_identity::SessionIdentity;
use std::sync::Arc;

fn controller(published: &[Message]) -> RecoverMessageController {
    let mut state = ActiveSessionState::new(SessionIdentity::ephemeral());
    state.publish(published);
    RecoverMessageController::new(Arc::new(RecoverMessage::new(Arc::new(
        tokio::sync::RwLock::new(state),
    ))))
}

#[tokio::test]
async fn wire_fields_map_to_message_and_tool_call_selectors() {
    let message = Message::assistant(
        "answer body",
        vec![ToolCall {
            id: "call-1".into(),
            name: "bash".into(),
            arguments: "{\"command\":\"ls\"}".into(),
        }],
    );
    let id = message.id().to_string();
    let controller = controller(std::slice::from_ref(&message));

    let whole = controller
        .recover(
            GetMessageFields {
                message_id: &id,
                tool_call_id: None,
                offset: None,
                thinking_offset: None,
                limit: None,
            },
            &[],
        )
        .await
        .unwrap();
    let RecoveredContent::Message {
        ranged,
        thinking_offset,
        range,
        ..
    } = whole
    else {
        panic!("message selector");
    };
    assert!(!ranged);
    assert_eq!(thinking_offset, 0);
    assert_eq!(range.end, "answer body".len());

    let ranged = controller
        .recover(
            GetMessageFields {
                message_id: &id,
                tool_call_id: None,
                offset: Some(3),
                thinking_offset: Some(9),
                limit: Some(2),
            },
            &[],
        )
        .await
        .unwrap();
    let RecoveredContent::Message {
        ranged,
        thinking_offset,
        range,
        ..
    } = ranged
    else {
        panic!("message selector");
    };
    assert!(ranged);
    assert_eq!(thinking_offset, 9);
    assert_eq!((range.start, range.end), (3, 5));

    let arguments = controller
        .recover(
            GetMessageFields {
                message_id: &id,
                tool_call_id: Some("call-1"),
                offset: Some(1),
                limit: Some(4),
                thinking_offset: None,
            },
            &[],
        )
        .await
        .unwrap();
    let RecoveredContent::ToolCallArguments {
        tool_call, range, ..
    } = arguments
    else {
        panic!("tool-call selector");
    };
    assert_eq!(tool_call.id, "call-1");
    assert_eq!((range.start, range.end), (1, 5));

    let missing = controller
        .recover(
            GetMessageFields {
                message_id: &id,
                tool_call_id: Some("call-9"),
                offset: None,
                limit: None,
                thinking_offset: None,
            },
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(missing, RecoveryError::ToolCallNotFound { .. }));
    assert!(format!("{controller:?}").starts_with("RecoverMessageController"));
}

#[tokio::test]
async fn the_fallback_conversation_serves_refs_the_state_has_not_published() {
    let controller = controller(&[]);
    let live = Message::user("unpublished");
    let id = live.id().to_string();
    let fields = GetMessageFields {
        message_id: &id,
        tool_call_id: None,
        offset: None,
        thinking_offset: None,
        limit: None,
    };
    assert!(matches!(
        controller.recover(fields.clone(), &[]).await,
        Err(RecoveryError::MessageNotFound(_))
    ));
    let RecoveredContent::Message { message, .. } = controller
        .recover(fields, std::slice::from_ref(&live))
        .await
        .unwrap()
    else {
        panic!("message selector");
    };
    assert_eq!(message.content, "unpublished");
}
