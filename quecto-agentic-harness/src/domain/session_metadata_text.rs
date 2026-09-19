//! The text of session metadata (#2010): what a safe renderer would show,
//! folded so that the natural spelling of a word finds it, and how a path
//! that is not text is spelled. Pure: no tables beyond `std`'s.

/// The text a safe renderer would show, folded for comparison: every control
/// and invisible format character dropped, whitespace runs collapsed to one
/// space and trimmed, and case folded — Unicode lower-casing of the whole
/// text, then the spellings lower-casing keeps apart: final sigma is a sigma,
/// sharp s is `ss`, a dotless `ı` is an `i`, and the dot a dotted capital `İ`
/// lower-cases into is dropped (only after an `i`: any other letter keeps its
/// marks). Two limits remain, both documented: code points are compared as
/// stored — a composed and a decomposed spelling of one glyph are two texts
/// (the harness carries no normalization tables) — and ligatures and other
/// full-fold expansions (`ﬁ`) are not expanded.
pub fn visible_text(raw: &str) -> String {
    let shown: String = raw.chars().filter(|ch| !invisible(*ch)).collect();
    let mut folded = String::with_capacity(shown.len());
    for ch in shown.to_lowercase().chars() {
        match ch {
            'ς' => folded.push('σ'),
            'ß' => folded.push_str("ss"),
            'ı' => folded.push('i'),
            '\u{307}' if folded.ends_with('i') => {}
            other => folded.push(other),
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Controls that are not whitespace, and the format characters that reorder,
/// hide or split text (bidi controls, zero-width characters, the soft hyphen,
/// invisible operators, tags, the byte-order mark).
fn invisible(ch: char) -> bool {
    (ch.is_control() && !ch.is_whitespace())
        || matches!(ch,
            '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

/// A path as text. One that is text is itself; in one that is not, each byte
/// that is no UTF-8 is spelled `\xNN` (R1-H10), so two folders that differ
/// only in such a byte stay two — a lossy `U+FFFD` would make them one.
pub fn display_path(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut shown = String::new();
    for chunk in path.as_os_str().as_bytes().utf8_chunks() {
        shown.push_str(chunk.valid());
        for byte in chunk.invalid() {
            shown.push_str(&format!("\\x{byte:02X}"));
        }
    }
    shown
}

#[cfg(test)]
#[path = "session_metadata_text_tests.rs"]
mod tests;
