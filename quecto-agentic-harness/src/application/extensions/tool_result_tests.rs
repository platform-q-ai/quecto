use quecto_image::samples::{encode, png};
use quecto_image::{ImageMime, ImagePayload, ImageRefusal};

use super::*;

fn png_payload() -> ImagePayload {
    ImagePayload::new("image/png", encode(&png(2, 1)))
}

#[test]
fn admitted_images_keep_the_text_and_the_error_flag() {
    for is_error in [false, true] {
        let sent = SentImageBlocks::Entries(vec![Some(png_payload())]);
        let result = extension_tool_result("shot".into(), is_error, sent);
        assert_eq!(result.content, "shot");
        assert_eq!(result.is_error, is_error);
        assert_eq!(result.image_blocks.len(), 1);
        assert_eq!(result.image_blocks[0].mime(), ImageMime::Png);
        assert_eq!(result.image_blocks[0].data(), png_payload().data);
    }
}

#[test]
fn no_images_is_the_text_result_unchanged() {
    let result = extension_tool_result("t".into(), false, SentImageBlocks::default());
    assert_eq!((result.content.as_str(), result.is_error), ("t", false));
    assert!(result.image_blocks.is_empty());
}

#[test]
fn the_count_is_checked_before_any_entry() {
    let mut entries = vec![None];
    entries.extend((0..MAX_IMAGES_PER_MESSAGE).map(|_| Some(png_payload())));
    let refusal = admit(SentImageBlocks::Entries(entries)).unwrap_err();
    assert_eq!(
        refusal,
        ToolImagesRefusal::TooMany(MAX_IMAGES_PER_MESSAGE + 1)
    );
}

#[test]
fn the_first_failing_entry_is_named() {
    let sent = SentImageBlocks::Entries(vec![
        Some(png_payload()),
        Some(ImagePayload::new("image/png", "!!")),
        None,
    ]);
    assert_eq!(
        admit(sent).unwrap_err(),
        ToolImagesRefusal::Image {
            index: 1,
            refusal: ImageRefusal::InvalidBase64
        }
    );
}

#[test]
fn a_refused_result_without_text_is_the_refusal_alone() {
    let result = extension_tool_result(String::new(), false, SentImageBlocks::NotAList);
    assert!(result.is_error);
    assert_eq!(
        result.content,
        r#"Error: imageBlocks: expected an array of {"mimeType", "data"} objects"#
    );
}

#[test]
fn an_image_refused_before_a_malformed_entry_is_the_one_named() {
    let bad = Some(ImagePayload::new("image/gif", encode(&png(1, 1))));
    let sent = SentImageBlocks::Entries(vec![Some(png_payload()), bad, None]);
    assert_eq!(
        admit(sent).unwrap_err(),
        ToolImagesRefusal::Image {
            index: 1,
            refusal: ImageRefusal::SignatureMismatch(ImageMime::Gif)
        }
    );
    let sent = SentImageBlocks::Entries(vec![Some(png_payload()), None, Some(png_payload())]);
    assert_eq!(admit(sent).unwrap_err(), ToolImagesRefusal::Malformed(1));
}

#[test]
fn a_list_counted_past_the_limit_is_refused_by_its_length() {
    let sent = SentImageBlocks::TooMany(1_000_000);
    assert_eq!(
        admit(sent).unwrap_err().to_string(),
        "too many imageBlocks: 1000000; at most 8 per tool result"
    );
}
