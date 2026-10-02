//! #2422: a prompt's images on the wire, and the frame cap they can reach.
use super::*;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUg==";

fn prompt(message: String, images: Vec<quecto_image::ImageAttachment>) -> AgentCommand {
    AgentCommand::Prompt {
        message,
        images,
        streaming_behavior: None,
    }
}

#[test]
fn a_prompt_carries_its_images_in_their_wire_shape() {
    let png = quecto_image::ImageAttachment::new(quecto_image::ImagePayload::new("image/png", PNG))
        .unwrap();
    let json = command_to_json(prompt("look".into(), vec![png]), "p1");
    assert_eq!(
        json["images"],
        serde_json::json!([{"mimeType": "image/png", "data": PNG}])
    );
    let text_only = command_to_json(prompt("look".into(), Vec::new()), "p2");
    assert!(text_only.get("images").is_none(), "{text_only}");
}

/// A command past the agent's frame cap used to be dropped by the writer,
/// leaving `send` to time out after 120 s; it is refused up front instead.
#[tokio::test]
async fn a_command_past_the_frame_cap_is_refused_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = spawn_echo_agent(&dir, "prompt").await;
    let gw = UdsGateway::connect(&path).await.unwrap();
    let huge = "x".repeat(MAX_LINE_BYTES);
    for waits in [true, false] {
        let call = async {
            if waits {
                gw.send(prompt(huge.clone(), Vec::new())).await
            } else {
                gw.enqueue(prompt(huge.clone(), Vec::new())).await
            }
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), call)
            .await
            .expect("an over-cap command is answered at once");
        let err = result.expect_err("an over-cap command is refused");
        assert!(
            matches!(&err, ApiError::InvalidRequest(m) if m.contains("8388608")),
            "{err}"
        );
    }
}
