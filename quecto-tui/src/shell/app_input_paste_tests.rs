//! Full-path regressions for raw and bracketed multiline paste input (#1176).
//!
//! Each test drives the production decoder, pending-state loop, key routing,
//! editor, submit dispatcher, and UDS writer. Pasted newlines must remain editor
//! content until a separately delivered Enter explicitly submits.

use super::tui_harness::TuiHarness;
use super::*;
use tokio::sync::mpsc;

async fn drive_stdin_chunks(h: &mut TuiHarness, chunks: &[Vec<u8>]) {
    assert!(!chunks.is_empty());
    let (tx, mut rx) = mpsc::channel(chunks.len().max(1));
    for chunk in &chunks[1..] {
        tx.send(chunk.clone()).await.expect("queue stdin chunk");
    }
    // Keep the channel open so the production decoder observes a quiet burst
    // boundary, not synthetic stdin EOF.
    let mut kitty_fallback_done = true;
    h.app_mut()
        .process_stdin_bytes(
            chunks[0].clone(),
            &mut rx,
            Duration::from_millis(10),
            &mut kitty_fallback_done,
        )
        .await;
    drop(tx);
}

async fn assert_draft_without_model_action(h: &mut TuiHarness, chunks: &[Vec<u8>], expected: &str) {
    drive_stdin_chunks(h, chunks).await;
    assert_eq!(
        h.editor_text(),
        expected,
        "paste must remain one editor draft"
    );
    let commands = h.drain_commands().await;
    assert!(
        !commands.iter().any(|command| {
            command.contains("\"type\":\"prompt\"")
                || command.contains("\"type\":\"follow_up\"")
                || command.contains("\"type\":\"steer\"")
                || command.contains("\"streamingBehavior\":\"steer\"")
        }),
        "paste emitted a model action before explicit submit: {commands:?}"
    );
}

async fn explicitly_submit(h: &mut TuiHarness, expected: &str) {
    drive_stdin_chunks(h, &[b"\r".to_vec()]).await;
    let commands = h.drain_commands().await;
    let prompts: Vec<_> = commands
        .iter()
        .filter(|command| command.contains("\"type\":\"prompt\""))
        .collect();
    assert_eq!(prompts.len(), 1, "explicit Enter must submit exactly once");
    assert!(
        prompts[0].contains(&serde_json::to_string(expected).unwrap()),
        "submitted prompt must contain the complete paste: {prompts:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn raw_multiline_paste_with_trailing_newline_waits_for_explicit_submit() {
    let mut h = TuiHarness::new().await;
    let text = "alpha\nbeta\n";
    assert_draft_without_model_action(&mut h, &[text.as_bytes().to_vec()], text).await;
    explicitly_submit(&mut h, text).await;
}

#[tokio::test(start_paused = true)]
async fn final_newline_ended_chunk_never_becomes_enter() {
    let mut h = TuiHarness::new().await;
    let chunks = vec![b"alpha\n".to_vec(), b"beta\n".to_vec()];
    assert_draft_without_model_action(&mut h, &chunks, "alpha\nbeta\n").await;
}

#[tokio::test(start_paused = true)]
async fn raw_paste_lifetime_is_not_limited_to_five_fragments() {
    let mut h = TuiHarness::new().await;
    let chunks: Vec<Vec<u8>> = (0..9)
        .map(|line| format!("line {line}\n").into_bytes())
        .collect();
    let expected = String::from_utf8(chunks.concat()).unwrap();
    assert_draft_without_model_action(&mut h, &chunks, &expected).await;
}

#[tokio::test(start_paused = true)]
async fn confirmed_raw_paste_keeps_short_and_newline_only_fragments_as_text() {
    let mut h = TuiHarness::new().await;
    let chunks = vec![
        b"alpha\n".to_vec(),
        b"beta\n".to_vec(),
        b"\n".to_vec(),
        b"x\n".to_vec(),
        "é\n".as_bytes().to_vec(),
        b"z".to_vec(),
    ];
    assert_draft_without_model_action(&mut h, &chunks, "alpha\nbeta\n\nx\né\nz").await;
}

#[tokio::test(start_paused = true)]
async fn every_read_size_from_one_to_256_preserves_newlines_and_utf8() {
    let text = format!("αlpha\n{}\n雪-tail\n", "middle🙂".repeat(40));
    for read_size in 1..=256 {
        let mut h = TuiHarness::new().await;
        let chunks: Vec<Vec<u8>> = text
            .as_bytes()
            .chunks(read_size)
            .map(<[u8]>::to_vec)
            .collect();
        assert_draft_without_model_action(&mut h, &chunks, &text).await;
    }
}

#[tokio::test(start_paused = true)]
async fn large_bracketed_paste_waits_for_end_marker_beyond_six_reads() {
    let mut h = TuiHarness::new().await;
    let text = "bracketed line🙂\n".repeat(180);
    let framed = format!("\x1b[200~{text}\x1b[201~");
    let chunks: Vec<Vec<u8>> = framed.as_bytes().chunks(256).map(<[u8]>::to_vec).collect();
    assert!(chunks.len() > 6, "fixture must cross the old retry cap");
    assert_draft_without_model_action(&mut h, &chunks, &text).await;
}

#[tokio::test(start_paused = true)]
async fn slash_command_looking_paste_does_not_execute() {
    let mut h = TuiHarness::new().await;
    let text = "/quit\nstill drafting\n";
    assert_draft_without_model_action(&mut h, &[text.as_bytes().to_vec()], text).await;
    assert!(!h.should_exit(), "pasted /quit text must not execute");
}

#[tokio::test(start_paused = true)]
async fn paste_while_agent_runs_does_not_follow_up_before_explicit_submit() {
    let mut h = TuiHarness::new().await;
    h.app_mut().active_conn_mut().agent_state.start();
    let text = "alpha\nbeta\n";
    assert_draft_without_model_action(&mut h, &[text.as_bytes().to_vec()], text).await;

    drive_stdin_chunks(&mut h, &[b"\r".to_vec()]).await;
    let commands = h.drain_commands().await;
    let follow_ups: Vec<_> = commands
        .iter()
        .filter(|command| command.contains("\"type\":\"follow_up\""))
        .collect();
    assert_eq!(
        follow_ups.len(),
        1,
        "only explicit Enter may queue a follow-up"
    );
    assert!(follow_ups[0].contains("alpha\\nbeta\\n"));
}
