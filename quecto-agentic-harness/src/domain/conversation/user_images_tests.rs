use crate::domain::message::{Message, UserImageBlock};

#[test]
fn an_admitted_attachment_becomes_the_block_providers_send() {
    let payload = quecto_image::ImagePayload::new(
        "image/png",
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC",
    );
    let block = UserImageBlock::from(quecto_image::ImageAttachment::new(payload).unwrap());
    assert_eq!(block.mime(), quecto_image::ImageMime::Png);
    assert_eq!(block.mime_type(), "image/png");
    assert_eq!(
        block.data(),
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC"
    );
    assert!(
        !format!("{block:?}").contains(block.data()),
        "Debug hides it"
    );
    let message = Message::user("look").with_user_images(vec![block]);
    assert_eq!(message.user_image_blocks.len(), 1);
}

#[test]
#[should_panic(expected = "only a user message carries images")]
fn only_a_user_message_carries_images() {
    let block = UserImageBlock::unchecked_for_tests(quecto_image::ImageMime::Png, String::new());
    let _ = Message::assistant("no", vec![]).with_user_images(vec![block]);
}

fn block(mime: &str) -> UserImageBlock {
    let mime = quecto_image::ImageMime::parse_exact(mime).expect("an admitted type");
    UserImageBlock::sample(mime)
}

/// A sample is a real admitted image of its type.
#[test]
fn a_sample_block_is_an_admitted_image_of_its_type() {
    for mime in quecto_image::ImageMime::ALL {
        let block = UserImageBlock::sample(mime);
        assert_eq!(block.mime(), mime);
        let payload = quecto_image::ImagePayload::new(mime.as_str(), block.data());
        assert!(
            quecto_image::ImageAttachment::new(payload).is_ok(),
            "{mime}"
        );
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

/// What persistence kept is re-admitted, never trusted (#2422 review L1).
#[test]
fn a_restored_block_is_readmitted_by_the_strict_rules() {
    use quecto_image::{ImageMime, ImageRefusal};
    let kept = UserImageBlock::sample(ImageMime::Png);
    let restored = UserImageBlock::restore(ImageMime::Png, kept.data().to_owned()).unwrap();
    assert_eq!(restored, kept);
    assert_eq!(
        UserImageBlock::restore(ImageMime::Jpeg, kept.data().to_owned()).unwrap_err(),
        ImageRefusal::SignatureMismatch(ImageMime::Jpeg)
    );
    assert_eq!(
        UserImageBlock::restore(ImageMime::Png, "not base64!".to_owned()).unwrap_err(),
        ImageRefusal::InvalidBase64
    );
    let oversized = "A".repeat(quecto_image::MAX_ENCODED_LEN + 4);
    assert_eq!(
        UserImageBlock::restore(ImageMime::Png, oversized).unwrap_err(),
        ImageRefusal::TooLarge
    );
}
