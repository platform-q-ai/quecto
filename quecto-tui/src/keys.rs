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
    /// Insert key.
    Insert,
    /// Shift + Enter (for newline in editor).
    ShiftEnter,
    /// A paste event (bracketed paste content).
    Paste(String),
    /// Mouse scroll up (wheel up).
    ScrollUp,
    /// Mouse scroll down (wheel down).
    ScrollDown,
    /// Mouse press (button 0 = left) at (column, row) — 0-indexed.
    MousePress(u16, u16),
    /// Mouse drag (button 0 held) at (column, row) — 0-indexed.
    MouseDrag(u16, u16),
    /// Mouse release at (column, row) — 0-indexed.
    MouseRelease(u16, u16),
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
        // Alt + Enter (CR or LF)
        0x0D | 0x0A => Some((Key::Alt('\n'), 2)),
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

    // SGR mouse: \x1b[< button;col;row M/m
    if rest.starts_with(b"<") {
        return parse_sgr_mouse(&rest[1..]);
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
        b'u' => {
            // Kitty keyboard protocol: CSI <keycode> ; <modifiers> u
            // Parse keycode and modifiers from params.
            parse_kitty_key(params)
        }
        b'~' => match params {
            b"1" => Key::Home, // CSI 1 ~
            b"2" => Key::Insert,
            b"3" => Key::Delete,
            b"5" => Key::PageUp,
            b"6" => Key::PageDown,
            _ => Key::Unknown(rest[..=i].to_vec()),
        },
        _ => Key::Unknown(rest[..=i].to_vec()),
    };

    Some((key, total_consumed))
}

/// Parse SGR mouse sequence (input starts after `\x1b[<`).
///
/// Format: `button;col;row` terminated by `M` (press) or `m` (release).
/// Button 64 = scroll up, 65 = scroll down.
/// Button 0 = left press, 32 = left drag (#528).
/// Release is indicated by lowercase `m` terminator.
fn parse_sgr_mouse(rest: &[u8]) -> Option<(Key, usize)> {
    // Collect digits and semicolons until M or m terminator.
    let mut i = 0;
    while i < rest.len() && rest[i] != b'M' && rest[i] != b'm' {
        if !rest[i].is_ascii_digit() && rest[i] != b';' {
            // Invalid character — consume up to here and discard.
            // \x1b(1) + [(1) + <(1) + bytes up to invalid(i) + invalid(1) = i + 4
            return Some((Key::Unknown(Vec::new()), i + 4));
        }
        i += 1;
    }
    if i >= rest.len() {
        return None; // incomplete — waiting for M/m terminator
    }

    let is_release = rest[i] == b'm';

    // Total consumed from original input: \x1b(1) + [(1) + <(1) + params(i) + terminator(1)
    let total_consumed = i + 4;

    // Parse button;col;row from params.
    let params = std::str::from_utf8(&rest[..i]).unwrap_or("");
    let mut fields = params.split(';');
    let button: u32 = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let col: u16 = fields
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(1)
        .saturating_sub(1); // SGR uses 1-indexed columns
    let row: u16 = fields
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(1)
        .saturating_sub(1); // SGR uses 1-indexed rows

    match button {
        64 => Some((Key::ScrollUp, total_consumed)),
        65 => Some((Key::ScrollDown, total_consumed)),
        0 if is_release => Some((Key::MouseRelease(col, row), total_consumed)),
        0 => Some((Key::MousePress(col, row), total_consumed)),
        32 => Some((Key::MouseDrag(col, row), total_consumed)),
        _ => {
            // Silently consume other mouse events (right-click, middle-click, etc.)
            // — no heap allocation, just discard.
            Some((Key::Unknown(Vec::new()), total_consumed))
        }
    }
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

/// Parse a Kitty keyboard protocol key: `CSI <keycode> ; <modifiers> u`.
///
/// Common cases:
/// - `CSI 13 ; 2 u` = Shift+Enter (keycode 13, modifier 2=Shift)
/// - `CSI 9 ; 2 u` = Shift+Tab
fn parse_kitty_key(params: &[u8]) -> Key {
    // Parse "keycode;modifiers" from params bytes.
    let s = std::str::from_utf8(params).unwrap_or("");
    let parts: Vec<&str> = s.split(';').collect();
    let keycode: u32 = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let modifiers: u32 = parts
        .get(1)
        .and_then(|p| {
            // Modifiers may contain event type after colon (e.g. "2:1" for press).
            p.split(':').next().and_then(|m| m.parse().ok())
        })
        .unwrap_or(1); // 1 = no modifier in Kitty protocol

    // Kitty protocol: modifier value 1 = no modifier. Bits are (value - 1):
    //   bit 0 = Shift, bit 1 = Alt, bit 2 = Ctrl.
    // Guard against modifiers == 0 (malformed input) with saturating_sub.
    let mod_bits = modifiers.saturating_sub(1);
    let shift = mod_bits & 1 != 0;
    let alt = mod_bits & 2 != 0;
    let ctrl = mod_bits & 4 != 0;

    match keycode {
        13 if shift => Key::ShiftEnter,
        13 => Key::Enter,
        9 if shift => Key::BackTab,
        9 => Key::Tab,
        127 => Key::Backspace,
        27 => Key::Escape,
        // Ctrl+letter: keycode 97..=122 (a-z) with ctrl modifier.
        // Ctrl+Alt is deliberately treated as Ctrl-only to avoid
        // triggering unintended actions (Alt is dropped).
        97..=122 if ctrl => Key::Ctrl((keycode as u8) as char),
        // Alt+letter: keycode 97..=122 (a-z) with alt modifier (no ctrl).
        97..=122 if alt => Key::Alt((keycode as u8) as char),
        // Plain printable ASCII (keycode 32..=126) with no modifier.
        32..=126 if !ctrl && !alt && !shift => Key::Char(char::from(keycode as u8)),
        _ => Key::Unknown(params.to_vec()),
    }
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

    // ── Kitty protocol Ctrl+letter tests (issue #496) ─────────────────

    #[test]
    fn kitty_ctrl_d() {
        // CSI 100;5u — keycode 100='d', modifier 5=Ctrl+1
        let (key, _) = parse_key(b"\x1b[100;5u").unwrap();
        assert_eq!(key, Key::Ctrl('d'));
    }

    #[test]
    fn kitty_ctrl_c() {
        // CSI 99;5u — keycode 99='c', modifier 5=Ctrl+1
        let (key, _) = parse_key(b"\x1b[99;5u").unwrap();
        assert_eq!(key, Key::Ctrl('c'));
    }

    #[test]
    fn kitty_ctrl_a() {
        let (key, _) = parse_key(b"\x1b[97;5u").unwrap();
        assert_eq!(key, Key::Ctrl('a'));
    }

    #[test]
    fn kitty_ctrl_z() {
        let (key, _) = parse_key(b"\x1b[122;5u").unwrap();
        assert_eq!(key, Key::Ctrl('z'));
    }

    #[test]
    fn kitty_ctrl_l() {
        let (key, _) = parse_key(b"\x1b[108;5u").unwrap();
        assert_eq!(key, Key::Ctrl('l'));
    }

    #[test]
    fn kitty_ctrl_o() {
        let (key, _) = parse_key(b"\x1b[111;5u").unwrap();
        assert_eq!(key, Key::Ctrl('o'));
    }

    #[test]
    fn kitty_plain_d_no_modifier() {
        // CSI 100;1u — keycode 100='d', modifier 1=none
        let (key, _) = parse_key(b"\x1b[100;1u").unwrap();
        assert_eq!(key, Key::Char('d'));
    }

    #[test]
    fn kitty_alt_d() {
        // CSI 100;3u — keycode 100='d', modifier 3=Alt+1
        let (key, _) = parse_key(b"\x1b[100;3u").unwrap();
        assert_eq!(key, Key::Alt('d'));
    }

    #[test]
    fn kitty_shift_enter_still_works() {
        let (key, _) = parse_key(b"\x1b[13;2u").unwrap();
        assert_eq!(key, Key::ShiftEnter);
    }

    #[test]
    fn kitty_modifier_zero_no_panic() {
        // Modifier 0 is malformed but should not panic (saturating_sub).
        let (key, _) = parse_key(b"\x1b[100;0u").unwrap();
        // Should parse as plain 'd' (no modifiers after saturating_sub).
        assert_eq!(key, Key::Char('d'));
    }

    #[test]
    fn kitty_ctrl_alt_d_treated_as_ctrl() {
        // Modifier 7 = Ctrl+Alt+1 → (7-1)=6, ctrl=true, alt=true.
        // Ctrl arm matches first (Alt dropped deliberately).
        let (key, _) = parse_key(b"\x1b[100;7u").unwrap();
        assert_eq!(key, Key::Ctrl('d'));
    }

    // ── SGR mouse scroll tests (issue #519) ───────────────────────────

    #[test]
    fn sgr_mouse_scroll_up() {
        // \x1b[<64;10;5M — scroll up at column 10, row 5 (11 bytes)
        let input = b"\x1b[<64;10;5M";
        assert_eq!(input.len(), 11);
        let (key, n) = parse_key(input).unwrap();
        assert_eq!(key, Key::ScrollUp);
        assert_eq!(n, 11);
    }

    #[test]
    fn sgr_mouse_scroll_down() {
        // \x1b[<65;10;5M — scroll down at column 10, row 5 (11 bytes)
        let input = b"\x1b[<65;10;5M";
        assert_eq!(input.len(), 11);
        let (key, n) = parse_key(input).unwrap();
        assert_eq!(key, Key::ScrollDown);
        assert_eq!(n, 11);
    }

    #[test]
    fn sgr_mouse_left_click_press() {
        // \x1b[<0;10;5M — left click press at col 10, row 5 → MousePress(9, 4) (0-indexed)
        let (key, _) = parse_key(b"\x1b[<0;10;5M").unwrap();
        assert_eq!(key, Key::MousePress(9, 4));
    }

    #[test]
    fn sgr_mouse_scroll_up_release() {
        // \x1b[<64;10;5m — scroll up release (lowercase m)
        let (key, _) = parse_key(b"\x1b[<64;10;5m").unwrap();
        assert_eq!(key, Key::ScrollUp);
    }

    #[test]
    fn sgr_mouse_incomplete_returns_none() {
        // Incomplete SGR mouse sequence
        assert!(parse_key(b"\x1b[<64;10;").is_none());
    }

    // ── Mouse press/drag/release tests (issue #528) ──────────────────

    #[test]
    fn sgr_mouse_left_release() {
        // \x1b[<0;20;10m — left release at col 20, row 10 → MouseRelease(19, 9)
        let (key, _) = parse_key(b"\x1b[<0;20;10m").unwrap();
        assert_eq!(key, Key::MouseRelease(19, 9));
    }

    #[test]
    fn sgr_mouse_drag() {
        // \x1b[<32;15;7M — left drag at col 15, row 7 → MouseDrag(14, 6)
        let (key, _) = parse_key(b"\x1b[<32;15;7M").unwrap();
        assert_eq!(key, Key::MouseDrag(14, 6));
    }

    #[test]
    fn sgr_mouse_press_at_origin() {
        // Column 1, row 1 → 0-indexed (0, 0)
        let (key, _) = parse_key(b"\x1b[<0;1;1M").unwrap();
        assert_eq!(key, Key::MousePress(0, 0));
    }

    #[test]
    fn sgr_mouse_right_click_ignored() {
        // Button 2 = right click → Unknown
        let (key, _) = parse_key(b"\x1b[<2;10;5M").unwrap();
        assert!(matches!(key, Key::Unknown(_)));
    }

    #[test]
    fn sgr_mouse_right_release_ignored() {
        // Button 2 release → Unknown (only left button release triggers MouseRelease)
        let (key, _) = parse_key(b"\x1b[<2;10;5m").unwrap();
        assert!(matches!(key, Key::Unknown(_)));
    }
}
