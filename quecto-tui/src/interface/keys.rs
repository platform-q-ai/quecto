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
    /// Ctrl+Shift + a lowercase letter (e.g. Ctrl+Shift+A = CtrlShift('a')).
    CtrlShift(char),
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

    // Collect CSI parameter/intermediate bytes until the final byte.
    // Kitty keyboard protocol may include ':' alternate-key/event fields.
    let mut i = 0;
    while i < rest.len() && !(0x40..=0x7E).contains(&rest[i]) {
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
            _ => {
                parse_modify_other_keys(params).unwrap_or_else(|| Key::Unknown(rest[..=i].to_vec()))
            }
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
    let key_field = parts.first().copied().unwrap_or("");
    let mut key_fields = key_field.split(':');
    let primary_keycode: u32 = key_fields.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let _shifted_keycode: Option<u32> = key_fields
        .next()
        .filter(|p| !p.is_empty())
        .and_then(|p| p.parse().ok());
    let base_layout_keycode: Option<u32> = key_fields.next().and_then(|p| p.parse().ok());
    let keycode = effective_kitty_keycode(primary_keycode, base_layout_keycode);
    let modifier_field = parts.get(1).copied().unwrap_or("1");
    let mut modifier_parts = modifier_field.split(':');
    let modifiers: u32 = modifier_parts
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(1); // 1 = no modifier in Kitty protocol
    if matches!(modifier_parts.next(), Some("3")) {
        return Key::Unknown(params.to_vec());
    }

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
        // Ctrl+Shift+letter: terminals may report either lowercase base
        // keycodes (97..=122) or uppercase shifted keycodes (65..=90).
        97..=122 if ctrl && shift => Key::CtrlShift((keycode as u8) as char),
        65..=90 if ctrl && shift => Key::CtrlShift(((keycode as u8) + b'a' - b'A') as char),
        // Ctrl+letter: keycode 97..=122 (a-z) with ctrl modifier only.
        // Ctrl+Alt is deliberately treated as Ctrl-only.
        97..=122 if ctrl => Key::Ctrl((keycode as u8) as char),
        // Alt+letter: keycode 97..=122 (a-z) with alt modifier (no ctrl).
        97..=122 if alt => Key::Alt((keycode as u8) as char),
        // Plain printable ASCII (keycode 32..=126) with no modifier.
        32..=126 if !ctrl && !alt && !shift => Key::Char(char::from(keycode as u8)),
        _ => Key::Unknown(params.to_vec()),
    }
}

fn effective_kitty_keycode(primary: u32, base_layout: Option<u32>) -> u32 {
    let is_latin_letter = (b'a' as u32..=b'z' as u32).contains(&primary)
        || (b'A' as u32..=b'Z' as u32).contains(&primary);
    let is_digit = (b'0' as u32..=b'9' as u32).contains(&primary);
    let is_known_symbol = matches!(primary, 32 | 45 | 47 | 59 | 91 | 92 | 93 | 95);
    if is_latin_letter || is_digit || is_known_symbol {
        primary
    } else {
        base_layout.unwrap_or(primary)
    }
}

fn parse_modify_other_keys(params: &[u8]) -> Option<Key> {
    let s = std::str::from_utf8(params).ok()?;
    let mut parts = s.split(';');
    // xterm modifyOtherKeys mode 2: CSI 27 ; modifiers ; codepoint ~
    if parts.next()? != "27" {
        return None;
    }
    let modifiers = parts.next()?.parse::<u32>().ok()?;
    let codepoint = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let mod_bits = modifiers.saturating_sub(1);
    let shift = mod_bits & 1 != 0;
    let alt = mod_bits & 2 != 0;
    let ctrl = mod_bits & 4 != 0;

    match codepoint {
        65..=90 if ctrl && shift => Some(Key::CtrlShift(((codepoint as u8) + b'a' - b'A') as char)),
        97..=122 if ctrl && shift => Some(Key::CtrlShift((codepoint as u8) as char)),
        65..=90 if ctrl => Some(Key::Ctrl(((codepoint as u8) + b'a' - b'A') as char)),
        97..=122 if ctrl => Some(Key::Ctrl((codepoint as u8) as char)),
        65..=90 if alt => Some(Key::Alt(((codepoint as u8) + b'a' - b'A') as char)),
        97..=122 if alt => Some(Key::Alt((codepoint as u8) as char)),
        32..=126 if !ctrl && !alt && !shift => Some(Key::Char(char::from(codepoint as u8))),
        _ => None,
    }
}

/// Parse bracketed paste content. Input starts after `\x1b[200~`.
fn parse_bracketed_paste(rest: &[u8]) -> Option<(Key, usize)> {
    // Search for the end marker: \x1b[201~
    let end_marker = b"\x1b[201~";
    if let Some(pos) = find_subsequence(rest, end_marker) {
        let content = String::from_utf8_lossy(&rest[..pos]).replace("\r\n", "\n");
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
#[path = "keys_tests.rs"]
mod tests;
