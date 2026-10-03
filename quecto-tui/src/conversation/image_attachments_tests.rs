//! #2425: the composer's attachment policy, against real image files from
//! `quecto_image::samples`.

use super::*;
use quecto_image::samples;

/// A PNG whose file is about `bytes` long.
fn png_of(bytes: usize) -> Vec<u8> {
    samples::png_with_body(4, 4, bytes)
}

#[test]
fn an_admitted_image_is_kept_with_its_name_and_sent_as_admitted() {
    let mut pending = PendingImages::default();
    let png = samples::png(2, 3);
    pending
        .admit("shot.png", &png)
        .expect("a real PNG is admitted");

    assert_eq!(pending.chips().len(), 1);
    let sent = pending.attachments();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].mime_type(), "image/png");
    assert_eq!(sent[0].data(), quecto_image::encode(&png));
}

#[test]
fn the_type_comes_from_the_bytes_not_the_name() {
    let mut pending = PendingImages::default();
    pending
        .admit("photo.png", &samples::jpeg(1, 1))
        .expect("a JPEG named .png is still a JPEG");
    assert_eq!(pending.attachments()[0].mime_type(), "image/jpeg");
}

#[test]
fn every_admitted_type_is_attachable() {
    let mut pending = PendingImages::default();
    for mime in quecto_image::ImageMime::ALL {
        pending
            .admit(mime.as_str(), &samples::sample(mime))
            .unwrap_or_else(|refusal| panic!("{mime}: {refusal}"));
    }
    let types: Vec<_> = pending
        .attachments()
        .iter()
        .map(|image| image.mime_type())
        .collect();
    assert_eq!(
        types,
        ["image/png", "image/jpeg", "image/gif", "image/webp"]
    );
}

#[test]
fn bytes_that_are_no_admitted_image_are_refused_and_not_kept() {
    let mut pending = PendingImages::default();
    let refusal = pending
        .admit("notes.txt", b"just some text")
        .expect_err("text is not an image");
    assert_eq!(refusal, AttachRefusal::NotAnImage);
    assert!(pending.is_empty());
    assert_eq!(
        refusal.to_string(),
        "not an image/png, image/jpeg, image/gif or image/webp file"
    );
}

#[test]
fn quecto_image_refusals_are_passed_on_word_for_word() {
    let mut pending = PendingImages::default();
    let too_big = png_of(quecto_image::MAX_IMAGE_BYTES);
    let refusal = pending.admit("huge.png", &too_big).expect_err("too large");
    assert_eq!(
        refusal,
        AttachRefusal::Refused(quecto_image::ImageRefusal::TooLarge)
    );
    assert_eq!(
        refusal.to_string(),
        "image decodes to more than 3932160 bytes (3.75 MiB)"
    );

    let bare = b"\x89PNG\r\n\x1a\n".to_vec();
    let refusal = pending.admit("bare.png", &bare).expect_err("no header");
    assert_eq!(refusal.to_string(), "not a readable image/png image");
    assert!(pending.is_empty());
}

#[test]
fn a_message_holds_at_most_eight_images() {
    let mut pending = PendingImages::default();
    for n in 0..quecto_image::MAX_IMAGES_PER_MESSAGE {
        pending
            .admit(&format!("{n}.png"), &samples::png(1, 1))
            .expect("within the count");
    }
    let refusal = pending
        .admit("ninth.png", &samples::png(1, 1))
        .expect_err("the ninth is refused");
    assert_eq!(refusal, AttachRefusal::TooMany);
    assert_eq!(refusal.to_string(), "at most 8 images per message");
    assert_eq!(pending.chips().len(), 8);
}

#[test]
fn a_message_s_images_must_fit_one_protocol_frame() {
    let mut pending = PendingImages::default();
    let large = png_of(quecto_image::MAX_IMAGE_BYTES - 100);
    pending
        .admit("one.png", &large)
        .expect("one large image fits");
    let refusal = pending
        .admit("two.png", &large)
        .expect_err("a second large one would not fit the frame");
    assert_eq!(refusal, AttachRefusal::OverMessageBudget);
    assert_eq!(pending.chips().len(), 1);
    assert!(
        refusal.to_string().contains("7 MiB"),
        "names the budget: {refusal}"
    );
}

#[test]
fn the_notice_names_the_image_and_the_reason_on_one_line() {
    let notice = refusal_notice("notes.txt", &AttachRefusal::NotAnImage);
    assert_eq!(
        notice,
        "Image not attached: notes.txt: not an image/png, image/jpeg, image/gif or image/webp file"
    );
    assert!(!notice.contains('\n'));
}

#[test]
fn chips_number_name_and_size_each_image() {
    let mut pending = PendingImages::default();
    pending.admit("tiny.png", &samples::png(1, 1)).unwrap();
    pending
        .admit("screenshot.png", &png_of(240 * 1024))
        .unwrap();
    pending
        .admit("big.png", &png_of(3 * 1024 * 1024 / 2))
        .unwrap();
    let chips = pending.chips();
    assert_eq!(chips.len(), 3);
    assert!(chips[0].starts_with("[image 1: tiny.png · ") && chips[0].ends_with(" B]"));
    assert_eq!(chips[1], "[image 2: screenshot.png · 240 KB]");
    assert_eq!(chips[2], "[image 3: big.png · 1.5 MB]");
}

#[test]
fn a_long_name_is_cut_in_its_chip() {
    let mut pending = PendingImages::default();
    let name = format!("{}.png", "a".repeat(80));
    pending.admit(&name, &samples::png(1, 1)).unwrap();
    let chip = &pending.chips()[0];
    assert!(chip.contains('…'), "{chip}");
    assert!(chip.chars().count() < 70, "{chip}");
}

#[test]
fn remove_last_drops_the_newest_and_clear_drops_all() {
    let mut pending = PendingImages::default();
    assert!(!pending.remove_last(), "nothing to remove");
    pending.admit("a.png", &samples::png(1, 1)).unwrap();
    pending.admit("b.gif", &samples::gif(1, 1)).unwrap();
    assert!(pending.remove_last());
    assert_eq!(pending.chips().len(), 1);
    assert!(pending.chips()[0].contains("a.png"));
    pending.admit("c.png", &samples::png(1, 1)).unwrap();
    pending.clear();
    assert!(pending.is_empty());
    assert!(pending.chips().is_empty());
}

#[test]
fn markers_stand_above_the_text_one_per_image() {
    assert_eq!(with_image_markers("hello", 0), "hello");
    assert_eq!(
        with_image_markers("what is this?", 1),
        "[image]\nwhat is this?"
    );
    assert_eq!(with_image_markers("", 2), "[image] [image]");
}

#[test]
fn an_image_path_is_absolute_home_relative_or_workspace_relative() {
    let home = Path::new("/home/me");
    let ws = Path::new("/work/repo");
    let resolve = |arg| resolve_image_path(arg, Some(home), ws);
    assert_eq!(resolve("/tmp/a.png"), Ok(PathBuf::from("/tmp/a.png")));
    assert_eq!(
        resolve("~/Pictures/a.png"),
        Ok(PathBuf::from("/home/me/Pictures/a.png"))
    );
    assert_eq!(resolve("~"), Ok(PathBuf::from("/home/me")));
    assert_eq!(
        resolve("shots/a.png"),
        Ok(PathBuf::from("/work/repo/shots/a.png"))
    );
    assert_eq!(
        resolve("  shots/a.png  "),
        Ok(PathBuf::from("/work/repo/shots/a.png"))
    );
    assert_eq!(
        resolve("'/tmp/my shot.png'"),
        Ok(PathBuf::from("/tmp/my shot.png")),
        "a dropped, quoted path is unquoted"
    );
    assert_eq!(resolve(""), Err(ImagePathError::Missing));
    assert_eq!(resolve("   "), Err(ImagePathError::Missing));
    assert_eq!(
        resolve_image_path("~/a.png", None, ws),
        Err(ImagePathError::NoHome)
    );
    assert!(
        ImagePathError::Missing
            .to_string()
            .starts_with("Usage: /image <path>")
    );
}

#[test]
fn labels_name_the_file_or_the_clipboard_type() {
    assert_eq!(file_label(Path::new("/tmp/shots/a.png")), "a.png");
    assert_eq!(clipboard_label(&samples::png(1, 1)), "clipboard.png");
    assert_eq!(clipboard_label(&samples::jpeg(1, 1)), "clipboard.jpeg");
    assert_eq!(clipboard_label(b"text"), "clipboard");
}
