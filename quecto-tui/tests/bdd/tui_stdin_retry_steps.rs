//! Steps for `tui_stdin_retry.feature`.
//!
//! Multi-fragment CSI escape reassembly is driven against REAL production code:
//!  * `quecto_tui::interface::stdin_buffer::StdinBuffer` for feed/drain/force-drain
//!    reassembly (fragment-by-fragment, exactly as the event loop feeds it), and
//!  * the real app retry loop `process_stdin_bytes` (with `MAX_ESCAPE_RETRIES`
//!    and the 10ms escape timeout) via the headless harness for the retry cap.
//!
//! Emitted key identity is confirmed through the real `keys::parse_key`.

use super::*;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::interface::keys::{Key, parse_key};
use quecto_tui::interface::stdin_buffer::StdinBuffer;

// ── Background ────────────────────────────────────────────────────────────

#[given("the TUI uses a StdinBuffer with retry-on-pending logic")]
fn given_uses_buffer(world: &mut TuiWorld) {
    world.tui_stdin_fragments.clear();
    world.tui_stdin_emitted.clear();
    world.tui_stdin_pending_after = false;
    world.tui_stdin_force_drained = false;
    world.tui_stdin_leftover = None;
}

#[given("the escape timeout is 10ms per retry")]
fn given_escape_timeout(world: &mut TuiWorld) {
    // The real retry loop uses a 10ms `escape_timeout` (see the cap scenario,
    // which drives `process_stdin_bytes` with that exact value).
    world.tui_stdin_leftover = None;
}

// ── Fragment arrival ──────────────────────────────────────────────────────

#[given("fragment 1 arrives: ESC (0x1b)")]
#[given("only ESC (0x1b) arrives")]
fn given_esc_fragment(world: &mut TuiWorld) {
    world.tui_stdin_fragments.push(vec![0x1b]);
}

#[given(regex = r#"^\d+ms later fragment \d+ arrives: "(.+)"$"#)]
fn given_later_fragment(world: &mut TuiWorld, payload: String) {
    world.tui_stdin_fragments.push(payload.into_bytes());
}

#[given("no more data arrives within the retry window")]
fn given_no_more_data(world: &mut TuiWorld) {
    // No further fragment queued — the retry window will expire in the When.
    assert!(
        !world.tui_stdin_fragments.is_empty(),
        "a bare ESC fragment should already be queued"
    );
}

#[given("an incomplete CSI sequence that never completes")]
fn given_incomplete_csi(world: &mut TuiWorld) {
    // ESC[ with only parameter bytes to follow — no final byte (0x40..=0x7E),
    // so the sequence can never complete and the loop must hit its cap.
    world.tui_stdin_fragments = vec![b"\x1b[".to_vec()];
}

#[given("a complete CSI sequence ESC[A arrives in one read")]
fn given_complete_csi(world: &mut TuiWorld) {
    world.tui_stdin_fragments = vec![b"\x1b[A".to_vec()];
}

// ── When: real StdinBuffer / retry loop ───────────────────────────────────

#[when("the retry loop processes pending data")]
fn when_process_pending(world: &mut TuiWorld) {
    // Feed fragments one at a time and drain complete sequences between reads,
    // exactly as the event loop does — no force-drain on a clean reassembly.
    let mut buf = StdinBuffer::new();
    let mut emitted = Vec::new();
    for frag in &world.tui_stdin_fragments {
        buf.feed(frag);
        emitted.extend(buf.drain_complete());
    }
    world.tui_stdin_pending_after = buf.has_pending();
    world.tui_stdin_force_drained = false;
    world.tui_stdin_emitted = emitted;
}

#[when("the retry loop exhausts all attempts")]
fn when_exhaust(world: &mut TuiWorld) {
    let mut buf = StdinBuffer::new();
    let mut emitted = Vec::new();
    for frag in &world.tui_stdin_fragments {
        buf.feed(frag);
        emitted.extend(buf.drain_complete());
    }
    // Retry window elapsed with data still pending — force-drain it.
    let forced = buf.drain_all();
    world.tui_stdin_force_drained = !forced.is_empty();
    emitted.extend(forced);
    world.tui_stdin_pending_after = buf.has_pending();
    world.tui_stdin_emitted = emitted;
}

#[when("the buffer processes the data")]
fn when_process_complete(world: &mut TuiWorld) {
    let mut buf = StdinBuffer::new();
    let mut emitted = Vec::new();
    for frag in &world.tui_stdin_fragments {
        buf.feed(frag);
        emitted.extend(buf.drain_complete());
    }
    world.tui_stdin_pending_after = buf.has_pending();
    world.tui_stdin_force_drained = false;
    world.tui_stdin_emitted = emitted;
}

#[when("the retry loop runs")]
fn when_retry_loop_runs(world: &mut TuiWorld) {
    // Drive the REAL app retry loop. Queue more never-completing follow-up
    // fragments than the cap allows, so the leftover proves the loop stopped.
    let first = world.tui_stdin_fragments[0].clone();
    let followups: Vec<&[u8]> = vec![b"1"; 8];
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let (leftover, pending) = rt.block_on(async {
        let mut h = TuiHarness::new().await;
        let leftover = h.drive_stdin_retry_loop(&first, &followups).await;
        (leftover, h.stdin_has_pending())
    });
    world.tui_stdin_leftover = Some(leftover);
    world.tui_stdin_pending_after = pending;
}

// ── Then ──────────────────────────────────────────────────────────────────

#[then("the emitted sequence should be ESC[A (Up arrow)")]
fn then_emitted_up_arrow(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_emitted,
        vec![b"\x1b[A".to_vec()],
        "fragments should reassemble into a single ESC[A sequence"
    );
    let (key, _) = parse_key(&world.tui_stdin_emitted[0]).expect("ESC[A should parse");
    assert_eq!(key, Key::Up, "ESC[A must decode to the Up arrow");
}

#[then("no bytes should be force-drained as individual bytes")]
fn then_no_force_drain(world: &mut TuiWorld) {
    assert!(
        !world.tui_stdin_force_drained,
        "a clean reassembly must not force-drain individual bytes"
    );
}

#[then(r#"the ESC and "[" should NOT be emitted as separate bytes"#)]
fn then_not_separate_bytes(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_emitted.len(),
        1,
        "the reassembly must emit exactly one sequence, not per-byte fragments"
    );
    assert!(
        !world.tui_stdin_emitted.contains(&vec![0x1b])
            && !world.tui_stdin_emitted.contains(&vec![b'[']),
        "ESC and '[' must not appear as standalone emitted bytes"
    );
}

#[then("ESC should be emitted as a bare Escape key")]
fn then_bare_escape(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_emitted,
        vec![vec![0x1b]],
        "an unterminated ESC should force-drain as a lone Escape byte"
    );
    let (key, _) = parse_key(&world.tui_stdin_emitted[0]).expect("ESC should parse");
    assert_eq!(key, Key::Escape, "a lone 0x1b must decode to Escape");
}

#[then("the buffer should be empty")]
fn then_buffer_empty(world: &mut TuiWorld) {
    assert!(
        !world.tui_stdin_pending_after,
        "the buffer must hold no pending bytes after force-drain"
    );
}

#[then("it should stop after at most 5 retry iterations")]
fn then_cap_iterations(world: &mut TuiWorld) {
    // 8 follow-up fragments queued; a cap of 5 leaves exactly 3 unconsumed.
    let leftover = world.tui_stdin_leftover.expect("retry loop leftover");
    assert_eq!(
        leftover, 3,
        "the loop must consume at most 5 fragments (8 queued − 5 = 3 leftover)"
    );
}

#[then("force-drain the incomplete bytes")]
fn then_force_drain_incomplete(world: &mut TuiWorld) {
    assert!(
        !world.tui_stdin_pending_after,
        "after the cap is hit the incomplete bytes must be force-drained"
    );
}

#[then("the sequence should be emitted immediately")]
fn then_emitted_immediately(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_emitted,
        vec![b"\x1b[A".to_vec()],
        "a complete sequence in one read should be emitted directly"
    );
}

#[then("no retry loop should be entered")]
fn then_no_retry(world: &mut TuiWorld) {
    assert!(
        !world.tui_stdin_pending_after,
        "a complete read leaves nothing pending, so no retry loop is needed"
    );
}
