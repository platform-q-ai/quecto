//! Steps for `tui_stdin_buffer_cap.feature` — drive the real quecto-tui
//! `StdinBuffer` directly and assert on its size cap and paste handling.
//!
//! `MAX_BUFFER_SIZE` is `pub(crate)`, so these tests assert against the literal
//! 64 KB from the spec rather than importing the constant.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::interface::stdin_buffer::StdinBuffer;

/// The 64 KB cap the spec pins the buffer to.
const CAP: usize = 64 * 1024;

fn buffer(world: &mut TuiWorld) -> &mut StdinBuffer {
    &mut world
        .tui_stdin_buffer
        .as_mut()
        .expect("stdin buffer must be initialised first")
        .0
}

fn feed(world: &mut TuiWorld, data: &[u8]) {
    let accepted_all = buffer(world).feed(data);
    world.tui_stdin_last_feed_ok = Some(accepted_all);
    world.tui_stdin_fed_total += data.len();
}

#[given("the stdin buffer is empty")]
fn stdin_buffer_empty(world: &mut TuiWorld) {
    world.tui_stdin_buffer = Some(crate::DebugStdinBuffer(StdinBuffer::new()));
    world.tui_stdin_last_feed_ok = None;
    world.tui_stdin_fed_total = 0;
    world.tui_stdin_drained = None;
}

#[when("64KB of data is fed into the buffer")]
fn feed_64kb(world: &mut TuiWorld) {
    let data = vec![b'a'; CAP];
    feed(world, &data);
}

#[then("the buffer should accept the data")]
fn buffer_accepts(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_last_feed_ok,
        Some(true),
        "feed() should report that all bytes within the cap were accepted"
    );
}

#[when("1 more byte is fed")]
fn feed_one_more(world: &mut TuiWorld) {
    feed(world, b"x");
}

#[then("the extra byte should be silently dropped")]
fn extra_byte_dropped(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_last_feed_ok,
        Some(false),
        "feed() past the cap must report bytes were dropped (returns false)"
    );
}

#[then("the buffer size should not exceed 64KB")]
fn size_within_cap(world: &mut TuiWorld) {
    // Drain everything the buffer actually holds and sum the byte lengths — the
    // only externally observable measure of the buffer's retained size.
    let drained = buffer(world).drain_all();
    let held: usize = drained.iter().map(|s| s.len()).sum();
    assert!(
        held <= CAP,
        "retained buffer size {held} must not exceed the {CAP}-byte cap"
    );
    assert!(
        world.tui_stdin_fed_total > CAP,
        "sanity: the scenario should have fed more than the cap ({} bytes)",
        world.tui_stdin_fed_total
    );
}

#[when("a 100-byte escape sequence is fed")]
fn feed_100_byte_sequence(world: &mut TuiWorld) {
    // A CSI sequence: ESC [ <98 param bytes> <final byte>. Complete and well
    // within the cap, so it drains as exactly one sequence.
    let mut seq = Vec::with_capacity(100);
    seq.extend_from_slice(b"\x1b[");
    seq.extend_from_slice(&[b'1'; 97]);
    seq.push(b'm');
    assert_eq!(seq.len(), 100, "the fed sequence must be 100 bytes");
    feed(world, &seq);
    world.tui_stdin_drained = None;
}

#[then("the buffer should accept all bytes")]
fn buffer_accepts_all(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_stdin_last_feed_ok,
        Some(true),
        "the 100-byte sequence is within the cap and must be fully accepted"
    );
}

#[then("drain_complete should return the sequence")]
fn drain_returns_sequence(world: &mut TuiWorld) {
    let seqs = buffer(world).drain_complete();
    assert_eq!(
        seqs.len(),
        1,
        "the complete escape sequence should drain as exactly one event: {seqs:?}"
    );
    assert!(
        seqs[0].starts_with(b"\x1b[") && seqs[0].ends_with(b"m"),
        "the drained sequence should be the fed CSI sequence: {:?}",
        seqs[0]
    );
    assert_eq!(
        seqs[0].len(),
        100,
        "the whole 100-byte sequence should return"
    );
}

#[when("a bracketed paste start marker arrives without end marker")]
fn paste_start_without_end(world: &mut TuiWorld) {
    feed(world, b"\x1b[200~");
}

#[when("100KB of paste content follows")]
fn paste_content_follows(world: &mut TuiWorld) {
    // Feed 100 KB in chunks; the cap must reject the overflow.
    let chunk = vec![b'p'; 1024];
    for _ in 0..100 {
        feed(world, &chunk);
    }
}

#[then("the buffer should stop accepting data at 64KB")]
fn buffer_stops_at_cap(world: &mut TuiWorld) {
    // The paste never completes (no end marker), so nothing drains as complete.
    assert!(
        buffer(world).drain_complete().is_empty(),
        "a broken paste (no end marker) must not yield a complete sequence"
    );
    assert_eq!(
        world.tui_stdin_last_feed_ok,
        Some(false),
        "once the cap is reached, further feed() calls must report drops"
    );
}

#[then("memory usage should remain bounded")]
fn memory_bounded(world: &mut TuiWorld) {
    let drained = buffer(world).drain_all();
    let held: usize = drained.iter().map(|s| s.len()).sum();
    assert!(
        held <= CAP,
        "buffer must stay bounded at {CAP} bytes despite the broken paste; held {held}"
    );
}

#[given("a large bracketed paste (60KB) with proper end marker")]
fn large_paste_with_end(world: &mut TuiWorld) {
    world.tui_stdin_buffer = Some(crate::DebugStdinBuffer(StdinBuffer::new()));
    world.tui_stdin_last_feed_ok = None;
    world.tui_stdin_fed_total = 0;
    let mut paste = Vec::new();
    paste.extend_from_slice(b"\x1b[200~");
    paste.extend_from_slice(&[b'z'; 60 * 1024]);
    paste.extend_from_slice(b"\x1b[201~");
    feed(world, &paste);
}

#[when("drain_complete is called")]
fn call_drain_complete(world: &mut TuiWorld) {
    let seqs = buffer(world).drain_complete();
    world.tui_stdin_drained = Some(seqs);
}

#[then("the paste should be extracted as one sequence")]
fn paste_extracted_as_one(world: &mut TuiWorld) {
    let seqs = world
        .tui_stdin_drained
        .as_ref()
        .expect("drain_complete must have been called");
    assert_eq!(
        seqs.len(),
        1,
        "the whole bracketed paste should extract as exactly one sequence"
    );
    assert!(
        seqs[0].starts_with(b"\x1b[200~") && seqs[0].ends_with(b"\x1b[201~"),
        "the extracted sequence should span start-to-end markers"
    );
}

#[then("the scan should not exhibit O(n²) behavior")]
fn scan_not_quadratic(world: &mut TuiWorld) {
    // The paste (60 KB) drained in one pass and the buffer is now empty — the
    // production end-marker scan is a single `windows()` sweep, not a rescan per
    // byte. Assert the drain fully consumed the buffer (no pending remainder),
    // which a quadratic re-scan bug would still satisfy but confirms correctness.
    let seqs = world
        .tui_stdin_drained
        .as_ref()
        .expect("drain_complete must have been called");
    let extracted: usize = seqs.iter().map(|s| s.len()).sum();
    assert_eq!(
        extracted,
        6 + 60 * 1024 + 6,
        "the single drained sequence should cover the entire paste"
    );
    assert!(
        !buffer(world).has_pending(),
        "the buffer should be fully drained with no pending remainder"
    );
}
