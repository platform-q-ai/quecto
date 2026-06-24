//! Single shared ANSI escape-sequence scanner (#758).
//!
//! Historically the crate hand-rolled the same ESC / CSI / OSC state machine in
//! ~8 places, each with slightly different terminator rules. This module is the
//! one correct implementation: all visible-width, truncation, highlight,
//! selection-strip and control-sanitize helpers consume [`ansi_segments`].
//!
//! Terminator rules (the canonical, agreed behaviour):
//! - **CSI** (`ESC [`): parameter/intermediate bytes followed by a final byte in
//!   the range `0x40..=0x7E` (`@`..=`~`).
//! - **OSC** (`ESC ]`): terminated by BEL (`\x07`) **or** ST (`ESC \`).
//! - **Other** two-byte escapes (`ESC` + one byte): the single following byte.
//! - An unterminated escape at end-of-string consumes the rest of the string.

/// A classified slice of a terminal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiSegment<'a> {
    /// A run of ordinary (non-escape) characters. May still contain stray
    /// control characters such as `\n` or `\x07`; callers decide how to treat
    /// those.
    Text(&'a str),
    /// A complete escape sequence, including the leading `ESC` and its
    /// terminator (CSI, OSC, or a two-byte escape).
    Escape(&'a str),
}

/// Iterate over `s` as alternating [`AnsiSegment::Text`] / [`AnsiSegment::Escape`]
/// slices, using the one canonical terminator ruleset documented on this module.
///
/// Consecutive escape sequences each yield their own `Escape` segment; runs of
/// text between escapes are coalesced into a single `Text` segment.
pub fn ansi_segments(s: &str) -> AnsiSegments<'_> {
    AnsiSegments { rest: s }
}

/// Iterator returned by [`ansi_segments`].
pub struct AnsiSegments<'a> {
    rest: &'a str,
}

impl<'a> Iterator for AnsiSegments<'a> {
    type Item = AnsiSegment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.starts_with('\x1b') {
            let len = escape_len(self.rest);
            let (escape, rest) = self.rest.split_at(len);
            self.rest = rest;
            Some(AnsiSegment::Escape(escape))
        } else {
            let end = self.rest.find('\x1b').unwrap_or(self.rest.len());
            let (text, rest) = self.rest.split_at(end);
            self.rest = rest;
            Some(AnsiSegment::Text(text))
        }
    }
}

/// Byte length of the escape sequence starting at the leading `ESC` of `s`.
///
/// `s` must begin with `ESC` (`0x1b`). An unterminated sequence consumes the
/// remainder of the string.
fn escape_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    debug_assert_eq!(bytes.first(), Some(&0x1b));
    match bytes.get(1) {
        // CSI: parameter/intermediate bytes then a final byte in 0x40..=0x7E.
        Some(b'[') => {
            let mut i = 2;
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if (0x40..=0x7E).contains(&b) {
                    return i;
                }
            }
            bytes.len()
        }
        // OSC: terminated by BEL (0x07) or ST (ESC \). The terminator bytes are
        // all < 0x80, so scanning raw bytes never splits a UTF-8 sequence.
        Some(b']') => {
            let mut i = 2;
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    return i + 1;
                }
                if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                    return i + 2;
                }
                i += 1;
            }
            bytes.len()
        }
        // Other two-byte escapes: ESC plus the single following character.
        Some(_) => match s[1..].chars().next() {
            Some(c) => 1 + c.len_utf8(),
            None => 1,
        },
        // Lone ESC at end of string.
        None => 1,
    }
}

/// Remove every ANSI escape sequence, keeping all other characters verbatim
/// (including ordinary control characters and newlines).
///
/// This replaces both `app::strip_ansi_for_selection` and
/// `app_methods::strip_ansi`, which were two divergent implementations of the
/// same job.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            out.push_str(text);
        }
    }
    out
}

/// Remove ANSI escape sequences **and** all control characters — for untrusted
/// ids, model names and similar single-line display strings.
///
/// This is the single shared replacement for the ~8 inlined
/// `chars().filter(|c| !c.is_control())` sites and the `strip_terminal_control`
/// wrapper.
pub fn sanitize_control(s: &str) -> String {
    sanitize_control_inner(s, false)
}

/// Like [`sanitize_control`] but keeps `\n` (for markdown source parsing); all
/// other control characters and escape sequences are still dropped.
pub fn sanitize_control_keep_newlines(s: &str) -> String {
    sanitize_control_inner(s, true)
}

fn sanitize_control_inner(s: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            for ch in text.chars() {
                if (keep_newlines && ch == '\n') || !ch.is_control() {
                    out.push(ch);
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "ansi_tests.rs"]
mod ansi_tests;
