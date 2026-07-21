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
/// Kitty release events encode event type `3` in a colon-delimited sub-field
/// after the modifier parameter, e.g. `CSI 97 ; 1 : 3 u`. The parser is
/// anchored to a single CSI sequence so unrelated text containing `:3u` is not
/// treated as a release.
pub fn is_key_release(input: &[u8]) -> bool {
    let Some(rest) = input.strip_prefix(b"\x1b[") else {
        return false;
    };
    let Some((&terminator, params)) = rest.split_last() else {
        return false;
    };
    if !matches!(
        terminator,
        b'u' | b'~' | b'A' | b'B' | b'C' | b'D' | b'H' | b'F'
    ) {
        return false;
    }
    let Ok(params) = std::str::from_utf8(params) else {
        return false;
    };
    params
        .split(';')
        .skip(1)
        .filter_map(|field| field.split(':').next_back())
        .any(|event_type| event_type == "3")
}

#[cfg(test)]
#[path = "kitty_tests.rs"]
mod tests;
