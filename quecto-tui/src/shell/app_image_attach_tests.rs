//! #2425: images attached to a message — `/image <path>`, `Ctrl+V`, the
//! chips above the composer, the keys that edit them, the send payload, the
//! refusal notices and the transcript's `[image]` markers — driven through
//! the headless render harness. The clipboard is always a fake.

use super::app_response::ATTACH_BACKFILL_ID;
use super::tui_harness::TuiHarness;
use super::*;
use crate::shell::clipboard_image::ClipboardRead;
use quecto_image::samples;

pub(super) async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

/// Write `bytes` to `name` in a fresh directory; the directory lives as long
/// as the returned guard.
pub(super) fn image_file(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

/// `/image <path>`, then deliver the file read the way the event loop does.
pub(super) async fn attach(h: &mut TuiHarness, path: &std::path::Path) {
    h.submit(&format!("/image {}", path.display()));
    h.settle_attachment_reads().await;
}

/// The commands of `type` among `lines`, parsed.
pub(super) fn sent(lines: &[String], kind: &str) -> Vec<serde_json::Value> {
    lines
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|cmd| cmd["type"] == kind)
        .collect()
}

#[tokio::test]
async fn slash_image_attaches_a_file_as_a_chip_above_the_composer() {
    let mut h = harness().await;
    let (_dir, path) = image_file("screenshot.png", &samples::png(4, 4));
    attach(&mut h, &path).await;

    assert_eq!(h.attachment_chips().len(), 1);
    let bottom = h.last();
    let chip_row = bottom
        .lines()
        .position(|line| line.contains("[image 1: screenshot.png · "))
        .unwrap_or_else(|| panic!("the chip renders below the chat:\n{bottom}"));
    let editor_row = bottom
        .lines()
        .position(|line| line.contains('─'))
        .expect("the editor's border renders");
    assert!(
        chip_row < editor_row,
        "the chip sits above the composer:\n{bottom}"
    );
    assert_eq!(h.editor_text(), "", "the command left the editor");
    assert!(
        sent(&h.try_drain_commands(), "prompt").is_empty(),
        "attaching sends nothing"
    );
}

#[tokio::test]
async fn slash_image_resolves_a_workspace_relative_path() {
    let mut h = harness().await;
    let (dir, _) = image_file("diagram.gif", &samples::gif(2, 2));
    h.set_workspace_root(dir.path().to_path_buf());
    h.submit("/image diagram.gif");
    h.settle_attachment_reads().await;
    assert!(
        h.attachment_chips()[0].starts_with("[image 1: diagram.gif · "),
        "{:?}",
        h.attachment_chips()
    );
}

#[tokio::test]
async fn enter_sends_the_images_with_the_prompt_and_clears_the_chips() {
    let mut h = harness().await;
    let png = samples::png(4, 4);
    let (_dir, path) = image_file("shot.png", &png);
    attach(&mut h, &path).await;
    let (_dir2, path2) = image_file("photo.jpg", &samples::jpeg(2, 2));
    attach(&mut h, &path2).await;
    let _ = h.drain_commands().await;

    for ch in "what is this?".chars() {
        h.type_char(ch);
    }
    h.press(Key::Enter);

    let prompts = sent(&h.drain_commands().await, "prompt");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    let prompt = &prompts[0];
    assert_eq!(prompt["message"], "what is this?");
    assert_eq!(prompt["images"][0]["mimeType"], "image/png");
    assert_eq!(prompt["images"][0]["data"], quecto_image::encode(&png));
    assert_eq!(prompt["images"][1]["mimeType"], "image/jpeg");
    assert_eq!(prompt["images"].as_array().map(Vec::len), Some(2));

    assert!(h.attachment_chips().is_empty(), "sending clears the chips");
    assert!(!h.last().contains("[image 1:"), "{}", h.last());
    assert_eq!(
        h.active_user_entries().last().map(String::as_str),
        Some("[image] [image]\nwhat is this?"),
        "the transcript marks the images"
    );
}

#[tokio::test]
async fn enter_with_only_chips_sends_an_images_only_prompt() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(1, 1));
    attach(&mut h, &path).await;
    let _ = h.drain_commands().await;

    h.press(Key::Enter);

    let prompts = sent(&h.drain_commands().await, "prompt");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0]["message"], "");
    assert_eq!(prompts[0]["images"].as_array().map(Vec::len), Some(1));
    assert!(h.attachment_chips().is_empty());
    assert_eq!(
        h.active_user_entries().last().map(String::as_str),
        Some("[image]")
    );
}

#[tokio::test]
async fn a_text_only_prompt_carries_no_images_field() {
    let mut h = harness().await;
    let _ = h.drain_commands().await;
    h.submit("plain words");
    let prompts = sent(&h.drain_commands().await, "prompt");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].get("images").is_none(), "{}", prompts[0]);
}

#[tokio::test]
async fn while_the_agent_runs_the_images_ride_the_follow_up() {
    let mut h = harness().await;
    h.event(Event::AgentStart);
    let (_dir, path) = image_file("shot.webp", &samples::webp_lossless(2, 2));
    attach(&mut h, &path).await;
    let _ = h.drain_commands().await;

    h.submit("and this one");

    let follow_ups = sent(&h.drain_commands().await, "follow_up");
    assert_eq!(follow_ups.len(), 1, "{follow_ups:?}");
    assert_eq!(follow_ups[0]["message"], "and this one");
    assert_eq!(follow_ups[0]["images"][0]["mimeType"], "image/webp");
    assert!(h.attachment_chips().is_empty());
}

#[tokio::test]
async fn backspace_in_an_empty_composer_removes_the_last_chip() {
    let mut h = harness().await;
    let (_dir, a) = image_file("first.png", &samples::png(1, 1));
    let (_dir2, b) = image_file("second.png", &samples::png(2, 2));
    attach(&mut h, &a).await;
    attach(&mut h, &b).await;

    // With text in the editor, Backspace edits the text.
    h.type_char('x');
    h.press(Key::Backspace);
    assert_eq!(h.editor_text(), "");
    assert_eq!(h.attachment_chips().len(), 2);
    // A fresh press, not the key repeat of the one that emptied the editor.
    h.advance_clock(std::time::Duration::from_secs(1));

    h.press(Key::Backspace);
    let chips = h.attachment_chips();
    assert_eq!(chips.len(), 1);
    assert!(chips[0].contains("first.png"), "{chips:?}");
    assert!(!h.last().contains("second.png"), "{}", h.last());

    // Another fresh press (review round 2: a held one removes one chip).
    h.advance_clock(std::time::Duration::from_secs(1));
    h.press(Key::Backspace);
    assert!(h.attachment_chips().is_empty());
    assert!(!h.last().contains("[image"), "{}", h.last());
}

#[tokio::test]
async fn esc_clears_every_chip_and_the_text_when_idle() {
    let mut h = harness().await;
    let (_dir, a) = image_file("first.png", &samples::png(1, 1));
    attach(&mut h, &a).await;
    attach(&mut h, &a).await;
    h.type_char('x');
    assert_eq!(h.attachment_chips().len(), 2);

    h.press(Key::Escape);

    assert!(h.attachment_chips().is_empty());
    assert_eq!(h.editor_text(), "");
    assert!(!h.last().contains("[image"), "{}", h.last());
    assert!(
        h.app_mut().ac().rewind.selector.is_none(),
        "clearing the chips is not the rewind double-Esc"
    );
}

#[tokio::test]
async fn esc_while_running_still_aborts_and_keeps_the_chips() {
    let mut h = harness().await;
    h.event(Event::AgentStart);
    let (_dir, a) = image_file("first.png", &samples::png(1, 1));
    attach(&mut h, &a).await;

    h.press(Key::Escape);

    assert_eq!(h.pending_aborts(), 1, "Esc aborts the run, as before");
    assert_eq!(h.attachment_chips().len(), 1, "the chips stay");
}

#[tokio::test]
async fn ctrl_c_clears_the_chips_with_the_editor() {
    let mut h = harness().await;
    h.event(Event::AgentStart);
    let (_dir, a) = image_file("first.png", &samples::png(1, 1));
    attach(&mut h, &a).await;

    h.press(Key::Ctrl('c'));

    assert!(
        h.attachment_chips().is_empty(),
        "the composer is cleared first"
    );
    assert_eq!(h.pending_aborts(), 0, "the run is not aborted yet");
}

#[tokio::test]
async fn refusals_show_a_one_line_notice_and_attach_nothing() {
    let mut h = harness().await;

    let (_dir, text) = image_file("notes.txt", b"just words");
    attach(&mut h, &text).await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some(
            "Image not attached: notes.txt: not an image/png, image/jpeg, image/gif or image/webp file"
        )
    );

    let big = samples::png_with_body(4, 4, quecto_image::MAX_IMAGE_BYTES);
    let (_dir2, big) = image_file("huge.png", &big);
    attach(&mut h, &big).await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some("Image not attached: huge.png: image decodes to more than 3932160 bytes (3.75 MiB)")
    );

    let missing = std::path::Path::new("/nonexistent-2425/gone.png");
    attach(&mut h, missing).await;
    let notice = h.last_notification().unwrap_or_default();
    assert!(
        notice.starts_with("Image not attached: gone.png: "),
        "{notice}"
    );

    h.submit("/image");
    let notice = h.last_notification().unwrap_or_default();
    assert!(notice.starts_with("Usage: /image <path>"), "{notice}");

    assert!(h.attachment_chips().is_empty());
    let frame = h.full_frame();
    assert!(frame.contains("Image not attached: huge.png"), "{frame}");
    assert!(sent(&h.try_drain_commands(), "prompt").is_empty());
}

#[tokio::test]
async fn a_directory_is_not_an_image_file() {
    let mut h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    attach(&mut h, dir.path()).await;
    let notice = h.last_notification().unwrap_or_default();
    assert!(notice.starts_with("Image not attached: "), "{notice}");
    assert!(h.attachment_chips().is_empty());
}

#[tokio::test]
async fn ctrl_v_attaches_a_clipboard_image() {
    let mut h = harness().await;
    h.set_clipboard(ClipboardRead::Image(samples::png(8, 8)));
    h.paste_clipboard().await;
    let chips = h.attachment_chips();
    assert_eq!(chips.len(), 1, "{chips:?}");
    assert!(
        chips[0].starts_with("[image 1: clipboard.png · "),
        "{chips:?}"
    );
    assert!(h.last().contains("[image 1: clipboard.png"), "{}", h.last());
}

#[tokio::test]
async fn ctrl_v_with_text_on_the_clipboard_pastes_the_text() {
    let mut h = harness().await;
    h.set_clipboard(ClipboardRead::Text("pasted words".into()));
    h.paste_clipboard().await;
    assert_eq!(h.editor_text(), "pasted words");
    assert!(h.attachment_chips().is_empty());
}

#[tokio::test]
async fn ctrl_v_refusals_and_failures_are_one_line_notices() {
    let mut h = harness().await;

    h.set_clipboard(ClipboardRead::Failed("wl-paste timed out after 2 s".into()));
    h.paste_clipboard().await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some("Clipboard read failed: wl-paste timed out after 2 s")
    );

    h.set_clipboard(ClipboardRead::Image(b"GIF89a-but-truncated".to_vec()));
    h.paste_clipboard().await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some("Image not attached: clipboard.gif: not a readable image/gif image")
    );

    h.set_clipboard(ClipboardRead::UnsupportedImage("image/bmp".into()));
    h.paste_clipboard().await;
    assert_eq!(
        h.last_notification().as_deref(),
        Some(
            "Image not attached: clipboard (image/bmp): not an image/png, image/jpeg, image/gif or image/webp file"
        ),
        "one wording for the admitted types"
    );

    h.set_clipboard(ClipboardRead::NoTool);
    h.paste_clipboard().await;
    let notice = h.last_notification().unwrap_or_default();
    assert!(
        notice.contains("wl-paste") && notice.contains("xclip"),
        "{notice}"
    );

    assert!(h.attachment_chips().is_empty());
    assert_eq!(h.editor_text(), "");
}

#[tokio::test]
async fn history_shows_a_user_message_s_images_as_markers() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.test_arm_attach_backfill(ATTACH_BACKFILL_ID);
    a.handle_response(
        Some(ATTACH_BACKFILL_ID.to_string()),
        "get_messages".to_string(),
        true,
        Some(serde_json::json!({
            "messages": [
                {"role": "user", "content": "compare these", "id": "u1",
                 "imageCount": 2, "imageMimeTypes": ["image/png", "image/jpeg"]},
                {"role": "assistant", "content": "they differ", "id": "a1"},
                {"role": "user", "content": "", "id": "u2",
                 "imageCount": 1, "imageMimeTypes": ["image/png"]},
            ],
            "hasMoreBefore": false
        })),
        None,
    );
    assert_eq!(
        h.active_user_entries(),
        ["[image] [image]\ncompare these", "[image]"]
    );
    let frame = h.full_frame();
    assert!(frame.contains("> [image] [image]"), "{frame}");
}

#[tokio::test]
async fn help_names_the_image_command_and_keys() {
    let mut h = harness().await;
    let help = h.show_help_frame();
    assert!(help.contains("/image"), "{help}");
    assert!(help.contains("Ctrl+V"), "{help}");
    assert!(help.contains("last image"), "{help}");
    assert!(
        help.contains("images stay while a sub-agent is focused"),
        "{help}"
    );
}

#[tokio::test]
async fn a_focused_sub_agent_receives_the_images_on_its_own_connection() {
    use super::tui_harness::{
        drain_child_commands_until_quiet, spawn_subagent_socket_with_commands,
        subagent_with_socket, subagents_changed,
    };
    let mut h = harness().await;
    let (socket, mut child_commands) = spawn_subagent_socket_with_commands("worker");
    h.event(subagents_changed(vec![subagent_with_socket(
        "worker",
        "idle",
        None,
        Some(socket),
    )]));
    h.select(Some("worker"));
    let _ = drain_child_commands_until_quiet(&mut child_commands).await;
    let (_dir, path) = image_file("shot.png", &samples::png(2, 2));
    attach(&mut h, &path).await;
    let _ = h.drain_commands().await;

    h.submit("for the worker");

    let child = drain_child_commands_until_quiet(&mut child_commands).await;
    let prompts = sent(&child, "prompt");
    assert_eq!(prompts.len(), 1, "{child:?}");
    assert_eq!(prompts[0]["message"], "for the worker");
    assert_eq!(prompts[0]["images"][0]["mimeType"], "image/png");
    assert!(
        sent(&h.drain_commands().await, "prompt").is_empty(),
        "nothing goes to the master"
    );
    assert!(h.attachment_chips().is_empty());
    assert_eq!(
        h.active_user_entries().last().map(String::as_str),
        Some("[image]\nfor the worker")
    );
}

#[tokio::test]
async fn a_message_refused_for_a_lost_connection_keeps_its_chips() {
    let mut h = harness().await;
    let (_dir, path) = image_file("shot.png", &samples::png(2, 2));
    attach(&mut h, &path).await;
    h.app_mut().ac_mut().agent_connected = false;

    h.submit("are you there?");

    assert_eq!(
        h.attachment_chips().len(),
        1,
        "the images wait for a resend"
    );
}
