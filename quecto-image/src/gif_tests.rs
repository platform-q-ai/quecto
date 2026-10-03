use super::*;
use crate::samples::{animated_gif, encode, gif};

#[test]
fn a_gif_of_more_than_one_frame_is_animated() {
    assert!(!is_animated_gif(&encode(&animated_gif(1))));
    assert!(is_animated_gif(&encode(&animated_gif(2))));
    assert!(is_animated_gif(&encode(&animated_gif(3))));
    assert!(!is_animated_gif(&encode(&gif(4, 4))), "a still sample");
}

#[test]
fn only_a_well_formed_gif_is_animated() {
    assert!(!is_animated_gif("cG5n"), "not a GIF by header");
    assert!(!is_animated_gif("!!!not base64!!!"));
    assert!(!is_animated_gif(""));
    let truncated = &encode(&animated_gif(2))[..40];
    assert!(!is_animated_gif(truncated), "one frame read");
}

/// Base64 is read as for any image already held: padding optional.
#[test]
fn an_unpadded_animated_gif_is_read() {
    let data = encode(&animated_gif(2));
    let unpadded = data.trim_end_matches('=');
    assert_ne!(unpadded.len(), data.len(), "the fixture is padded");
    assert!(is_animated_gif(unpadded));
}
