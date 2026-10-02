use super::*;
use crate::domain::conversation::image_headers::{
    encode, gif, jpeg, jpeg_with, png, webp_extended, webp_lossless, webp_lossy,
};

const SIZES: [(u16, u16); 7] = [
    (1, 1),
    (800, 600),
    (1920, 1080),
    (4000, 3000),
    (10_000, 10),
    (1, 4_000),
    (3_000, 1_000),
];

fn read(mime: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    image_dimensions(mime, &encode(bytes)).map(|d| (d.width, d.height))
}

fn pair((width, height): (u16, u16)) -> (u32, u32) {
    (u32::from(width), u32::from(height))
}

#[test]
fn png_dimensions_come_from_ihdr() {
    for size in SIZES {
        let (w, h) = pair(size);
        assert_eq!(read("image/png", &png(w, h)), Some((w, h)), "{size:?}");
    }
    let wide = u32::MAX >> 1;
    assert_eq!(read("image/png", &png(wide, 7)), Some((wide, 7)));
}

#[test]
fn jpeg_dimensions_come_from_the_frame_header() {
    for (w, h) in SIZES {
        assert_eq!(read("image/jpeg", &jpeg(w, h)), Some(pair((w, h))));
    }
    assert_eq!(
        read("image/jpeg", &jpeg(65_535, 65_535)),
        Some((65_535, 65_535))
    );
}

/// A camera JPEG puts EXIF (and its thumbnail) before the frame header:
/// the walk jumps over it, whatever the base64 group alignment.
#[test]
fn jpeg_frame_after_a_large_exif_block_is_found() {
    for exif in [1, 2, 3, 60_000, 65_000] {
        let bytes = jpeg_with(1920, 1080, exif, 0xC0);
        assert_eq!(
            read("image/jpeg", &bytes),
            Some((1920, 1080)),
            "exif {exif}"
        );
    }
}

/// Every frame-header marker carries the size: baseline, extended,
/// progressive, lossless, and the arithmetic-coded kinds.
#[test]
fn every_jpeg_frame_kind_is_read() {
    for sof in [
        0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
    ] {
        let bytes = jpeg_with(640, 480, 0, sof);
        assert_eq!(read("image/jpeg", &bytes), Some((640, 480)), "SOF {sof:#x}");
    }
}

/// Fill bytes (0xFF) may pad before any marker.
#[test]
fn jpeg_fill_bytes_before_a_marker_are_skipped() {
    let mut bytes = jpeg(320, 240);
    bytes.splice(2..2, [0xFF, 0xFF, 0xFF]);
    assert_eq!(read("image/jpeg", &bytes), Some((320, 240)));
}

#[test]
fn gif_dimensions_come_from_the_logical_screen_descriptor() {
    for (w, h) in SIZES {
        assert_eq!(read("image/gif", &gif(w, h)), Some(pair((w, h))));
    }
    let mut gif87 = gif(64, 32);
    gif87[..6].copy_from_slice(b"GIF87a");
    assert_eq!(read("image/gif", &gif87), Some((64, 32)));
}

#[test]
fn webp_dimensions_come_from_each_bitstream_kind() {
    for (w, h) in [(1, 1), (800, 600), (1920, 1080), (4000, 3000), (16_383, 10)] {
        assert_eq!(read("image/webp", &webp_lossy(w, h)), Some(pair((w, h))));
    }
    for (w, h) in [(1, 1), (800, 600), (1920, 1080), (4000, 3000), (16_384, 1)] {
        assert_eq!(read("image/webp", &webp_lossless(w, h)), Some((w, h)));
    }
    for (w, h) in [(1, 1), (1920, 1080), (4000, 3000), (16_777_216, 3)] {
        assert_eq!(read("image/webp", &webp_extended(w, h)), Some((w, h)));
    }
}

/// MIME types are case-insensitive; only the four formats are read.
#[test]
fn the_mime_allowlist_is_case_insensitive_and_closed() {
    assert_eq!(read("IMAGE/PNG", &png(3, 4)), Some((3, 4)));
    assert_eq!(read("image/Jpeg", &jpeg(3, 4)), Some((3, 4)));
    for mime in [
        "image/bmp",
        "image/svg+xml",
        "application/octet-stream",
        "",
        "png",
    ] {
        assert_eq!(read(mime, &png(3, 4)), None, "{mime}");
    }
}

/// The MIME type decides the parser: a PNG labelled JPEG is unreadable.
#[test]
fn a_mislabelled_image_is_unreadable() {
    assert_eq!(read("image/jpeg", &png(3, 4)), None);
    assert_eq!(read("image/png", &gif(3, 4)), None);
    assert_eq!(read("image/gif", &webp_lossy(3, 4)), None);
    assert_eq!(read("image/webp", &jpeg(3, 4)), None);
}

#[test]
fn a_truncated_header_is_unreadable() {
    let cases: [(&str, Vec<u8>, usize); 6] = [
        ("image/png", png(800, 600), 23),
        ("image/jpeg", jpeg(800, 600), 97),
        ("image/gif", gif(800, 600), 9),
        ("image/webp", webp_lossy(800, 600), 29),
        ("image/webp", webp_lossless(800, 600), 24),
        ("image/webp", webp_extended(800, 600), 29),
    ];
    for (mime, bytes, keep) in cases {
        for cut in 0..=keep {
            assert_eq!(read(mime, &bytes[..cut]), None, "{mime} cut at {cut}");
        }
    }
}

#[test]
fn a_corrupt_signature_is_unreadable() {
    let mut bad_png = png(800, 600);
    bad_png[1] = b'X';
    let mut bad_ihdr = png(800, 600);
    bad_ihdr[12] = b'J';
    let mut bad_gif = gif(800, 600);
    bad_gif[4] = b'8';
    let mut bad_riff = webp_lossy(800, 600);
    bad_riff[8] = b'X';
    let mut bad_start_code = webp_lossy(800, 600);
    bad_start_code[23] = 0;
    let mut bad_vp8l = webp_lossless(800, 600);
    bad_vp8l[20] = 0x2E;
    let mut bad_chunk = webp_lossy(800, 600);
    bad_chunk[12..16].copy_from_slice(b"ALPH");
    let mut no_soi = jpeg(800, 600);
    no_soi[1] = 0xD9;
    let cases = [
        ("image/png", bad_png),
        ("image/png", bad_ihdr),
        ("image/gif", bad_gif),
        ("image/webp", bad_riff),
        ("image/webp", bad_start_code),
        ("image/webp", bad_vp8l),
        ("image/webp", bad_chunk),
        ("image/jpeg", no_soi),
    ];
    for (n, (mime, bytes)) in cases.into_iter().enumerate() {
        assert_eq!(read(mime, &bytes), None, "case {n}");
    }
}

/// A JPEG whose scan starts before any frame header, whose segment length
/// is under 2, whose marker byte is missing, or which never reaches a frame
/// header within the segment bound is unreadable.
#[test]
fn a_malformed_jpeg_segment_walk_is_unreadable() {
    let sos_first = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 63, 0];
    let short_len = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01, 0, 0, 0, 0];
    let no_marker = [0xFF, 0xD8, 0x00, 0xE0, 0x00, 0x04, 0, 0];
    let eoi = [0xFF, 0xD8, 0xFF, 0xD9];
    for bytes in [&sos_first[..], &short_len, &no_marker, &eoi] {
        assert_eq!(read("image/jpeg", bytes), None, "{bytes:x?}");
    }
    let mut endless = vec![0xFF, 0xD8];
    for _ in 0..10_000 {
        endless.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x02]);
    }
    endless.extend_from_slice(&jpeg(10, 10)[2..]);
    assert_eq!(read("image/jpeg", &endless), None, "past the segment bound");
}

/// A side of 0 (a JPEG sized by a later DNL, or a corrupt header) is not a
/// size.
#[test]
fn a_zero_side_is_unreadable() {
    assert_eq!(read("image/png", &png(0, 600)), None);
    assert_eq!(read("image/jpeg", &jpeg(800, 0)), None);
    assert_eq!(read("image/gif", &gif(0, 0)), None);
    assert_eq!(read("image/webp", &webp_lossy(0, 5)), None);
}

#[test]
fn bad_base64_empty_and_non_ascii_input_are_unreadable_without_panicking() {
    for data in [
        "",
        "=",
        "====",
        "!!!!!!!!",
        "iVBORw0KGgo",
        "中文中文中文中文中文中文中文中文中文中文中文",
    ] {
        for mime in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            assert_eq!(image_dimensions(mime, data), None, "{mime} {data:?}");
        }
    }
    let mut spaced = encode(&png(800, 600));
    spaced.insert(8, '\n');
    assert_eq!(image_dimensions("image/png", &spaced), None);
}

/// Unpadded base64 reads the same as padded.
#[test]
fn unpadded_base64_is_read() {
    let bytes = gif(800, 600);
    let unpadded = encode(&bytes[..10]).trim_end_matches('=').to_string();
    assert_eq!(
        image_dimensions("image/gif", &unpadded).map(|d| d.width),
        Some(800)
    );
}

/// A final group whose unused bits are set (a lax encoder) still decodes.
#[test]
fn non_canonical_trailing_bits_are_read() {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut chars = encode(&gif(800, 600)[..10]).into_bytes();
    assert_eq!(&chars[14..], b"==");
    let value = ALPHABET.iter().position(|c| *c == chars[13]).unwrap();
    assert_eq!(value & 0x0F, 0, "canonical: the unused bits are clear");
    chars[13] = ALPHABET[value | 1];
    let lax = String::from_utf8(chars).unwrap();
    assert_eq!(
        image_dimensions("image/gif", &lax).map(|d| d.height),
        Some(600)
    );
}

/// VP8's top two bits of each side are an upscaling hint, not size.
#[test]
fn the_vp8_upscaling_bits_are_not_size() {
    let mut bytes = webp_lossy(800, 600);
    bytes[27] |= 0xC0;
    bytes[29] |= 0x40;
    assert_eq!(read("image/webp", &bytes), Some((800, 600)));
}

/// The walk reads a frame header behind `MAX_JPEG_SEGMENTS - 1` segments
/// (the frame's own marker is the last step) and gives up one later.
#[test]
fn the_jpeg_segment_bound_is_exact() {
    // `jpeg` has two segments (APP0, DQT) before its frame header.
    let behind = |segments: usize| {
        let mut bytes = vec![0xFF, 0xD8];
        for _ in 0..segments - 2 {
            bytes.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x03, b'c']);
        }
        bytes.extend_from_slice(&jpeg(640, 480)[2..]);
        read("image/jpeg", &bytes)
    };
    // A camera file and a split ICC profile fit: lowering the bound under
    // 256 fails here.
    assert_eq!(behind(255), Some((640, 480)));
    assert_eq!(behind(MAX_JPEG_SEGMENTS - 1), Some((640, 480)));
    assert_eq!(behind(MAX_JPEG_SEGMENTS), None);
    assert_eq!(behind(MAX_JPEG_SEGMENTS + 1), None);
}
