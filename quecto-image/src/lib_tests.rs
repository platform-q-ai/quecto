use super::*;
use crate::samples::{gif, jpeg, png, png_with_body, webp_extended, webp_lossless, webp_lossy};

fn payload(mime: &str, bytes: &[u8]) -> ImagePayload {
    ImagePayload::new(mime, encode(bytes))
}

fn refusal(payload: ImagePayload) -> String {
    ImageAttachment::new(payload).unwrap_err().to_string()
}

#[test]
fn admits_each_allowed_type_with_a_readable_header() {
    let gif87 = {
        let mut bytes = gif(5, 6);
        bytes[..6].copy_from_slice(b"GIF87a");
        bytes
    };
    for (mime, bytes) in [
        ("image/png", png(3, 4)),
        ("image/jpeg", jpeg(3, 4)),
        ("image/gif", gif(3, 4)),
        ("image/gif", gif87),
        ("image/webp", webp_lossy(3, 4)),
        ("image/webp", webp_lossless(3, 4)),
        ("image/webp", webp_extended(3, 4)),
    ] {
        let image = ImageAttachment::new(payload(mime, &bytes)).expect(mime);
        assert_eq!(image.mime_type(), mime);
        assert_eq!(image.mime(), ImageMime::parse_exact(mime).unwrap());
        assert_eq!(image.decoded_len(), bytes.len());
        assert_eq!(image.data(), encode(&bytes));
        assert!(image.dimensions().width > 0, "{mime}");
    }
    let image = ImageAttachment::new(payload("image/png", &png(3, 4))).unwrap();
    assert_eq!(
        image.dimensions(),
        Dimensions {
            width: 3,
            height: 4
        }
    );
}

#[test]
fn refuses_a_type_off_the_allowlist_with_its_exact_message() {
    assert_eq!(
        refusal(payload("image/svg+xml", &png(1, 1))),
        "mimeType \"image/svg+xml\" is not allowed; use image/png, image/jpeg, image/gif or image/webp"
    );
    // The match is exact: no case folding, no parameters.
    for declared in ["IMAGE/PNG", "image/png; charset=binary", " image/png", ""] {
        assert!(
            matches!(
                ImageAttachment::new(payload(declared, &png(1, 1))),
                Err(ImageRefusal::UnsupportedMime(_))
            ),
            "{declared:?} must be refused"
        );
    }
}

#[test]
fn an_unsupported_type_is_echoed_cut_and_escaped() {
    let declared = format!("image/{}\n", "x".repeat(200));
    let ImageRefusal::UnsupportedMime(echoed) =
        ImageAttachment::new(payload(&declared, &png(1, 1))).unwrap_err()
    else {
        panic!("expected an unsupported type");
    };
    assert_eq!(echoed.chars().count(), 64);
    assert!(!refusal(payload("image/a\nb", &png(1, 1))).contains('\n'));
}

#[test]
fn refuses_data_that_is_not_strict_standard_base64() {
    let bytes = png(800, 600);
    let mut wrapped = encode(&bytes);
    wrapped.insert(4, '\n');
    let url_safe = encode(&[0xFB, 0xFF, 0xFE])
        .replace('+', "-")
        .replace('/', "_");
    let unpadded = encode(&bytes).trim_end_matches('=').to_string();
    assert!(
        encode(&bytes).ends_with('='),
        "the fixture must need padding"
    );
    for data in [wrapped, url_safe, unpadded, "not base64!".to_string()] {
        assert_eq!(
            refusal(ImagePayload::new("image/png", data.clone())),
            "data is not valid standard base64",
            "{data:?}"
        );
    }
}

#[test]
fn refuses_bytes_that_are_not_the_declared_type() {
    assert_eq!(
        refusal(payload("image/png", &jpeg(2, 2))),
        "data does not start with the image/png signature"
    );
    assert_eq!(
        refusal(payload("image/webp", b"RIFF\0\0\0\0WAVE")),
        "data does not start with the image/webp signature"
    );
    assert_eq!(
        refusal(ImagePayload::new("image/gif", "")),
        "data does not start with the image/gif signature"
    );
}

/// A signature alone is not an image: its header must give a pixel size.
#[test]
fn refuses_a_bare_signature_or_a_broken_header_as_unreadable() {
    let mut truncated = png(800, 600);
    truncated.truncate(20);
    let mut zero_side = gif(0, 4);
    zero_side.truncate(30);
    for (mime, bytes) in [
        ("image/png", b"\x89PNG\r\n\x1a\n".to_vec()),
        ("image/webp", b"RIFF\x04\0\0\0WEBP".to_vec()),
        ("image/jpeg", vec![0xFF, 0xD8, 0xFF]),
        ("image/gif", b"GIF89a".to_vec()),
        ("image/png", truncated),
        ("image/gif", zero_side),
    ] {
        assert_eq!(
            refusal(payload(mime, &bytes)),
            format!("not a readable {mime} image"),
            "{bytes:x?}"
        );
    }
}

#[test]
fn admits_exactly_the_size_limit_and_refuses_one_byte_more() {
    assert_eq!(MAX_IMAGE_BYTES, 3_932_160, "3.75 MiB");
    assert_eq!(MAX_ENCODED_LEN, 5 * 1024 * 1024, "its base64 is 5 MiB");
    let at_limit = png_with_body(2, 2, MAX_IMAGE_BYTES - 57);
    assert_eq!(at_limit.len(), MAX_IMAGE_BYTES);
    let image = ImageAttachment::new(payload("image/png", &at_limit)).unwrap();
    assert_eq!(image.decoded_len(), MAX_IMAGE_BYTES);
    assert_eq!(image.data().len(), MAX_ENCODED_LEN);
    let over = png_with_body(2, 2, MAX_IMAGE_BYTES - 56);
    assert_eq!(
        refusal(payload("image/png", &over)),
        "image decodes to more than 3932160 bytes (3.75 MiB)"
    );
}

#[test]
fn refuses_oversized_text_before_decoding_it() {
    // Not base64 at all: only the length bound can refuse it.
    let data = "!".repeat(MAX_ENCODED_LEN + 1);
    assert_eq!(
        ImageAttachment::new(ImagePayload::new("image/png", data)).unwrap_err(),
        ImageRefusal::TooLarge
    );
}

#[test]
fn a_file_read_from_disk_is_admitted_from_its_bytes() {
    let bytes = jpeg(640, 480);
    let image = ImageAttachment::from_bytes(ImageMime::Jpeg, &bytes).unwrap();
    assert_eq!(image.data(), encode(&bytes));
    assert_eq!(image.dimensions().width, 640);
    assert_eq!(
        ImageAttachment::from_bytes(ImageMime::Png, &[0x89, b'P', b'N', b'G']).unwrap_err(),
        ImageRefusal::SignatureMismatch(ImageMime::Png)
    );
    assert_eq!(
        ImageAttachment::from_bytes(ImageMime::Png, b"\x89PNG\r\n\x1a\n").unwrap_err(),
        ImageRefusal::Unreadable(ImageMime::Png)
    );
    let too_big = png_with_body(2, 2, MAX_IMAGE_BYTES);
    assert_eq!(
        ImageAttachment::from_bytes(ImageMime::Png, &too_big).unwrap_err(),
        ImageRefusal::TooLarge
    );
}

#[test]
fn a_message_takes_at_most_eight_images() {
    let eight = vec![payload("image/png", &png(1, 1)); MAX_IMAGES_PER_MESSAGE];
    assert_eq!(validate_images(eight).unwrap().len(), 8);
    let nine = vec![payload("image/png", &png(1, 1)); 9];
    assert_eq!(
        validate_images(nine).unwrap_err().to_string(),
        "too many images: 9; at most 8 per message"
    );
}

#[test]
fn a_refusal_names_the_failing_image_and_refuses_the_whole_list() {
    let images = vec![
        payload("image/png", &png(1, 1)),
        payload("image/jpeg", &png(1, 1)),
        payload("image/gif", b"GIF"),
    ];
    assert_eq!(
        validate_images(images).unwrap_err().to_string(),
        "images[1]: data does not start with the image/jpeg signature"
    );
    assert_eq!(validate_images(Vec::new()).unwrap(), Vec::new());
}

#[test]
fn the_payload_round_trips_in_its_wire_shape() {
    let wire = r#"{"mimeType":"image/png","data":"AAAA"}"#;
    let payload: ImagePayload = serde_json::from_str(wire).unwrap();
    assert_eq!(payload, ImagePayload::new("image/png", "AAAA"));
    assert_eq!(serde_json::to_string(&payload).unwrap(), wire);
}

#[test]
fn an_attachment_serialises_as_the_payload_it_came_from() {
    let sent = payload("image/webp", &webp_lossy(2, 2));
    let image = ImageAttachment::new(sent.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&image).unwrap(),
        serde_json::to_value(&sent).unwrap()
    );
    assert_eq!(image.into_data(), sent.data);
}

#[test]
fn debug_output_never_carries_the_base64() {
    let sent = payload("image/png", &png(1, 1));
    let image = ImageAttachment::new(sent.clone()).unwrap();
    for shown in [format!("{sent:?}"), format!("{image:?}")] {
        assert!(!shown.contains(&sent.data), "{shown}");
    }
}

#[test]
fn every_type_parses_from_its_exact_name_only() {
    for mime in ImageMime::ALL {
        assert_eq!(ImageMime::parse_exact(mime.as_str()), Some(mime));
        assert_eq!(ImageMime::parse_exact(&mime.as_str().to_uppercase()), None);
        assert_eq!(mime.to_string(), mime.as_str());
    }
    assert_eq!(ImageMime::parse_exact("image/bmp"), None);
}

#[test]
fn sniff_names_the_type_a_file_starts_with() {
    assert_eq!(ImageMime::sniff(&png(1, 1)), Some(ImageMime::Png));
    assert_eq!(ImageMime::sniff(&jpeg(1, 1)), Some(ImageMime::Jpeg));
    assert_eq!(ImageMime::sniff(b"GIF87a\x01"), Some(ImageMime::Gif));
    assert_eq!(ImageMime::sniff(&gif(1, 1)), Some(ImageMime::Gif));
    assert_eq!(ImageMime::sniff(&webp_lossy(1, 1)), Some(ImageMime::Webp));
    for bytes in [&b"hello world"[..], b"", b"RIFF\0\0\0\0WAVE", b"\x89PN"] {
        assert_eq!(ImageMime::sniff(bytes), None, "{bytes:x?}");
    }
}

#[test]
fn every_sample_is_an_admitted_image_of_its_type() {
    for mime in ImageMime::ALL {
        let bytes = crate::samples::sample(mime);
        assert_eq!(ImageMime::sniff(&bytes), Some(mime));
        assert!(ImageAttachment::from_bytes(mime, &bytes).is_ok(), "{mime}");
    }
}

/// The refusal text names the allowlist and the limit from their sources.
#[test]
fn refusal_text_comes_from_the_allowlist_and_the_limit() {
    let types: Vec<&str> = ImageMime::ALL.iter().map(|m| m.as_str()).collect();
    let text = ImageRefusal::UnsupportedMime("x".into()).to_string();
    for name in types {
        assert!(text.contains(name), "{text}");
    }
    assert_eq!(mebibytes(MAX_IMAGE_BYTES), "3.75");
    assert_eq!(mebibytes(4 * 1024 * 1024), "4");
    assert_eq!(mebibytes(5 * 1024 * 1024 / 2), "2.5");
}

#[test]
fn a_null_images_field_reads_as_none() {
    #[derive(serde::Deserialize)]
    struct Message {
        #[serde(default, deserialize_with = "images_or_null")]
        images: Vec<ImagePayload>,
    }
    for wire in [r#"{"images":null}"#, "{}", r#"{"images":[]}"#] {
        let message: Message = serde_json::from_str(wire).unwrap();
        assert!(message.images.is_empty(), "{wire}");
    }
    let one: Message =
        serde_json::from_str(r#"{"images":[{"mimeType":"image/png","data":"AA=="}]}"#).unwrap();
    assert_eq!(one.images.len(), 1);
}
