use super::display_path;

#[test]
fn a_path_that_is_text_is_itself_and_a_byte_that_is_not_is_spelled() {
    use std::os::unix::ffi::OsStrExt;
    let path = |bytes: &'static [u8]| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    assert_eq!(
        display_path(&path("/w/日本 語/é".as_bytes())),
        "/w/日本 語/é"
    );
    assert_eq!(display_path(&path(b"/w/caf\xe9/x")), "/w/caf\\xE9/x");
    assert_eq!(display_path(&path(b"\xff\xfe")), "\\xFF\\xFE");
    // A truncated multi-byte sequence: each stray byte, nothing swallowed.
    assert_eq!(display_path(&path(b"a\xe2\x80z")), "a\\xE2\\x80z");
    assert_eq!(display_path(std::path::Path::new("")), "");
    // R2-H10: injective — a folder literally named `caf\xE9` is not the byte.
    assert_eq!(display_path(&path(b"/w/caf\\xE9")), "/w/caf\\\\xE9");
    assert_ne!(
        display_path(&path(b"/w/caf\\xE9")),
        display_path(&path(b"/w/caf\xe9"))
    );
    assert_eq!(display_path(&path(b"a\\\\\xe9")), "a\\\\\\\\\\xE9");
}
