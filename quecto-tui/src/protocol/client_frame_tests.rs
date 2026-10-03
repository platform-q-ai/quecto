//! #2425 review round 2: whether a user message fits one protocol frame,
//! measured without serializing it, against what serde actually writes.

use super::super::{Command, MAX_LINE_BYTES};
use quecto_image::{ImageAttachment, ImageMime, samples};

fn image(mime: ImageMime) -> ImageAttachment {
    ImageAttachment::from_bytes(mime, &samples::sample(mime)).unwrap()
}

fn messages() -> Vec<Command> {
    let texts = [
        String::new(),
        "plain words".to_string(),
        "quotes \" and \\ backslashes".to_string(),
        "lines\nand\ttabs\rand \u{8}\u{c} and \u{1} \u{1f} \u{7f}".to_string(),
        "unicode é ✓ 🦀 and \u{2028}".to_string(),
    ];
    let mut out = Vec::new();
    for text in texts {
        out.push(Command::Prompt {
            id: None,
            message: text.clone(),
            streaming_behavior: None,
            images: Vec::new(),
        });
        out.push(Command::Prompt {
            id: Some("p-\"1\"".into()),
            message: text.clone(),
            streaming_behavior: Some("steer".into()),
            images: vec![image(ImageMime::Png)],
        });
        out.push(Command::Steer {
            id: Some("s-1".into()),
            message: text.clone(),
            images: vec![image(ImageMime::Jpeg), image(ImageMime::Webp)],
        });
        out.push(Command::FollowUp {
            id: None,
            message: text,
            images: vec![image(ImageMime::Gif)],
        });
    }
    out
}

#[test]
fn a_user_message_s_frame_length_is_what_serde_writes() {
    for cmd in messages() {
        let written = serde_json::to_string(&cmd).unwrap().len();
        assert_eq!(cmd.user_message_len(), Some(written), "{cmd:?}");
    }
    assert_eq!(Command::Abort { id: None }.user_message_len(), None);
}

/// The legacy line's cap counts its newline, so a payload exactly at the cap
/// does not fit; one byte under it does.
#[test]
fn the_frame_cap_counts_the_line_s_newline() {
    let empty = Command::Prompt {
        id: None,
        message: String::new(),
        streaming_behavior: None,
        images: Vec::new(),
    };
    let base = serde_json::to_string(&empty).unwrap().len();
    let at = |payload: usize| Command::Prompt {
        id: None,
        message: "a".repeat(payload - base),
        streaming_behavior: None,
        images: Vec::new(),
    };
    assert!(!at(MAX_LINE_BYTES).fits_one_frame());
    assert!(at(MAX_LINE_BYTES - 1).fits_one_frame());
    assert!(empty.fits_one_frame());
}
