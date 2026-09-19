//! The one rendering of untrusted persisted metadata on the socket (#2009,
//! #2011): bounded, with nothing a terminal or a reader could be misled by.

/// Untrusted persisted metadata is bounded and cannot inject terminal controls
/// or invisibly reorder or hide text.
pub(in crate::interface::cli) fn safe_display(raw: &str) -> String {
    raw.chars()
        .take(4096)
        .map(|ch| {
            if is_unsafe_display(ch) {
                '\u{fffd}'
            } else {
                ch
            }
        })
        .collect()
}

/// Terminal controls, and the invisible format characters that reorder or
/// hide text (bidi embeddings, overrides and isolates, zero-width characters
/// and marks, the byte-order mark): none reaches a client.
fn is_unsafe_display(ch: char) -> bool {
    ch.is_control()
        || matches!(ch, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}
