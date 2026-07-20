use super::*;

#[test]
fn single_printable_char() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"a");
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"a".to_vec()]);
    assert!(!buf.has_pending());
}

#[test]
fn multiple_printable_chars() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"abc");
    let seqs = buf.drain_complete();
    assert_eq!(seqs.len(), 3);
    assert_eq!(seqs[0], b"a".to_vec());
    assert_eq!(seqs[1], b"b".to_vec());
    assert_eq!(seqs[2], b"c".to_vec());
}

#[test]
fn complete_csi_in_one_read() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b[A"); // Up arrow
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
}

#[test]
fn split_csi_across_reads() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b");
    let seqs = buf.drain_complete();
    assert!(seqs.is_empty(), "bare ESC should be held");
    assert!(buf.has_pending());

    buf.feed(b"[A");
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
    assert!(!buf.has_pending());
}

#[test]
fn split_csi_three_reads() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b");
    assert!(buf.drain_complete().is_empty());
    buf.feed(b"[");
    assert!(buf.drain_complete().is_empty());
    buf.feed(b"A");
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
}

#[test]
fn bare_escape_on_timeout() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b");
    assert!(buf.drain_complete().is_empty());
    // Timeout — force drain.
    let seqs = buf.drain_all();
    assert_eq!(seqs, vec![vec![0x1b]]);
    assert!(!buf.has_pending());
}

#[test]
fn ctrl_d_not_swallowed() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x04"); // Ctrl+D
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![vec![0x04]]);
}

#[test]
fn ctrl_c_not_swallowed() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x03"); // Ctrl+C
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![vec![0x03]]);
}

#[test]
fn mixed_text_and_escape() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"hello\x1b[Aworld");
    let seqs = buf.drain_complete();
    assert_eq!(seqs.len(), 11); // h, e, l, l, o, ESC[A, w, o, r, l, d
    assert_eq!(seqs[5], b"\x1b[A".to_vec()); // Up arrow
}

#[test]
fn csi_with_params() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b[13;2u"); // Kitty Shift+Enter
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"\x1b[13;2u".to_vec()]);
}

#[test]
fn kitty_press_and_release_in_one_read_split_into_sequences() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b[65;1:1u\x1b[65;1:3u");
    let seqs = buf.drain_complete();
    assert_eq!(seqs.len(), 2);
    assert_eq!(seqs[0], b"\x1b[65;1:1u".to_vec());
    assert_eq!(seqs[1], b"\x1b[65;1:3u".to_vec());
    assert!(crate::interface::kitty::is_key_release(&seqs[1]));
}

#[test]
fn ss3_sequence() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1bOA"); // SS3 Up arrow
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"\x1bOA".to_vec()]);
}

#[test]
fn alt_enter() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b\r"); // Alt+Enter
    let seqs = buf.drain_complete();
    assert_eq!(seqs, vec![b"\x1b\r".to_vec()]);
}

#[test]
fn utf8_char() {
    let mut buf = StdinBuffer::new();
    buf.feed("é".as_bytes());
    let seqs = buf.drain_complete();
    assert_eq!(seqs.len(), 1);
    assert_eq!(std::str::from_utf8(&seqs[0]).unwrap(), "é");
}

#[test]
fn bracketed_paste() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b[200~hello\x1b[201~");
    let seqs = buf.drain_complete();
    assert_eq!(seqs.len(), 1);
    assert!(seqs[0].starts_with(b"\x1b[200~"));
}

#[test]
fn raw_multiline_is_a_typed_paste_event_after_quiet_boundary() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"alpha\nbeta\ngamma\n");
    assert_eq!(buf.pending_reason(), Some(PendingReason::RawPaste));
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::Paste("alpha\nbeta\ngamma\n".into())]
    );
}

#[test]
fn lone_enter_replays_as_key_after_raw_candidate_timeout() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\r");
    assert_eq!(buf.pending_reason(), Some(PendingReason::RawCandidate));
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::KeySequence(b"\r".to_vec())]
    );
}

#[test]
fn a_subsequent_fragment_confirms_raw_paste_without_length_threshold() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"a\n");
    assert_eq!(buf.pending_reason(), Some(PendingReason::RawCandidate));
    buf.feed(b"b\n");
    assert_eq!(buf.pending_reason(), Some(PendingReason::RawPaste));
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::Paste("a\nb\n".into())]
    );
}

#[test]
fn confirmed_raw_paste_latch_keeps_short_follow_up_fragments() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"a\nb\n");
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::Paste("a\nb\n".into())]
    );
    buf.feed(b"\n");
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::Paste("\n".into())]
    );
    buf.feed("é".as_bytes());
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::Paste("é".into())]
    );
}

#[test]
fn explicit_enter_after_quiet_raw_paste_is_not_captured_by_latch() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"a\nb\n");
    assert!(matches!(
        buf.finish_pending(false).as_slice(),
        [InputEvent::Paste(_)]
    ));
    buf.begin_input_action(b"\r");
    buf.feed(b"\r");
    assert_eq!(
        buf.finish_pending(false),
        vec![InputEvent::KeySequence(b"\r".to_vec())]
    );
}

#[test]
fn bracketed_paste_waits_for_exact_end_marker() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b[200~body\n");
    assert_eq!(buf.pending_reason(), Some(PendingReason::BracketedPaste));
    assert!(buf.finish_pending(false).is_empty());
    buf.feed(b"tail\x1b[201~");
    assert_eq!(
        buf.drain_events(),
        vec![InputEvent::Paste("body\ntail".into())]
    );
}

// --- 3-fragment CSI split regression tests (#466) ---

/// Simulate the retry loop from app.rs (synchronous approximation).
///
/// `fragments` is a list of byte slices arriving in sequence.
/// `max_retries` is the maximum number of retry iterations.
/// Returns all emitted sequences.
///
/// Note: This does not model timing/timeouts — each fragment is assumed
/// to arrive within the retry window. For timeout-sensitive behavior,
/// an async integration test with tokio channels would be needed.
fn simulate_retry_loop(fragments: &[&[u8]], max_retries: usize) -> Vec<Vec<u8>> {
    let mut buf = StdinBuffer::new();
    let mut all_sequences = Vec::new();
    let mut frag_idx = 0;

    // Feed first fragment.
    if frag_idx < fragments.len() {
        buf.feed(fragments[frag_idx]);
        frag_idx += 1;
    }

    // Drain complete sequences immediately.
    all_sequences.extend(buf.drain_complete());

    // Retry loop while pending.
    let mut retries = 0;
    while buf.has_pending() && retries < max_retries {
        retries += 1;
        if frag_idx < fragments.len() {
            // More data arrives within timeout.
            buf.feed(fragments[frag_idx]);
            frag_idx += 1;
            all_sequences.extend(buf.drain_complete());
        } else {
            // Timeout — no more data.
            break;
        }
    }

    // Force drain anything still pending after retries exhausted.
    all_sequences.extend(buf.drain_all());

    all_sequences
}

#[test]
fn three_fragment_csi_with_multi_retry() {
    // 3-fragment split: ESC → [ → A
    // With max_retries=5, all 3 fragments arrive within the retry window.
    let seqs = simulate_retry_loop(&[b"\x1b", b"[", b"A"], 5);
    assert_eq!(
        seqs,
        vec![b"\x1b[A".to_vec()],
        "3-fragment CSI should be reassembled with multi-retry"
    );
}

#[test]
fn three_fragment_csi_with_single_retry_fails() {
    // 3-fragment split: ESC → [ → A
    // With max_retries=1 (the old bug), only ESC + [ arrive before drain_all.
    let seqs = simulate_retry_loop(&[b"\x1b", b"[", b"A"], 1);
    // With only 1 retry: ESC is pending, retry gets "[", ESC[ is still incomplete,
    // drain_all breaks it into ESC + "[", then "A" is fed but the loop is done.
    // This should NOT produce a clean ESC[A — this test documents the bug.
    assert_ne!(
        seqs,
        vec![b"\x1b[A".to_vec()],
        "single retry should NOT reassemble 3-fragment CSI (documents the bug)"
    );
}

#[test]
fn two_fragment_csi_with_multi_retry() {
    let seqs = simulate_retry_loop(&[b"\x1b", b"[A"], 5);
    assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
}

#[test]
fn bare_escape_after_retry_exhaustion() {
    // Only ESC arrives, no more fragments.
    let seqs = simulate_retry_loop(&[b"\x1b"], 5);
    assert_eq!(
        seqs,
        vec![vec![0x1b]],
        "bare ESC should be emitted after retries"
    );
}

#[test]
fn complete_sequence_no_retry() {
    let seqs = simulate_retry_loop(&[b"\x1b[A"], 5);
    assert_eq!(seqs, vec![b"\x1b[A".to_vec()]);
}

#[test]
fn four_fragment_csi_with_params() {
    // ESC → [ → 1;5 → C (Ctrl+Right)
    let seqs = simulate_retry_loop(&[b"\x1b", b"[", b"1;5", b"C"], 5);
    assert_eq!(seqs, vec![b"\x1b[1;5C".to_vec()]);
}

// --- Buffer size cap tests (#467 / #1176) ---

#[test]
fn feed_accepts_data_within_cap() {
    let mut buf = StdinBuffer::new();
    let data = vec![b'a'; 1000];
    assert!(buf.feed(&data));
    assert_eq!(buf.buf.len(), 1000);
}

#[test]
fn oversized_staged_input_is_rejected_as_a_whole() {
    let mut buf = StdinBuffer::new();
    assert!(buf.feed(&vec![b'a'; MAX_BUFFER_SIZE]));
    assert!(!buf.feed(b"extra"));
    assert!(
        buf.buf.is_empty(),
        "partial input must not survive overflow"
    );
    assert_eq!(buf.drain_events(), vec![InputEvent::Overflow]);
}

#[test]
fn broken_bracketed_paste_overflow_never_emits_partial_content() {
    let mut buf = StdinBuffer::new();
    assert!(buf.feed(BRACKETED_PASTE_START));
    assert!(!buf.feed(&vec![b'x'; MAX_BUFFER_SIZE]));
    assert_eq!(buf.drain_events(), vec![InputEvent::Overflow]);
    assert_eq!(buf.pending_reason(), Some(PendingReason::BracketedPaste));
    assert!(buf.finish_pending(true).is_empty());
    assert!(!buf.has_pending());
}

#[test]
fn eof_discards_incomplete_bracketed_paste_explicitly() {
    let mut buf = StdinBuffer::new();
    buf.feed(b"\x1b[200~partial");
    assert_eq!(
        buf.finish_pending(true),
        vec![InputEvent::IncompleteBracketedPaste]
    );
    assert!(!buf.has_pending());
}

#[test]
fn paste_end_marker_found_with_windows() {
    let mut buf = StdinBuffer::new();
    // Build a proper bracketed paste: start + content + end.
    let mut paste = Vec::new();
    paste.extend_from_slice(b"\x1b[200~");
    paste.extend_from_slice(b"hello world");
    paste.extend_from_slice(b"\x1b[201~");
    buf.feed(&paste);
    let seqs = buf.drain_complete();
    assert_eq!(seqs.len(), 1, "paste should be one sequence");
    assert!(seqs[0].starts_with(b"\x1b[200~"));
}
