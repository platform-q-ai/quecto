//! Text read from a file, as a diagnostic shows it (#2617): control
//! characters escaped, so a name can neither forge a line nor reach the
//! terminal as an escape sequence. One escaper for every layer that names
//! what a configuration, template or registry file holds.

/// The longest a name is shown; a longer one is cut and ends in `…`.
pub const NAME_CHARS: usize = 64;

/// `text` printable whole, every control character (a newline, an ANSI
/// escape) escaped.
pub fn printable(text: &str) -> String {
    let shown = text.escape_debug().to_string();
    debug_assert!(!shown.contains(char::is_control), "escaped: {shown:?}");
    shown
}

/// `text` printable, as [`printable`], and cut to [`NAME_CHARS`] escaped
/// characters: a key or a name as a diagnostic names it.
pub fn printable_name(text: &str) -> String {
    let mut out: String = text
        .chars()
        .flat_map(char::escape_debug)
        .take(NAME_CHARS)
        .collect();
    if text
        .chars()
        .flat_map(char::escape_debug)
        .nth(NAME_CHARS)
        .is_some()
    {
        out.push('…');
    }
    out
}

#[cfg(test)]
#[path = "printable_tests.rs"]
mod tests;
