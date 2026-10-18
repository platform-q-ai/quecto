/// Truncate a string to at most `max_len` characters, appending "…" if truncated.
/// Uses char-boundary-safe slicing so multibyte UTF-8 does not panic.
pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max_len).map_or(s.len(), |(i, _)| i);
        let mut truncated = s[..end].to_string();
        truncated.push('…');
        truncated
    }
}
