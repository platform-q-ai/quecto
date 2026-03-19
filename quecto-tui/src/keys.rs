//! Key parsing — converts raw terminal escape sequences into [`Key`] values.
//!
//! No external crates; just pattern matching on VT100/xterm sequences.

/// A parsed terminal key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// A printable Unicode character.
    Char(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    BackTab, // Shift+Tab
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// Ctrl + a lowercase letter (e.g. Ctrl+C = Ctrl('c')).
    Ctrl(char),
    /// Alt + a character.
    Alt(char),
    /// A paste event (bracketed paste content).
    Paste(String),
    /// Unrecognised sequence.
    Unknown(Vec<u8>),
}

/// Parse a single key event from raw terminal input bytes.
///
/// Returns `(key, bytes_consumed)`. If the input is incomplete (e.g. a partial
/// escape sequence), returns `None`.
pub fn parse_key(input: &[u8]) -> Option<(Key, usize)> {
    if input.is_empty() {
        return None;
    }

    match input[0] {
        // ── Ctrl keys ──────────────────────────────────────────────
        0x00 => Some((Key::Ctrl('@'), 1)), // Ctrl+@/Space
        0x01..=0x07 => Some((Key::Ctrl((input[0] + b'a' - 1) as char), 1)), // Ctrl+A..G
        0x08 => Some((Key::Backspace, 1)), // Ctrl+H / Backspace
        0x09 => Some((Key::Tab, 1)),       // Ctrl+I / Tab
        0x0A => Some((Key::Enter, 1)),     // Ctrl+J / LF
        0x0B..=0x0C => Some((Key::Ctrl((input[0] + b'a' - 1) as char), 1)), // Ctrl+K..L
        0x0D => Some((Key::Enter, 1)),     // Ctrl+M / CR
        0x0E..=0x1A => Some((Key::Ctrl((input[0] + b'a' - 1) as char), 1)), // Ctrl+N..Z
        0x1B => parse_escape(&input[1..]),
        0x7F => Some((Key::Backspace, 1)), // DEL

        // ── Printable UTF-8 ────────────────────────────────────────
        _ => parse_utf8_char(input),
    }
}

/// Parse an escape sequence (input starts after the leading `\x1b`).
fn parse_escape(rest: &[u8]) -> Option<(Key, usize)> {
    if rest.is_empty() {
        // Bare Escape
        return Some((Key::Escape, 1));
    }

    match rest[0] {
        b'[' => parse_csi(&rest[1..]),
        b'O' => parse_ss3(&rest[1..]),
        // Alt + printable character
        0x20..=0x7E => Some((Key::Alt(rest[0] as char), 2)),
        _ => Some((Key::Escape, 1)),
    }
}

/// Parse a CSI sequence (input starts after `\x1b[`).
fn parse_csi(rest: &[u8]) -> Option<(Key, usize)> {
    if rest.is_empty() {
        return None; // incomplete
    }

    // Bracketed paste: \x1b[200~ ... \x1b[201~
    if rest.starts_with(b"200~") {
        return parse_bracketed_paste(&rest[4..]);
    }

    // Collect parameter bytes (digits and semicolons)
    let mut i = 0;
    while i < rest.len() && (rest[i].is_ascii_digit() || rest[i] == b';') {
        i += 1;
    }

    if i >= rest.len() {
        return None; // incomplete — waiting for terminator
    }

    let params = &rest[..i];
    let terminator = rest[i];
    let total_consumed = 2 + i + 1; // \x1b + [ + params + terminator

    let key = match terminator {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::BackTab,
        b'~' => match params {
            b"2" => Key::Home, // some terminals
            b"3" => Key::Delete,
            b"5" => Key::PageUp,
            b"6" => Key::PageDown,
            _ => Key::Unknown(rest[..=i].to_vec()),
        },
        _ => Key::Unknown(rest[..=i].to_vec()),
    };

    Some((key, total_consumed))
}

/// Parse an SS3 sequence (input starts after `\x1b O`).
fn parse_ss3(rest: &[u8]) -> Option<(Key, usize)> {
    if rest.is_empty() {
        return None; // incomplete
    }

    let total_consumed = 3; // \x1b + O + char
    let key = match rest[0] {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        _ => Key::Unknown(vec![b'O', rest[0]]),
    };

    Some((key, total_consumed))
}

/// Parse bracketed paste content. Input starts after `\x1b[200~`.
fn parse_bracketed_paste(rest: &[u8]) -> Option<(Key, usize)> {
    // Search for the end marker: \x1b[201~
    let end_marker = b"\x1b[201~";
    if let Some(pos) = find_subsequence(rest, end_marker) {
        let content = String::from_utf8_lossy(&rest[..pos]).into_owned();
        // total: \x1b[ (2) + 200~ (4) + content + \x1b[201~ (6)
        let consumed = 2 + 4 + pos + end_marker.len();
        Some((Key::Paste(content), consumed))
    } else {
        None // incomplete paste
    }
}

/// Parse a single UTF-8 character from the input.
fn parse_utf8_char(input: &[u8]) -> Option<(Key, usize)> {
    let s = std::str::from_utf8(input).ok().or_else(|| {
        // Try progressively shorter prefixes (1–4 bytes)
        for len in (1..=4.min(input.len())).rev() {
            if let Ok(s) = std::str::from_utf8(&input[..len]) {
                return Some(s);
            }
        }
        None
    })?;

    let ch = s.chars().next()?;
    Some((Key::Char(ch), ch.len_utf8()))
}

/// Find a byte subsequence within a slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ─── Convenience matchers ─────────────────────────────────────────────────────

impl Key {
    /// Check if this key matches Ctrl+C.
    pub fn is_ctrl_c(&self) -> bool {
        matches!(self, Key::Ctrl('c'))
    }

    /// Check if this key is a printable character.
    pub fn is_char(&self) -> bool {
        matches!(self, Key::Char(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arrow_up() {
        let input = b"\x1b[A";
        let (key, n) = parse_key(input).unwrap();
        assert_eq!(key, Key::Up);
        assert_eq!(n, 3);
    }

    #[test]
    fn parse_arrow_down() {
        let (key, _) = parse_key(b"\x1b[B").unwrap();
        assert_eq!(key, Key::Down);
    }

    #[test]
    fn parse_arrow_right() {
        let (key, _) = parse_key(b"\x1b[C").unwrap();
        assert_eq!(key, Key::Right);
    }

    #[test]
    fn parse_arrow_left() {
        let (key, _) = parse_key(b"\x1b[D").unwrap();
        assert_eq!(key, Key::Left);
    }

    #[test]
    fn parse_enter_cr() {
        let (key, n) = parse_key(b"\r").unwrap();
        assert_eq!(key, Key::Enter);
        assert_eq!(n, 1);
    }

    #[test]
    fn parse_enter_lf() {
        let (key, _) = parse_key(b"\n").unwrap();
        assert_eq!(key, Key::Enter);
    }

    #[test]
    fn parse_backspace() {
        let (key, _) = parse_key(b"\x7f").unwrap();
        assert_eq!(key, Key::Backspace);
    }

    #[test]
    fn parse_escape_bare() {
        let (key, n) = parse_key(b"\x1b").unwrap();
        assert_eq!(key, Key::Escape);
        assert_eq!(n, 1);
    }

    #[test]
    fn parse_tab() {
        let (key, _) = parse_key(b"\t").unwrap();
        assert_eq!(key, Key::Tab);
    }

    #[test]
    fn parse_delete() {
        let (key, _) = parse_key(b"\x1b[3~").unwrap();
        assert_eq!(key, Key::Delete);
    }

    #[test]
    fn parse_home() {
        let (key, _) = parse_key(b"\x1b[H").unwrap();
        assert_eq!(key, Key::Home);
    }

    #[test]
    fn parse_end() {
        let (key, _) = parse_key(b"\x1b[F").unwrap();
        assert_eq!(key, Key::End);
    }

    #[test]
    fn parse_ctrl_c() {
        let (key, _) = parse_key(b"\x03").unwrap();
        assert_eq!(key, Key::Ctrl('c'));
        assert!(key.is_ctrl_c());
    }

    #[test]
    fn parse_ctrl_a() {
        let (key, _) = parse_key(b"\x01").unwrap();
        assert_eq!(key, Key::Ctrl('a'));
    }

    #[test]
    fn parse_ctrl_d() {
        let (key, _) = parse_key(b"\x04").unwrap();
        assert_eq!(key, Key::Ctrl('d'));
    }

    #[test]
    fn parse_printable_ascii() {
        let (key, n) = parse_key(b"a").unwrap();
        assert_eq!(key, Key::Char('a'));
        assert_eq!(n, 1);
    }

    #[test]
    fn parse_printable_utf8() {
        let input = "é".as_bytes();
        let (key, n) = parse_key(input).unwrap();
        assert_eq!(key, Key::Char('é'));
        assert_eq!(n, 2);
    }

    #[test]
    fn parse_printable_cjk() {
        let input = "日".as_bytes();
        let (key, n) = parse_key(input).unwrap();
        assert_eq!(key, Key::Char('日'));
        assert_eq!(n, 3);
    }

    #[test]
    fn parse_alt_a() {
        let (key, n) = parse_key(b"\x1ba").unwrap();
        assert_eq!(key, Key::Alt('a'));
        assert_eq!(n, 2);
    }

    #[test]
    fn parse_page_up() {
        let (key, _) = parse_key(b"\x1b[5~").unwrap();
        assert_eq!(key, Key::PageUp);
    }

    #[test]
    fn parse_page_down() {
        let (key, _) = parse_key(b"\x1b[6~").unwrap();
        assert_eq!(key, Key::PageDown);
    }

    #[test]
    fn parse_shift_tab() {
        let (key, _) = parse_key(b"\x1b[Z").unwrap();
        assert_eq!(key, Key::BackTab);
    }

    #[test]
    fn parse_bracketed_paste() {
        let input = b"\x1b[200~hello world\x1b[201~";
        let (key, n) = parse_key(input).unwrap();
        assert_eq!(key, Key::Paste("hello world".to_string()));
        assert_eq!(n, input.len());
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_key(b"").is_none());
    }

    #[test]
    fn parse_incomplete_csi_returns_none() {
        // \x1b[ without a terminator — should return None (wait for more input)
        assert!(parse_key(b"\x1b[").is_none());
    }
}
