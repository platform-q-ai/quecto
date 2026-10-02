use super::*;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn rejects_empty_message() {
    let gw = MockGateway::connected();
    let err = execute(&gw, String::new(), Vec::new()).await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(&gw, "hi".into(), Vec::new()).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}

#[tokio::test]
async fn forwards_steer_command() {
    let gw = MockGateway::connected();
    execute(&gw, "focus".into(), Vec::new()).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::Steer { message, .. }] if message == "focus"
    ));
}

// ── #2422: image attachments ──────────────────────────────────────────────────

const PNG: &str = "iVBORw0KGgoAAAANSUhEUg==";

fn png() -> quecto_image::ImagePayload {
    quecto_image::ImagePayload::new("image/png", PNG)
}

#[tokio::test]
async fn forwards_admitted_images() {
    let gw = MockGateway::connected();
    execute(&gw, "look".into(), vec![png()]).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::Steer { images, .. }] if images.len() == 1 && images[0].data() == PNG
    ));
}

#[tokio::test]
async fn takes_images_without_text() {
    let gw = MockGateway::connected();
    execute(&gw, String::new(), vec![png()]).await.unwrap();
    assert_eq!(gw.commands().len(), 1);
}

#[tokio::test]
async fn a_refused_image_refuses_the_command_with_the_exact_message() {
    let gw = MockGateway::connected();
    let bad = quecto_image::ImagePayload::new("image/webp", PNG);
    let err = execute(&gw, "look".into(), vec![bad]).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid request: images[0]: data does not start with the image/webp signature"
    );
    assert!(gw.commands().is_empty());
}
