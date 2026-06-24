//! RED-phase tests for the unified ANSI scanner (#758).
//!
//! These pin the canonical terminator behaviour that all ~8 previously
//! divergent scanners must converge on. They fail until `ansi.rs` is
//! implemented in the GREEN phase.

use super::{
    AnsiSegment, ansi_segments, sanitize_control, sanitize_control_keep_newlines, strip_ansi,
};

// ── ansi_segments: classification ─────────────────────────────────────────

#[test]
fn segments_plain_text_is_single_text_segment() {
    let segs: Vec<_> = ansi_segments("hello world").collect();
    assert_eq!(segs, vec![AnsiSegment::Text("hello world")]);
}

#[test]
fn segments_empty_string_yields_nothing() {
    let segs: Vec<_> = ansi_segments("").collect();
    assert_eq!(segs, Vec::<AnsiSegment>::new());
}

#[test]
fn segments_splits_csi_from_text() {
    let segs: Vec<_> = ansi_segments("\x1b[31mred\x1b[0m").collect();
    assert_eq!(
        segs,
        vec![
            AnsiSegment::Escape("\x1b[31m"),
            AnsiSegment::Text("red"),
            AnsiSegment::Escape("\x1b[0m"),
        ]
    );
}

#[test]
fn segments_csi_final_byte_at_range_edges() {
    // `@` (0x40) and `~` (0x7E) are both valid CSI final bytes.
    let segs: Vec<_> = ansi_segments("\x1b[1@x\x1b[3~y").collect();
    assert_eq!(
        segs,
        vec![
            AnsiSegment::Escape("\x1b[1@"),
            AnsiSegment::Text("x"),
            AnsiSegment::Escape("\x1b[3~"),
            AnsiSegment::Text("y"),
        ]
    );
}

#[test]
fn segments_osc_terminated_by_bel() {
    let segs: Vec<_> = ansi_segments("\x1b]0;title\x07body").collect();
    assert_eq!(
        segs,
        vec![
            AnsiSegment::Escape("\x1b]0;title\x07"),
            AnsiSegment::Text("body"),
        ]
    );
}

#[test]
fn segments_osc_terminated_by_st() {
    // ST = ESC \ . This is the terminator that several old scanners ignored.
    let segs: Vec<_> = ansi_segments("\x1b]8;;https://x\x1b\\link").collect();
    assert_eq!(
        segs,
        vec![
            AnsiSegment::Escape("\x1b]8;;https://x\x1b\\"),
            AnsiSegment::Text("link"),
        ]
    );
}

#[test]
fn segments_unterminated_escape_consumes_remainder() {
    let segs: Vec<_> = ansi_segments("ok\x1b[31").collect();
    assert_eq!(
        segs,
        vec![AnsiSegment::Text("ok"), AnsiSegment::Escape("\x1b[31")]
    );
}

// ── strip_ansi: the two old impls must collapse to one ─────────────────────

#[test]
fn strip_ansi_removes_csi() {
    assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
}

#[test]
fn strip_ansi_removes_csi_with_tilde_terminator() {
    assert_eq!(strip_ansi("a\x1b[3~b"), "ab");
}

#[test]
fn strip_ansi_removes_osc_with_bel() {
    assert_eq!(strip_ansi("\x1b]0;t\x07x"), "x");
}

#[test]
fn strip_ansi_removes_osc_with_st() {
    assert_eq!(strip_ansi("\x1b]8;;url\x1b\\link\x1b]8;;\x1b\\"), "link");
}

#[test]
fn strip_ansi_keeps_plain_control_chars_and_newlines() {
    // strip_ansi removes only escape sequences, not bare control chars.
    assert_eq!(strip_ansi("a\nb"), "a\nb");
}

#[test]
fn strip_ansi_preserves_unicode() {
    assert_eq!(strip_ansi("café\x1b[0m🎉"), "café🎉");
}

// ── strip_ansi: the resolved CSI-terminator divergence ─────────────────────

#[test]
fn strip_ansi_terminates_csi_on_at_sign() {
    // Regression for the #758 divergence: the old selection scanner treated the
    // CSI final byte as "ascii-alpha or `~`", so it ran past `@` (0x40) and ate
    // following text; the diagnostic scanner used 0x40..=0x7E and stopped at `@`.
    // The unified scanner stops at `@`, keeping the trailing text.
    assert_eq!(strip_ansi("\x1b[1@x"), "x");
}

// ── sanitize_control: replaces the ~8 inlined filters + wrappers ───────────

#[test]
fn sanitize_control_drops_control_and_escapes() {
    assert_eq!(sanitize_control("a\x00b\x7f\x1b[31mc"), "abc");
}

#[test]
fn sanitize_control_drops_newlines_by_default() {
    assert_eq!(sanitize_control("a\nb"), "ab");
}

#[test]
fn sanitize_control_keep_newlines_keeps_only_newlines() {
    assert_eq!(sanitize_control_keep_newlines("a\nb\x07"), "a\nb");
}

#[test]
fn sanitize_control_strips_osc_with_st() {
    assert_eq!(sanitize_control("\x1b]8;;url\x1b\\link"), "link");
}
