use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];
const GIF87: &[u8] = b"GIF87a\x01\0\x01\0";
const GIF89: &[u8] = b"GIF89a\x01\0\x01\0";
const WEBP: &[u8] = b"RIFF\x24\0\0\0WEBPVP8 ";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn payload(mime: &str, bytes: &[u8]) -> ImagePayload {
    ImagePayload::new(mime, b64(bytes))
}

fn refusal(payload: ImagePayload) -> String {
    ImageAttachment::new(payload).unwrap_err().to_string()
}

#[test]
fn admits_each_allowed_type_with_its_signature() {
    for (mime, bytes) in [
        ("image/png", PNG),
        ("image/jpeg", JPEG),
        ("image/gif", GIF87),
        ("image/gif", GIF89),
        ("image/webp", WEBP),
    ] {
        let image = ImageAttachment::new(payload(mime, bytes)).expect(mime);
        assert_eq!(image.mime_type(), mime);
        assert_eq!(image.decoded_len(), bytes.len());
        assert_eq!(image.data(), b64(bytes));
    }
}

#[test]
fn refuses_a_type_off_the_allowlist_with_its_exact_message() {
    assert_eq!(
        refusal(payload("image/svg+xml", PNG)),
        "mimeType \"image/svg+xml\" is not allowed; use image/png, image/jpeg, image/gif or image/webp"
    );
    // The match is exact: no case folding, no parameters.
    for declared in ["IMAGE/PNG", "image/png; charset=binary", " image/png", ""] {
        assert!(
            matches!(
                ImageAttachment::new(payload(declared, PNG)),
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
        ImageAttachment::new(payload(&declared, PNG)).unwrap_err()
    else {
        panic!("expected an unsupported type");
    };
    assert_eq!(echoed.chars().count(), 64);
    assert!(!refusal(payload("image/a\nb", PNG)).contains('\n'));
}

#[test]
fn refuses_data_that_is_not_standard_base64() {
    let mut wrapped = b64(PNG);
    wrapped.insert(4, '\n');
    let url_safe = b64(&[0xFB, 0xFF, 0xFE]).replace('+', "-").replace('/', "_");
    let unpadded = b64(PNG).trim_end_matches('=').to_string();
    assert!(b64(PNG).ends_with('='), "the fixture must need padding");
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
        refusal(payload("image/png", JPEG)),
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

#[test]
fn admits_exactly_the_size_limit_and_refuses_one_byte_more() {
    let mut at_limit = PNG.to_vec();
    at_limit.resize(MAX_IMAGE_BYTES, 0);
    assert_eq!(
        ImageAttachment::new(payload("image/png", &at_limit))
            .unwrap()
            .decoded_len(),
        MAX_IMAGE_BYTES
    );
    at_limit.push(0);
    assert_eq!(
        refusal(payload("image/png", &at_limit)),
        "image decodes to more than 5242880 bytes (5 MiB)"
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
fn a_message_takes_at_most_eight_images() {
    let eight = vec![payload("image/png", PNG); MAX_IMAGES_PER_MESSAGE];
    assert_eq!(validate_images(eight.clone()).unwrap().len(), 8);
    assert!(check_images(&eight).is_ok());
    let nine = vec![payload("image/png", PNG); 9];
    let expected = "too many images: 9; at most 8 per message";
    assert_eq!(
        validate_images(nine.clone()).unwrap_err().to_string(),
        expected
    );
    assert_eq!(check_images(&nine).unwrap_err().to_string(), expected);
}

#[test]
fn a_refusal_names_the_failing_image_and_refuses_the_whole_list() {
    let images = vec![
        payload("image/png", PNG),
        payload("image/jpeg", PNG),
        payload("image/gif", b"GIF"),
    ];
    let expected = "images[1]: data does not start with the image/jpeg signature";
    assert_eq!(check_images(&images).unwrap_err().to_string(), expected);
    assert_eq!(validate_images(images).unwrap_err().to_string(), expected);
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
    let sent = payload("image/webp", WEBP);
    let image = ImageAttachment::new(sent.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&image).unwrap(),
        serde_json::to_value(&sent).unwrap()
    );
    assert_eq!(image.into_data(), sent.data);
}

#[test]
fn debug_output_never_carries_the_base64() {
    let sent = payload("image/png", PNG);
    let image = ImageAttachment::new(sent.clone()).unwrap();
    for shown in [format!("{sent:?}"), format!("{image:?}")] {
        assert!(!shown.contains(&sent.data), "{shown}");
    }
}

#[test]
fn every_type_parses_from_its_own_name_only() {
    for mime in ImageMime::ALL {
        assert_eq!(ImageMime::parse(mime.as_str()), Some(mime));
        assert_eq!(mime.to_string(), mime.as_str());
    }
    assert_eq!(ImageMime::parse("image/bmp"), None);
}
