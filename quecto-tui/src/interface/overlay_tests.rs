use super::*;

#[test]
fn splice_line_basic() {
    let result = splice_line("AAAAAAAAAA", "XX", 3, 2, 10);
    let plain: String = result.chars().filter(|c| !c.is_control()).collect();
    assert!(plain.contains("XX"), "should contain overlay: {}", plain);
}

#[test]
fn take_visible_chars_basic() {
    assert_eq!(take_visible_chars("hello world", 5), "hello");
}

#[test]
fn take_visible_chars_with_ansi() {
    let s = "\x1b[31mhello\x1b[0m world";
    let result = take_visible_chars(s, 5);
    assert!(result.contains("hello"));
}

#[test]
fn skip_visible_chars_basic() {
    assert_eq!(skip_visible_chars("hello world", 6), "world");
}

#[test]
fn splice_line_preserves_surrounding_content() {
    // Overlay 2 chars at col 3, width 10 → chars 0-2, overlay, chars 5+
    let result = splice_line("AAAAAAAAAA", "XX", 3, 2, 10);
    let plain: String = result.chars().filter(|c| !c.is_control()).collect();
    assert!(
        plain.starts_with("AAA"),
        "prefix should be preserved: {plain}"
    );
    assert!(plain.contains("XX"), "overlay should appear: {plain}");
    assert!(
        plain.ends_with("AAAAA"),
        "suffix should be preserved: {plain}"
    );
}

#[test]
fn splice_line_with_ansi_base() {
    let base = "\x1b[31mAAAAAAAAAA\x1b[0m";
    let result = splice_line(base, "XX", 3, 2, 10);
    let plain: String = result.chars().filter(|c| !c.is_control()).collect();
    assert!(
        plain.contains("XX"),
        "overlay should appear through ANSI: {plain}"
    );
}
