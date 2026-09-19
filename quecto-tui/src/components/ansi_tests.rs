//! RED-phase tests for the unified ANSI scanner (#758).
//!
//! These pin the canonical terminator behaviour that all ~8 previously
//! divergent scanners must converge on. They fail until `ansi.rs` is
//! implemented in the GREEN phase.

use super::{
    AnsiSegment, ansi_segments, ansi_segments_legacy_csi, sanitize_control,
    sanitize_control_keep_newlines, sanitize_control_truncated, sanitize_untrusted_label,
    strip_ansi,
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

#[test]
fn sanitize_control_two_byte_escape_consumes_following_byte() {
    // Documented intentional shift from the old `is_control()` filter: a bare
    // ESC plus a non-`[`/`]` byte now consumes that byte too.
    assert_eq!(sanitize_control("a\x1bbc"), "ac");
}

#[test]
fn sanitize_control_strips_bidi_override_and_isolates() {
    // Trojan-Source display-spoofing characters must not reach the renderer.
    assert_eq!(sanitize_control("a\u{202E}b\u{2066}c\u{200F}d"), "abcd");
}

#[test]
fn sanitize_control_keep_newlines_still_strips_bidi() {
    assert_eq!(sanitize_control_keep_newlines("a\u{202E}\nb"), "a\nb");
}

#[test]
fn sanitize_control_truncated_reports_overflow_and_truncates() {
    let (s, truncated) = sanitize_control_truncated("\x1b[31mhello\x1b[0m world", 5);
    assert_eq!(s, "hello");
    assert!(truncated);
}

#[test]
fn sanitize_control_truncated_no_overflow() {
    let (s, truncated) = sanitize_control_truncated("a\x00b\x1b[0mc", 10);
    assert_eq!(s, "abc");
    assert!(!truncated);
}

#[test]
fn sanitize_control_truncated_exact_fit_not_truncated() {
    let (s, truncated) = sanitize_control_truncated("abc", 3);
    assert_eq!(s, "abc");
    assert!(!truncated);
}

#[test]
fn sanitize_control_strips_c1_controls_without_swallowing_text() {
    // 8-bit C1 introducers (U+009B CSI, U+009D OSC) are stripped as lone
    // control characters; the following printable text must survive. Ported
    // from the deleted sanitize.rs `strips_c1_control_chars` test (#984).
    assert_eq!(sanitize_control("a\u{009B}31mb\u{009D}c"), "a31mbc");
}

// ── ansi_segments_legacy_csi: legacy CSI-tail quirk ────────────────────────

#[test]
fn legacy_csi_extends_nonalpha_final_byte_through_tail() {
    // `\x1b[1@` ends on `@` (non-alpha, non-`~`), so the legacy scanner keeps
    // consuming up to and including the first ASCII letter.
    let segs: Vec<_> = ansi_segments_legacy_csi("\x1b[1@Xrest").collect();
    assert_eq!(
        segs,
        vec![AnsiSegment::Escape("\x1b[1@X"), AnsiSegment::Text("rest"),]
    );
}

#[test]
fn legacy_csi_matches_canonical_for_alpha_and_tilde_finals() {
    for input in ["\x1b[31mred", "\x1b[3~del"] {
        let legacy: Vec<_> = ansi_segments_legacy_csi(input).collect();
        let canonical: Vec<_> = ansi_segments(input).collect();
        assert_eq!(legacy, canonical, "input {input:?}");
    }
}

#[test]
fn legacy_csi_tail_stops_at_next_escape() {
    let segs: Vec<_> = ansi_segments_legacy_csi("\x1b[1@\x1b[0mz").collect();
    assert_eq!(
        segs,
        vec![
            AnsiSegment::Escape("\x1b[1@"),
            AnsiSegment::Escape("\x1b[0m"),
            AnsiSegment::Text("z"),
        ]
    );
}

// ── untrusted labels (#2011 review R2-H2, R2-T6, R2-T11) ───────────────────

/// Every character that is invisible and hides, splits or reorders a label is
/// dropped — not only the zero-width set: the soft hyphen, the grapheme
/// joiner, the Mongolian selectors, line/paragraph separators, the invisible
/// operators, interlinear annotations, variation selectors and tag characters.
#[test]
fn an_untrusted_label_drops_every_invisible_character() {
    let invisible = ['\u{ad}', '\u{34f}', '\u{61c}', '\u{feff}']
        .into_iter()
        .chain('\u{180b}'..='\u{180f}')
        .chain('\u{200b}'..='\u{200f}')
        .chain('\u{2028}'..='\u{202e}')
        .chain('\u{2060}'..='\u{206f}')
        .chain('\u{fe00}'..='\u{fe0f}')
        .chain('\u{fff9}'..='\u{fffb}')
        .chain('\u{e0000}'..='\u{e007f}')
        .chain('\u{e0100}'..='\u{e01ef}');
    for ch in invisible {
        let label = sanitize_untrusted_label(&format!("a{ch}b"), 64);
        assert_eq!(label, "ab", "U+{:04X}", ch as u32);
    }
    // Chat prose keeps its joiners and selectors: only labels are this strict.
    assert_eq!(sanitize_control("a\u{200d}\u{fe0f}b"), "a\u{200d}\u{fe0f}b");
}

/// A combining-mark flood cannot overdraw the rows around a label: at most
/// two marks follow a base character; ordinary accents are untouched.
#[test]
fn an_untrusted_label_bounds_combining_marks_per_base_character() {
    let flood = format!("/p/a{}/tail", "\u{301}".repeat(3000));
    assert_eq!(
        sanitize_untrusted_label(&flood, 512),
        "/p/a\u{301}\u{301}/tail"
    );
    assert_eq!(
        sanitize_untrusted_label("e\u{301}a\u{308}\u{304}", 64),
        "e\u{301}a\u{308}\u{304}"
    );
    // A label that opens with marks has no base for them to sit on.
    assert_eq!(sanitize_untrusted_label("\u{301}\u{301}\u{301}x", 64), "x");
}
