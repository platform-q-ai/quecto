use super::*;
use crate::application::use_cases::test_support::MockGateway;

fn input(wait: bool) -> SendPromptInput {
    SendPromptInput {
        message: "hi".into(),
        images: Vec::new(),
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
        [AgentCommand::Prompt { message, streaming_behavior, .. }]
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

// ── #2422: image attachments ──────────────────────────────────────────────────

/// A 2x2 PNG: signature, IHDR, IEND.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC";
/// A 2x2 JPEG: SOI, APP0 JFIF, SOF0, EOI.
const JPEG: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/wAALCAACAAIBAREA/9k=";

fn with_images(message: &str, images: Vec<quecto_image::ImagePayload>) -> SendPromptInput {
    SendPromptInput {
        message: message.into(),
        images,
        streaming_behavior: None,
        wait_for_completion: true,
    }
}

fn png() -> quecto_image::ImagePayload {
    quecto_image::ImagePayload::new("image/png", PNG)
}

fn sent_images(gw: &MockGateway) -> Vec<(String, String)> {
    match gw.commands().as_slice() {
        [AgentCommand::Prompt { images, .. }] => images
            .iter()
            .map(|i| (i.mime_type().to_string(), i.data().to_string()))
            .collect(),
        other => panic!("expected one prompt, got {other:?}"),
    }
}

#[tokio::test]
async fn forwards_admitted_images_in_order() {
    let gw = MockGateway::connected();
    let jpeg = quecto_image::ImagePayload::new("image/jpeg", JPEG);
    execute(&gw, with_images("look", vec![png(), jpeg]))
        .await
        .unwrap();
    assert_eq!(
        sent_images(&gw),
        [
            ("image/png".to_string(), PNG.to_string()),
            ("image/jpeg".to_string(), JPEG.to_string())
        ]
    );
}

#[tokio::test]
async fn an_images_only_prompt_is_sent() {
    let gw = MockGateway::connected();
    execute(&gw, with_images("", vec![png()])).await.unwrap();
    assert_eq!(sent_images(&gw).len(), 1);
}

#[tokio::test]
async fn empty_text_without_images_is_refused_as_before() {
    let gw = MockGateway::connected();
    let err = execute(&gw, with_images("", Vec::new())).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid request: message must not be empty"
    );
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn a_refused_image_refuses_the_prompt_with_the_exact_message() {
    for wait in [true, false] {
        let gw = MockGateway::connected();
        let mut input = with_images(
            "look",
            vec![png(), quecto_image::ImagePayload::new("image/gif", PNG)],
        );
        input.wait_for_completion = wait;
        let err = execute(&gw, input).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid request: images[1]: data does not start with the image/gif signature"
        );
        assert!(gw.commands().is_empty() && gw.enqueued().is_empty());
    }
}

#[tokio::test]
async fn a_refused_image_is_refused_before_the_connection_is_checked() {
    let gw = MockGateway::disconnected();
    let bad = quecto_image::ImagePayload::new("image/gif", PNG);
    let err = execute(&gw, with_images("look", vec![bad]))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)), "{err}");
}
