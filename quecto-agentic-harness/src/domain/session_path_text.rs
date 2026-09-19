//! How a path is spelled as text (#2010), injectively: two different paths
//! never share a spelling, so a row's `executionPath` alone tells them apart.

/// A path as text. Each byte that is no UTF-8 is spelled `\xNN` (R1-H10) — a
/// lossy `U+FFFD` would make two folders one — and a literal backslash is
/// doubled (R2-H10), so a folder really named `caf\xE9` is `caf\\xE9`, never
/// the spelling of the byte. A path without a backslash or such a byte —
/// nearly every path — is itself.
pub fn display_path(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut shown = String::new();
    for chunk in path.as_os_str().as_bytes().utf8_chunks() {
        shown.push_str(&chunk.valid().replace('\\', "\\\\"));
        for byte in chunk.invalid() {
            shown.push_str(&format!("\\x{byte:02X}"));
        }
    }
    shown
}

#[cfg(test)]
#[path = "session_path_text_tests.rs"]
mod tests;
