//! The one rendering of untrusted persisted metadata on the socket (#2009,
//! #2011): bounded, with nothing a terminal or a reader could be misled by.

/// Untrusted persisted metadata is bounded and cannot inject terminal controls
/// or invisibly reorder or hide text.
pub(in crate::interface::cli) fn safe_display(raw: &str) -> String {
    let shown = raw.chars().take(4096);
    let safe = shown.map(|ch| if unsafe_display(ch) { '\u{fffd}' } else { ch });
    safe.collect()
}

/// Terminal controls, and the invisible format characters that reorder, hide
/// or split text: every Bidi_Control character, zero-width characters and
/// marks, line/paragraph separators, the soft hyphen, the grapheme joiner, the
/// Mongolian selectors, invisible operators, interlinear annotations, tag
/// characters and the byte-order mark. Emoji/ideographic variation selectors
/// stay: they change how a visible glyph is drawn and conceal nothing.
fn unsafe_display(ch: char) -> bool {
    ch.is_control()
        || matches!(ch,
            '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}
