//! #2425 review round 1: image attachments end to end through the headless
//! render harness — the frame cap at submit, recalled stubs and rewinds that
//! had images, chips bound to their conversation, the clipboard read's
//! in-flight latch, key repeat on Backspace, the `/image` read off the event
//! loop and the refusals the first round left untested.

use super::app_image_attach_tests::{attach, harness, image_file, sent};
use super::app_response::ATTACH_BACKFILL_ID;
use super::*;
use crate::shell::clipboard_image::{ClipboardRead, ClipboardReader};
use quecto_image::samples;
use std::time::Duration;

/// A PNG whose base64 is close to the 5 MiB limit.
fn large_png() -> Vec<u8> {
    samples::png_with_body(4, 4, quecto_image::MAX_IMAGE_BYTES - 100)
}

#[tokio::test]
async fn a_message_over_the_frame_cap_is_refused_and_keeps_its_chips_and_text() {
    let mut h = harness().await;
    let (_dir, path) = image_file("big.png", &large_png());
    attach(&mut h, &path).await;
    let _ = h.drain_commands().await;
    // About 4.9 MiB of image base64 plus 3.5 MiB of text: over 8 MiB.
    let text = "a".repeat(3 * 1024 * 1024 + 512 * 1024);

    h.submit(&text);

    let notice = h.last_notification().unwrap_or_default();
    assert!(notice.starts_with("Message not sent: "), "{notice}");
    assert!(notice.contains("8 MiB"), "{notice}");
    assert_eq!(h.attachment_chips().len(), 1, "the chips stay");
    assert_eq!(
        h.editor_text().len(),
        text.len(),
        "the text is back in the editor"
    );
    assert!(
        h.active_user_entries().is_empty(),
        "nothing joins the transcript"
    );
    assert!(sent(&h.drain_commands().await, "prompt").is_empty());
}

#[tokio::test]
async fn a_ninth_image_is_refused_with_a_notice() {
    let mut h = harness().await;
    let (_dir, path) = image_file("one.png", &samples::png(1, 1));
    for _ in 0..quecto_image::MAX_IMAGES_PER_MESSAGE {
        attach(&mut h, &path).await;
    }
    assert_eq!(h.attachment_chips().len(), 8);
    attach(&mut h, &path).await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some("Image not attached: one.png: at most 8 images per message")
    );
    assert_eq!(h.attachment_chips().len(), 8);
}

#[tokio::test]
async fn an_image_past_the_message_budget_is_refused_with_a_notice() {
    let mut h = harness().await;
    let (_dir, path) = image_file("big.png", &large_png());
    attach(&mut h, &path).await;
    attach(&mut h, &path).await;
    let notice = h.last_notification().unwrap_or_default();
    assert!(
        notice.starts_with("Image not attached: big.png: "),
        "{notice}"
    );
    assert!(notice.contains("7 MiB"), "{notice}");
    assert_eq!(h.attachment_chips().len(), 1);
}

#[tokio::test]
async fn a_running_sub_agent_receives_the_images_on_a_follow_up() {
    use super::tui_harness::{
        drain_child_commands_until_quiet, spawn_subagent_socket_with_commands,
        subagent_with_socket, subagents_changed,
    };
    let mut h = harness().await;
    let (socket, mut child_commands) = spawn_subagent_socket_with_commands("worker");
    h.event(subagents_changed(vec![subagent_with_socket(
        "worker",
        "running",
        None,
        Some(socket),
    )]));
    h.select(Some("worker"));
    let _ = drain_child_commands_until_quiet(&mut child_commands).await;
    let (_dir, path) = image_file("shot.gif", &samples::gif(2, 2));
    attach(&mut h, &path).await;

    h.submit("one more");

    let child = drain_child_commands_until_quiet(&mut child_commands).await;
    let follow_ups = sent(&child, "follow_up");
    assert_eq!(follow_ups.len(), 1, "{child:?}");
    assert_eq!(follow_ups[0]["images"][0]["mimeType"], "image/gif");
    assert!(h.attachment_chips().is_empty());
}

#[tokio::test]
async fn an_unreachable_sub_agent_keeps_the_chips() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(1, 1));
    attach(&mut h, &path).await;
    h.focus_dead_subagent("gone");

    h.submit("hello?");

    assert_eq!(
        h.attachment_chips().len(),
        1,
        "the images wait for a resend"
    );
}

#[tokio::test]
async fn a_recalled_stub_keeps_its_image_markers() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.test_arm_attach_backfill(ATTACH_BACKFILL_ID);
    a.handle_response(
        Some(ATTACH_BACKFILL_ID.to_string()),
        "get_messages".to_string(),
        true,
        Some(serde_json::json!({
            "messages": [{
                "id": "stub-1", "role": "user", "collapsed": true,
                "content": "[user stub — recall available]", "imageCount": 2,
            }],
            "hasMoreBefore": false,
            "before": null,
        })),
        None,
    );
    let _ = h.drain_commands().await;
    super::app_paged_history_tests::prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = h.drain_commands().await;
    let request = sent(&commands, "get_message");
    let req_id = request[0]["id"].as_str().expect("a recall id").to_string();

    h.app_mut().handle_response(
        Some(req_id),
        "get_message".to_string(),
        true,
        Some(serde_json::json!({
            "id": "stub-1", "role": "user", "content": "the full question",
            "imageCount": 2, "imageMimeTypes": ["image/png", "image/png"],
        })),
        None,
    );

    assert_eq!(
        h.active_user_entries(),
        ["[image] [image]\nthe full question"]
    );
}

#[tokio::test]
async fn rewinding_to_a_message_with_images_says_they_were_not_restored() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.ac_mut().rewind.pending_load_id = Some("load".into());
    a.ac_mut().rewind.pending_apply_message_id = Some("u1".into());
    a.handle_response(
        Some("load".into()),
        "get_message".into(),
        true,
        Some(serde_json::json!({
            "id": "u1", "role": "user", "content": "look at these",
            "contentLength": 13, "hasMoreContent": false, "offset": 0,
            "imageCount": 2,
        })),
        None,
    );
    let apply = a
        .ac()
        .rewind
        .pending_apply_id
        .clone()
        .expect("rewind_to sent");
    a.handle_response(Some(apply), "rewind_to".into(), true, None, None);

    assert_eq!(h.editor_text(), "look at these");
    assert_eq!(
        h.last_notification().as_deref(),
        Some("2 images not restored: attach them again to send them")
    );
}

#[tokio::test]
async fn chips_do_not_outlive_their_conversation() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(1, 1));

    attach(&mut h, &path).await;
    h.submit("/clear");
    assert!(h.attachment_chips().is_empty(), "/clear drops the chips");
    assert_eq!(
        h.last_notification().as_deref(),
        Some("1 image not sent: the conversation changed")
    );

    attach(&mut h, &path).await;
    attach(&mut h, &path).await;
    h.submit("/new");
    assert!(h.attachment_chips().is_empty(), "/new drops the chips");
    assert_eq!(
        h.last_notification().as_deref(),
        Some("2 images not sent: the conversation changed")
    );

    attach(&mut h, &path).await;
    let a = h.app_mut();
    a.test_arm_resume_session("resume-1");
    a.handle_response(
        Some("resume-1".into()),
        "resume_session".into(),
        true,
        None,
        None,
    );
    assert!(h.attachment_chips().is_empty(), "a resume drops the chips");
}

#[tokio::test]
async fn setup_never_takes_the_chips() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(1, 1));
    attach(&mut h, &path).await;
    let _ = h.drain_commands().await;

    h.submit("/setup");

    let prompts = sent(&h.drain_commands().await, "prompt");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].get("images").is_none(), "{}", prompts[0]);
    assert_eq!(
        h.attachment_chips().len(),
        1,
        "the chips wait for a message"
    );
}

/// A reader whose read panics.
struct PanickingClipboard;

impl ClipboardReader for PanickingClipboard {
    fn read(&self) -> ClipboardRead {
        panic!("clipboard reader broke");
    }
}

#[tokio::test]
async fn a_panicking_clipboard_read_does_not_disable_ctrl_v() {
    let mut h = harness().await;
    h.app_mut()
        .set_clipboard(std::sync::Arc::new(PanickingClipboard));
    h.paste_clipboard().await;
    let notice = h.last_notification().unwrap_or_default();
    assert!(notice.starts_with("Clipboard read failed: "), "{notice}");

    h.set_clipboard(ClipboardRead::Image(samples::png(1, 1)));
    h.paste_clipboard().await;
    assert_eq!(h.attachment_chips().len(), 1, "Ctrl+V works again");
}

#[tokio::test]
async fn a_second_ctrl_v_while_a_read_runs_is_ignored_with_a_notice() {
    let mut h = harness().await;
    h.set_clipboard(ClipboardRead::Image(samples::png(1, 1)));
    h.press(Key::Ctrl('v'));
    h.press(Key::Ctrl('v'));
    assert_eq!(
        h.last_notification().as_deref(),
        Some("Already reading the clipboard")
    );
    h.settle_attachment_reads().await;
    assert_eq!(h.attachment_chips().len(), 1, "one read, one image");
}

#[tokio::test]
async fn held_backspace_stops_at_the_start_of_the_text() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(1, 1));
    attach(&mut h, &path).await;
    h.type_char('a');
    h.type_char('b');

    // Key repeat: the presses come with no time between them.
    for _ in 0..4 {
        h.press(Key::Backspace);
    }
    assert_eq!(h.editor_text(), "");
    assert_eq!(h.attachment_chips().len(), 1, "the repeat kept the chip");

    h.advance_clock(Duration::from_secs(1));
    h.press(Key::Backspace);
    assert!(h.attachment_chips().is_empty(), "a fresh press removes it");
}

#[tokio::test]
async fn slash_image_reads_the_file_off_the_event_loop() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(1, 1));
    h.submit(&format!("/image {}", path.display()));
    assert!(h.attachment_reads_in_flight(), "the read runs off the loop");
    assert!(h.attachment_chips().is_empty());
    h.settle_attachment_reads().await;
    assert_eq!(h.attachment_chips().len(), 1);
}

#[tokio::test]
async fn a_clipboard_of_files_says_what_it_holds() {
    let mut h = harness().await;
    h.set_clipboard(ClipboardRead::NotPasteable("text/uri-list".into()));
    h.paste_clipboard().await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some("Nothing to paste: the clipboard holds only text/uri-list, neither an image nor text")
    );
}
