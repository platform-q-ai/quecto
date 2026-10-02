//! #2421: an image goes to a model only when the model declares image input;
//! otherwise each image is a marker in the message it belonged to. The
//! conversation is never copied, and gets every image back afterwards.
use super::*;
use crate::domain::message::UserImageBlock;
use crate::domain::tool::ImageBlock;
use base64::Engine;

const MODEL: &str = "fireworks/glm-5p2";

fn modalities(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn user_with_images(text: &str, count: usize) -> Message {
    let mut message = Message::user(text);
    message.user_image_blocks = (0..count)
        .map(|index| UserImageBlock {
            mime_type: "image/png".into(),
            data: format!("cG5n{index}"),
        })
        .collect();
    message
}

fn tool_with_image(id: &str, text: &str) -> Message {
    let mut message = Message::tool(id, text);
    message.tool_name = Some("read".into());
    message.image_blocks = vec![ImageBlock {
        mime_type: "image/jpeg",
        data: "anBn".into(),
    }];
    message
}

/// A GIF of `frames` 1x1 frames, each after a graphic control extension,
/// with a global colour table and a comment extension, as base64.
fn gif(frames: usize) -> String {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend([1, 0, 1, 0, 0x80, 0, 0]); // 1x1, a 2-colour global table
    bytes.extend([0, 0, 0, 255, 255, 255]);
    bytes.extend([0x21, 0xFE, 3, b'h', b'e', b'y', 0]); // comment
    for _ in 0..frames {
        bytes.extend([0x21, 0xF9, 4, 0, 10, 0, 0, 0]); // graphic control
        bytes.extend([0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]); // image descriptor
        bytes.extend([2, 2, 0x4C, 0x01, 0]); // LZW size, one sub-block, end
    }
    bytes.push(0x3B);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn tool_with_gif(id: &str, frames: usize) -> Message {
    let mut message = Message::tool(id, "Read image file a.gif");
    message.image_blocks = vec![
        ImageBlock {
            mime_type: "image/png",
            data: "cG5n".into(),
        },
        ImageBlock {
            mime_type: "image/gif",
            data: gif(frames),
        },
        ImageBlock {
            mime_type: "image/jpeg",
            data: "anBn".into(),
        },
    ];
    message
}

#[test]
fn a_model_declaring_image_input_takes_every_image_over_anthropic() {
    let declared = modalities(&["text", "image"]);
    assert_eq!(
        ImageInput::declared(&declared, &TransportKind::AnthropicMessages),
        ImageInput::AllImages
    );
}

#[test]
fn a_model_declaring_image_input_takes_still_images_over_openai() {
    let declared = modalities(&["text", "image"]);
    assert_eq!(
        ImageInput::declared(&declared, &TransportKind::OpenAiCompletions),
        ImageInput::StillImages
    );
}

#[test]
fn a_model_that_declares_no_image_input_takes_none() {
    for transport in [
        TransportKind::AnthropicMessages,
        TransportKind::OpenAiCompletions,
    ] {
        for declared in [
            modalities(&["text"]),
            modalities(&["text", "audio"]),
            vec![],
        ] {
            assert_eq!(
                ImageInput::declared(&declared, &transport),
                ImageInput::NoImages,
                "{declared:?} over {transport:?}"
            );
        }
    }
    assert_eq!(ImageInput::default(), ImageInput::NoImages, "no entry");
}

#[test]
fn the_markers_name_the_model() {
    assert_eq!(
        not_sent_marker(MODEL),
        "[image not sent: fireworks/glm-5p2 takes no image input]"
    );
    assert_eq!(
        animated_gif_marker(MODEL),
        "[image not sent: animated GIF not supported by fireworks/glm-5p2]"
    );
}

#[test]
fn a_gif_of_more_than_one_frame_is_animated() {
    assert!(!is_animated_gif("image/gif", &gif(1)));
    assert!(is_animated_gif("image/gif", &gif(2)));
    assert!(is_animated_gif("IMAGE/GIF", &gif(3)));
}

#[test]
fn only_a_well_formed_gif_is_animated() {
    assert!(!is_animated_gif("image/png", &gif(2)), "not a GIF by type");
    assert!(!is_animated_gif("image/gif", "cG5n"), "not a GIF by header");
    assert!(!is_animated_gif("image/gif", "!!!not base64!!!"));
    assert!(!is_animated_gif("image/gif", ""));
    let truncated = &gif(2)[..40];
    assert!(!is_animated_gif("image/gif", truncated), "one frame read");
}

#[test]
fn a_model_that_takes_every_image_is_sent_the_conversation_as_it_is() {
    let mut messages = vec![user_with_images("look", 1), tool_with_gif("c1", 2)];
    let stored = messages.as_ptr();
    let sent = SentConversation::new(&mut messages, MODEL, ImageInput::AllImages);
    assert_eq!(sent.messages().as_ptr(), stored, "nothing is copied");
    assert_eq!(sent.messages()[0].user_image_blocks.len(), 1);
    assert_eq!(sent.messages()[1].image_blocks.len(), 3);
}

#[test]
fn each_user_image_becomes_a_marker_after_the_text() {
    let mut messages = vec![user_with_images("compare these", 2)];
    let marker = not_sent_marker(MODEL);
    {
        let sent = SentConversation::new(&mut messages, MODEL, ImageInput::NoImages);
        assert_eq!(
            sent.messages()[0].content,
            format!("compare these\n{marker}\n{marker}")
        );
        assert!(sent.messages()[0].user_image_blocks.is_empty());
    }
    assert_eq!(messages[0].content, "compare these", "put back");
    assert_eq!(messages[0].user_image_blocks.len(), 2);
    assert_eq!(messages[0].user_image_blocks[1].data, "cG5n1", "in order");
}

#[test]
fn an_image_with_no_text_becomes_the_marker_alone() {
    let mut messages = vec![user_with_images("", 1)];
    let sent = SentConversation::new(&mut messages, MODEL, ImageInput::NoImages);
    assert_eq!(sent.messages()[0].content, not_sent_marker(MODEL));
}

#[test]
fn messages_without_images_are_neither_copied_nor_changed() {
    let mut messages = vec![
        Message::user("read it"),
        tool_with_image("c1", "Read image file a.jpg"),
        Message::tool("c2", "plain"),
    ];
    let stored = messages.as_ptr();
    let texts = [messages[0].content.as_ptr(), messages[2].content.as_ptr()];
    let sent = SentConversation::new(&mut messages, MODEL, ImageInput::NoImages);
    assert_eq!(
        sent.messages().as_ptr(),
        stored,
        "the conversation is not copied"
    );
    assert_eq!(sent.messages()[0].content.as_ptr(), texts[0]);
    assert_eq!(sent.messages()[2].content.as_ptr(), texts[1]);
    assert_eq!(
        sent.messages()[1].content,
        format!("Read image file a.jpg\n{}", not_sent_marker(MODEL))
    );
    assert!(sent.messages()[1].image_blocks.is_empty());
    assert_eq!(sent.messages()[1].tool_call_id.as_deref(), Some("c1"));
}

#[test]
fn a_still_image_model_is_sent_every_image_but_an_animated_gif() {
    let mut messages = vec![tool_with_gif("c1", 2), tool_with_gif("c2", 1)];
    {
        let sent = SentConversation::new(&mut messages, MODEL, ImageInput::StillImages);
        let animated = &sent.messages()[0];
        assert_eq!(
            animated.content,
            format!("Read image file a.gif\n{}", animated_gif_marker(MODEL))
        );
        let kept: Vec<&str> = animated.image_blocks.iter().map(|i| i.mime_type).collect();
        assert_eq!(kept, ["image/png", "image/jpeg"]);
        let still = &sent.messages()[1];
        assert_eq!(still.content, "Read image file a.gif");
        assert_eq!(still.image_blocks.len(), 3);
    }
    let restored: Vec<&str> = messages[0]
        .image_blocks
        .iter()
        .map(|i| i.mime_type)
        .collect();
    assert_eq!(
        restored,
        ["image/png", "image/gif", "image/jpeg"],
        "in order"
    );
    assert_eq!(messages[0].content, "Read image file a.gif");
}

#[test]
fn the_token_estimate_is_the_conversations_again_afterwards() {
    let mut messages = vec![user_with_images("look", 2)];
    let before = messages[0].estimated_tokens();
    {
        let sent = SentConversation::new(&mut messages, MODEL, ImageInput::NoImages);
        assert_ne!(sent.messages()[0].estimated_tokens(), before);
    }
    assert_eq!(messages[0].estimated_tokens(), before);
}

#[test]
fn a_message_is_marked_the_same_whatever_follows_it() {
    let mut earlier = vec![user_with_images("look", 1)];
    let mut later = earlier.clone();
    later.push(Message::assistant("seen", vec![]));
    later.push(tool_with_image("c1", "more"));
    let first = SentConversation::new(&mut earlier, MODEL, ImageInput::NoImages).messages()[0]
        .content
        .clone();
    let second = SentConversation::new(&mut later, MODEL, ImageInput::NoImages).messages()[0]
        .content
        .clone();
    assert_eq!(first, second);
}
