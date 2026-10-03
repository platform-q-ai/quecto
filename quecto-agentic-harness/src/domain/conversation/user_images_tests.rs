use crate::domain::message::{Message, UserImageBlock};

#[test]
fn an_admitted_attachment_becomes_the_block_providers_send() {
    let payload = quecto_image::ImagePayload::new(
        "image/png",
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC",
    );
    let block = UserImageBlock::from(quecto_image::ImageAttachment::new(payload).unwrap());
    assert_eq!(block.mime_type, "image/png");
    assert_eq!(
        block.data,
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC"
    );
    let message = Message::user("look").with_user_images(vec![block]);
    assert_eq!(message.user_image_blocks.len(), 1);
}

#[test]
#[should_panic(expected = "only a user message carries images")]
fn only_a_user_message_carries_images() {
    let block = UserImageBlock {
        mime_type: "image/png".into(),
        data: String::new(),
    };
    let _ = Message::assistant("no", vec![]).with_user_images(vec![block]);
}

fn block(mime: &str) -> UserImageBlock {
    UserImageBlock {
        mime_type: mime.into(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC".into(),
    }
}

/// Attaching images changes what a message costs: the cached estimate is
/// dropped, never served stale.
#[test]
fn attaching_images_drops_the_cached_token_estimate() {
    let message = Message::user("look");
    let text_only = message.estimated_tokens();
    let message = message.with_user_images(vec![block("image/png")]);
    assert!(message.estimated_tokens() > text_only);
}

#[test]
fn an_images_only_message_is_saved_with_one_placeholder_per_image() {
    let two = Message::user("").with_user_images(vec![block("image/png"), block("image/png")]);
    assert_eq!(super::stored_text(&two), "[image]\n[image]");
    let with_text = Message::user("look").with_user_images(vec![block("image/png")]);
    assert_eq!(super::stored_text(&with_text), "look");
    assert_eq!(super::stored_text(&Message::user("")), "");
    assert_eq!(super::stored_text(&Message::user("hi")), "hi");
}
