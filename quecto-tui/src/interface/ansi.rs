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

/// Like [`ansi_segments`], but reproduces the legacy scanner quirk used by tab
/// expansion: a CSI sequence whose final byte is neither an ASCII letter nor
/// `~` (e.g. `\x1b[1@`, ICH) keeps consuming the following characters — up to
/// and including the first ASCII letter or `~` — as part of the `Escape`
/// segment. The tail scan stops early at the next `ESC` or end-of-string.
///
/// This exists so the quirk lives next to the canonical terminator rules
/// instead of being re-implemented with caller-side state (#984).
pub fn ansi_segments_legacy_csi(s: &str) -> AnsiSegmentsLegacyCsi<'_> {
    AnsiSegmentsLegacyCsi { rest: s }
}

/// Iterator returned by [`ansi_segments_legacy_csi`].
pub struct AnsiSegmentsLegacyCsi<'a> {
    rest: &'a str,
}

impl<'a> Iterator for AnsiSegmentsLegacyCsi<'a> {
    type Item = AnsiSegment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        if self.rest.starts_with('\x1b') {
            let mut len = escape_len(self.rest);
            let esc = &self.rest[..len];
            if esc.starts_with("\x1b[")
                && esc
                    .chars()
                    .last()
                    .is_some_and(|c| !c.is_ascii_alphabetic() && c != '~')
            {
                for ch in self.rest[len..].chars() {
                    if ch == '\x1b' {
                        break;
                    }
                    len += ch.len_utf8();
                    if ch.is_ascii_alphabetic() || ch == '~' {
                        break;
                    }
                }
            }
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
///
/// # Intentional semantics changes vs. the old inlined filters
///
/// The replaced sites used `chars().filter(|c| !c.is_control())`, which kept the
/// printable bytes that followed a malformed escape (e.g. a bare `ESC` plus a
/// letter left the letter visible). Routed through the canonical scanner, a
/// two-byte escape (`ESC` + one byte) now consumes that following byte too, so
/// `sanitize_control("a\x1bbc")` is `"ac"`, not `"abc"`. This uniform tightening
/// is intentional: these sites display untrusted ids/model names and do not key
/// or dedupe on the sanitized string, so dropping the stray byte is strictly
/// safer.
///
/// In addition to control characters, this also drops Unicode bidirectional
/// formatting/override characters (LRM/RLM, embeddings, overrides, and isolates)
/// to prevent Trojan-Source-style display spoofing of rendered ids and names.
pub fn sanitize_control(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            for ch in text.chars() {
                if keep_char(ch, false) {
                    out.push(ch);
                }
            }
        }
    }
    out
}

/// Like [`sanitize_control`] but keeps `\n` (for markdown source parsing); all
/// other control characters, escape sequences and bidi-control characters are
/// still dropped.
pub fn sanitize_control_keep_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            for ch in text.chars() {
                if keep_char(ch, true) {
                    out.push(ch);
                }
            }
        }
    }
    out
}

/// Like [`sanitize_control`] but stops after `max_chars` surviving characters.
///
/// Returns the (possibly truncated) sanitized string and whether truncation
/// occurred. This does in one pass and one allocation what callers previously
/// did with a full `sanitize_control` followed by a second `take(max_chars)`
/// collect plus a `.count()`.
pub fn sanitize_control_truncated(s: &str, max_chars: usize) -> (String, bool) {
    let mut out = String::with_capacity(s.len().min(max_chars.saturating_add(1)));
    let mut kept = 0usize;
    for seg in ansi_segments(s) {
        if let AnsiSegment::Text(text) = seg {
            for ch in text.chars() {
                if keep_char(ch, false) {
                    if kept == max_chars {
                        return (out, true);
                    }
                    out.push(ch);
                    kept += 1;
                }
            }
        }
    }
    (out, false)
}

/// Whether `ch` survives control/escape sanitization.
///
/// Drops ASCII/Unicode control characters (optionally keeping `\n`) and the
/// bidirectional formatting/override characters used in Trojan-Source display
/// spoofing.
pub(crate) fn keep_char(ch: char, keep_newlines: bool) -> bool {
    if keep_newlines && ch == '\n' {
        return true;
    }
    !ch.is_control() && !is_bidi_control(ch)
}

/// Unicode bidirectional control characters that can visually reorder text.
///
/// Covers the marks (LRM/RLM/ALM), embeddings/overrides (LRE..PDF + RLO/LRO),
/// and the directional isolates (LRI..PDI) — the full Trojan-Source set.
fn is_bidi_control(ch: char) -> bool {
    matches!(ch,
        '\u{200E}' | '\u{200F}' | '\u{061C}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
#[path = "ansi_tests.rs"]
mod ansi_tests;
