use crate::domain::message::{Message, UserImageBlock};

#[test]
fn an_admitted_attachment_becomes_the_block_providers_send() {
    let payload = quecto_image::ImagePayload::new("image/png", "iVBORw0KGgoAAAANSUhEUg==");
    let block = UserImageBlock::from(quecto_image::ImageAttachment::new(payload).unwrap());
    assert_eq!(block.mime_type, "image/png");
    assert_eq!(block.data, "iVBORw0KGgoAAAANSUhEUg==");
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
