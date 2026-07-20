use super::*;
use crate::application::use_cases::test_support::MockGateway;

fn input(wait: bool) -> SendPromptInput {
    SendPromptInput {
        message: "hi".into(),
        streaming_behavior: Some("steer".into()),
        wait_for_completion: wait,
    }
}

#[tokio::test]
async fn waits_for_completion_via_send() {
    let gw = MockGateway::connected();
    execute(&gw, input(true)).await.unwrap();
    assert_eq!(gw.commands().len(), 1);
    assert!(gw.enqueued().is_empty());
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::Prompt { message, streaming_behavior }]
            if message == "hi" && streaming_behavior.as_deref() == Some("steer")
    ));
}

#[tokio::test]
async fn fire_and_forget_via_enqueue() {
    let gw = MockGateway::connected();
    execute(&gw, input(false)).await.unwrap();
    assert!(gw.commands().is_empty());
    assert_eq!(gw.enqueued().len(), 1);
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(&gw, input(true)).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}
