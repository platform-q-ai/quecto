//! Kitty keyboard protocol support.
//!
//! Queries the terminal for Kitty protocol support on startup. If supported,
//! enables enhanced key reporting for accurate modifier detection.
//! Falls back to xterm modifyOtherKeys mode 2 for tmux compatibility.

use std::io::Write;

/// Kitty protocol state.
pub struct KittyProtocol {
    /// Whether Kitty keyboard protocol is active.
    pub active: bool,
    /// Whether xterm modifyOtherKeys is active (fallback).
    pub modify_other_keys: bool,
}

impl KittyProtocol {
    pub fn new() -> Self {
        Self {
            active: false,
            modify_other_keys: false,
        }
    }

    /// Send the Kitty protocol query to the terminal.
    ///
    /// The terminal will respond with `CSI ? <flags> u` if it supports
    /// the protocol. The response must be parsed from stdin.
    pub fn query(&self) {
        // CSI ? u — query current keyboard protocol flags.
        let _ = std::io::stdout().write_all(b"\x1b[?u");
        let _ = std::io::stdout().flush();
    }

    /// Enable the Kitty keyboard protocol with full flags.
    ///
    /// Flag 1 = disambiguate escape codes
    /// Flag 2 = report event types (press/repeat/release)
    /// Flag 4 = report alternate keys (shifted key, base layout key)
    pub fn enable(&mut self) {
        // CSI > 7 u — push flags (1 | 2 | 4 = 7).
        let _ = std::io::stdout().write_all(b"\x1b[>7u");
        let _ = std::io::stdout().flush();
        self.active = true;
    }

    /// Disable the Kitty keyboard protocol (pop flags).
    pub fn disable(&mut self) {
        if self.active {
            // CSI < u — pop flags.
            let _ = std::io::stdout().write_all(b"\x1b[<u");
            let _ = std::io::stdout().flush();
            self.active = false;
        }
    }

    /// Enable xterm modifyOtherKeys mode 2 (fallback for tmux).
    pub fn enable_modify_other_keys(&mut self) {
        // CSI > 4 ; 2 m
        let _ = std::io::stdout().write_all(b"\x1b[>4;2m");
        let _ = std::io::stdout().flush();
        self.modify_other_keys = true;
    }

    /// Disable xterm modifyOtherKeys.
    pub fn disable_modify_other_keys(&mut self) {
        if self.modify_other_keys {
            let _ = std::io::stdout().write_all(b"\x1b[>4;0m");
            let _ = std::io::stdout().flush();
            self.modify_other_keys = false;
        }
    }

    /// Parse a potential Kitty protocol response from input bytes.
    ///
    /// Response format: `CSI ? <flags> u` (e.g. `\x1b[?1u`).
    /// Returns `Some(flags)` if a response was found, `None` otherwise.
    pub fn parse_response(input: &[u8]) -> Option<u32> {
        // Look for \x1b[?<digits>u
        let s = std::str::from_utf8(input).ok()?;
        if let Some(start) = s.find("\x1b[?") {
            let rest = &s[start + 3..];
            if let Some(end) = rest.find('u') {
                let flags_str = &rest[..end];
                return flags_str.parse::<u32>().ok();
            }
        }
        None
    }

    /// Clean up on exit — disable all protocols.
    pub fn cleanup(&mut self) {
        self.disable();
        self.disable_modify_other_keys();
    }
}

impl Default for KittyProtocol {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a key input is a Kitty key release event.
///
/// Kitty release events have the form: `CSI <keycode> ; <modifiers> : 3 u`
/// The `:3` suffix indicates a release event.
pub fn is_key_release(input: &[u8]) -> bool {
    let s = match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // Pattern: ends with ":3u" (release event type)
    s.contains(":3u") || s.contains(":3~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_valid() {
        let input = b"\x1b[?1u";
        assert_eq!(KittyProtocol::parse_response(input), Some(1));
    }

    #[test]
    fn parse_response_flags_7() {
        let input = b"\x1b[?7u";
        assert_eq!(KittyProtocol::parse_response(input), Some(7));
    }

    #[test]
    fn parse_response_no_match() {
        let input = b"hello world";
        assert_eq!(KittyProtocol::parse_response(input), None);
    }

    #[test]
    fn parse_response_partial() {
        let input = b"\x1b[?";
        assert_eq!(KittyProtocol::parse_response(input), None);
    }

    #[test]
    fn is_key_release_true() {
        assert!(is_key_release(b"\x1b[97;1:3u")); // 'a' release
    }

    #[test]
    fn is_key_release_false() {
        assert!(!is_key_release(b"\x1b[97;1:1u")); // 'a' press
        assert!(!is_key_release(b"\x1b[A")); // arrow up (not Kitty)
    }

    #[test]
    fn new_starts_inactive() {
        let k = KittyProtocol::new();
        assert!(!k.active);
        assert!(!k.modify_other_keys);
    }

    #[test]
    fn default_same_as_new() {
        let k = KittyProtocol::default();
        assert!(!k.active);
        assert!(!k.modify_other_keys);
    }

    #[test]
    fn enable_sets_active() {
        let mut k = KittyProtocol::new();
        k.enable();
        assert!(k.active);
    }

    #[test]
    fn disable_clears_active() {
        let mut k = KittyProtocol::new();
        k.enable();
        k.disable();
        assert!(!k.active);
    }

    #[test]
    fn disable_when_inactive_is_noop() {
        let mut k = KittyProtocol::new();
        k.disable(); // should not panic
        assert!(!k.active);
    }

    #[test]
    fn enable_modify_other_keys() {
        let mut k = KittyProtocol::new();
        k.enable_modify_other_keys();
        assert!(k.modify_other_keys);
    }

    #[test]
    fn disable_modify_other_keys() {
        let mut k = KittyProtocol::new();
        k.enable_modify_other_keys();
        k.disable_modify_other_keys();
        assert!(!k.modify_other_keys);
    }

    #[test]
    fn disable_modify_other_keys_when_inactive_is_noop() {
        let mut k = KittyProtocol::new();
        k.disable_modify_other_keys(); // should not panic
        assert!(!k.modify_other_keys);
    }

    #[test]
    fn cleanup_clears_both() {
        let mut k = KittyProtocol::new();
        k.enable();
        k.enable_modify_other_keys();
        k.cleanup();
        assert!(!k.active);
        assert!(!k.modify_other_keys);
    }

    #[test]
    fn parse_response_with_prefix_noise() {
        // Response embedded in other input
        let input = b"some noise\x1b[?15u";
        assert_eq!(KittyProtocol::parse_response(input), Some(15));
    }

    #[test]
    fn parse_response_invalid_utf8() {
        let input = &[0xFF, 0xFE, 0x1b, b'[', b'?', b'1', b'u'];
        assert_eq!(KittyProtocol::parse_response(input), None);
    }

    #[test]
    fn is_key_release_tilde_variant() {
        assert!(is_key_release(b"\x1b[5;1:3~")); // PageUp release
    }

    #[test]
    fn is_key_release_invalid_utf8() {
        assert!(!is_key_release(&[0xFF, 0xFE]));
    }

    #[test]
    fn query_does_not_panic() {
        let k = KittyProtocol::new();
        k.query(); // writes to stdout — just verify no panic
    }
}
