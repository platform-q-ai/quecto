//! Shared display sanitizers for terminal-rendered component text.
//!
//! Agent- and model-sourced strings may contain ANSI/OSC escape sequences or
//! other control characters. Components should run untrusted text through these
//! helpers before placing it in the terminal frame.

/// Strip ANSI/OSC escape sequences and control characters, preserving normal
/// printable Unicode text.
pub fn strip_terminal_control(s: &str) -> String {
    strip_terminal_control_inner(s, false)
}

/// Strip terminal control while preserving `\n` for markdown source parsing.
pub fn strip_terminal_control_preserve_newlines(s: &str) -> String {
    strip_terminal_control_inner(s, true)
}

fn strip_terminal_control_inner(s: &str, preserve_newlines: bool) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            consume_escape_sequence(&mut chars);
            continue;
        }

        if preserve_newlines && ch == '\n' {
            result.push(ch);
            continue;
        }

        if !ch.is_control() {
            result.push(ch);
        }
    }

    result
}

fn consume_escape_sequence<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = char>,
{
    match chars.peek() {
        Some(&'[') => {
            chars.next();
            for c in chars.by_ref() {
                if ('\u{0040}'..='\u{007E}').contains(&c) {
                    break;
                }
            }
        }
        Some(&']') => {
            chars.next();
            loop {
                match chars.next() {
                    Some('\u{0007}') => break,
                    Some('\x1b') if chars.peek() == Some(&'\\') => {
                        chars.next();
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
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
