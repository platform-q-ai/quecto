use super::*;
use quecto_image::samples::{
    encode, gif, jpeg, png, png_with_body, webp_extended, webp_lossless, webp_lossy,
};

fn dims(width: u32, height: u32) -> Dimensions {
    Dimensions { width, height }
}

/// The long edge fits 2576 px, the aspect ratio kept (the short side
/// rounded up, so the estimate errs high); a smaller image is kept as it
/// is.
#[test]
fn scaling_fits_the_long_edge() {
    let cases = [
        ((1, 1), (1, 1)),
        ((800, 600), (800, 600)),
        ((1920, 1080), (1920, 1080)),
        ((2576, 2576), (2576, 2576)),
        ((4000, 3000), (2576, 1932)),
        ((3000, 4000), (1932, 2576)),
        ((3000, 1000), (2576, 859)),
        ((10_000, 10), (2576, 3)),
        ((5000, 55), (2576, 29)),
        ((1, 65_535), (1, 2576)),
        ((16_777_216, 16_777_216), (2576, 2576)),
    ];
    for ((w, h), (sw, sh)) in cases {
        assert_eq!(scaled(dims(w, h)), dims(sw, sh), "{w}x{h}");
    }
}

#[test]
fn scaled_sides_never_pass_the_cap_or_fall_to_zero() {
    for w in [1, 2, 7, 749, 2576, 2577, 4096, 9_999, 65_535, u32::MAX] {
        for h in [1, 3, 600, 2576, 2577, 40_000, u32::MAX] {
            let s = scaled(dims(w, h));
            assert!((1..=MAX_LONG_EDGE).contains(&s.width), "{w}x{h}: {s:?}");
            assert!((1..=MAX_LONG_EDGE).contains(&s.height), "{w}x{h}: {s:?}");
            // Rounding up may make a near-square image square, never
            // turn its long edge short.
            let (long, short) = match w >= h {
                true => (s.width, s.height),
                false => (s.height, s.width),
            };
            assert!(long >= short, "{w}x{h}: the long edge stays long: {s:?}");
        }
    }
}

/// After scaling, the larger of `ceil(w*h/750)` and Anthropic's 28 px
/// patches `ceil(w/28)*ceil(h/28)`, at least 85 and at most 4784.
#[test]
fn every_format_estimates_by_its_pixel_size() {
    let cases: [((u16, u16), usize); 9] = [
        ((1, 1), 85),
        ((800, 600), 640),
        ((1024, 768), 1049),
        ((1920, 1080), 2765),
        ((4000, 3000), 4784),
        ((3000, 1000), 2951),
        ((10_000, 10), 92),
        ((5000, 400), 736),
        ((2576, 29), 184),
    ];
    for ((w, h), tokens) in cases {
        let (w32, h32) = (u32::from(w), u32::from(h));
        let images = [
            ("image/png", png(w32, h32)),
            ("image/jpeg", jpeg(w, h)),
            ("image/gif", gif(w, h)),
            ("image/webp", webp_lossless(w32.min(16_384), h32)),
            ("image/webp", webp_extended(w32, h32)),
        ];
        for (mime, bytes) in images {
            let estimate = estimate_named_image_tokens(mime, &encode(&bytes));
            assert_eq!(estimate, tokens, "{mime} {w}x{h}");
        }
    }
    let lossy = encode(&webp_lossy(1920, 1080));
    assert_eq!(estimate_named_image_tokens("image/webp", &lossy), 2765);
}

/// What cannot be read costs the most an image can: 4,784.
#[test]
fn an_unreadable_image_costs_the_ceiling() {
    let mut truncated = encode(&png(800, 600));
    truncated.truncate(20);
    let cases = [
        ("image/png", truncated),
        ("image/png", encode(b"\x89PNG not really")),
        ("image/bmp", encode(&png(800, 600))),
        ("image/png", String::new()),
        ("image/png", "!!!! not base64 !!!!".to_string()),
        ("image/jpeg", "x".repeat(300)),
    ];
    for (mime, data) in cases {
        assert_eq!(
            estimate_named_image_tokens(mime, &data),
            UNREADABLE_IMAGE_TOKENS,
            "{mime} {data:.24}"
        );
    }
    assert_eq!(UNREADABLE_IMAGE_TOKENS, MAX_IMAGE_TOKENS);
    assert_eq!(MAX_IMAGE_TOKENS, 4_784);
    assert_eq!(MIN_IMAGE_TOKENS, 85);
}

/// The issue's case: a 1 MB screenshot is priced by its pixels (2765),
/// not its base64 (~350k at ASCII/4); the size of the body changes nothing.
#[test]
fn a_one_megabyte_screenshot_estimates_under_five_thousand_tokens() {
    let small = encode(&png_with_body(1920, 1080, 64));
    let large = encode(&png_with_body(1920, 1080, 1 << 20));
    assert!(large.len() > 1_300_000);
    assert_eq!(
        estimate_named_image_tokens("image/png", &large),
        2765,
        "under 5,000"
    );
    assert_eq!(
        estimate_named_image_tokens("image/png", &small),
        estimate_named_image_tokens("image/png", &large)
    );
}

/// Every estimate is within [85, 4784], however odd the size.
#[test]
fn every_estimate_is_within_the_floor_and_the_ceiling() {
    for w in [1, 2, 100, 2576, 2577, 4096, 16_383, 16_384] {
        for h in [1, 3, 600, 2576, 4_000, 16_384] {
            let estimate = estimate_named_image_tokens("image/webp", &encode(&webp_lossless(w, h)));
            assert!(
                (MIN_IMAGE_TOKENS..=MAX_IMAGE_TOKENS).contains(&estimate),
                "{w}x{h}"
            );
        }
    }
}

/// A thin strip is billed by 28 px patches, more than its area says:
/// 2576x29 is 92x2 patches (184), not ceil(74,704/750) (100).
#[test]
fn a_thin_image_costs_its_patches() {
    let strip = encode(&png(2576, 29));
    assert_eq!(estimate_named_image_tokens("image/png", &strip), 184);
    let line = encode(&png(10_000, 10));
    assert_eq!(
        estimate_named_image_tokens("image/png", &line),
        92,
        "2576x3 is 92 patches"
    );
    // Scaled to 2576x28.3: rounded up to 29 rows, two patch rows (184),
    // not rounded down to 28 and one (97).
    let scaled_strip = encode(&png(5000, 55));
    assert_eq!(estimate_named_image_tokens("image/png", &scaled_strip), 184);
    let wide = encode(&png(1920, 1080));
    assert_eq!(
        estimate_named_image_tokens("image/png", &wide),
        2765,
        "the area rate is larger"
    );
}

/// The typed estimate is the one a caller holding an `ImageMime` uses; the
/// string entry agrees with it and costs a type off the allowlist the most.
#[test]
fn the_typed_and_the_named_estimates_agree() {
    let data = encode(&png(1920, 1080));
    assert_eq!(
        estimate_image_tokens(quecto_image::ImageMime::Png, &data),
        estimate_named_image_tokens("image/png", &data)
    );
    assert_eq!(
        estimate_named_image_tokens("IMAGE/PNG", &data),
        UNREADABLE_IMAGE_TOKENS
    );
}
