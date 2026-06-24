//! Shared display sanitizers for terminal-rendered component text.
//!
//! Agent- and model-sourced strings may contain ANSI/OSC escape sequences or
//! other control characters. Components should run untrusted text through these
//! helpers before placing it in the terminal frame.

/// Strip ANSI/OSC escape sequences and control characters, preserving normal
/// printable Unicode text.
pub fn strip_terminal_control(s: &str) -> String {
    crate::interface::ansi::sanitize_control(s)
}

/// Strip terminal control while preserving `\n` for markdown source parsing.
pub fn strip_terminal_control_preserve_newlines(s: &str) -> String {
    crate::interface::ansi::sanitize_control_keep_newlines(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_control_chars() {
        assert_eq!(strip_terminal_control("a\x00b\x7fc"), "abc");
    }

    #[test]
    fn strips_c1_control_chars() {
        assert_eq!(strip_terminal_control("a\u{009B}31mb\u{009D}c"), "a31mbc");
    }

    #[test]
    fn strips_csi_sequences() {
        assert_eq!(strip_terminal_control("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strips_osc_sequences() {
        assert_eq!(
            strip_terminal_control("\x1b]8;;url\x07link\x1b]8;;\x07"),
            "link"
        );
    }

    #[test]
    fn can_preserve_newlines() {
        assert_eq!(strip_terminal_control_preserve_newlines("a\nb\x07"), "a\nb");
    }
}
