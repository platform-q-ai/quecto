use super::*;

#[test]
fn control_characters_are_escaped_and_nothing_is_cut() {
    assert_eq!(printable("a\nb\u{1b}[31m"), r"a\nb\u{1b}[31m");
    assert_eq!(printable(&"w".repeat(200)), "w".repeat(200));
}

#[test]
fn a_name_is_escaped_and_cut_at_its_limit() {
    let exact = "n".repeat(NAME_CHARS);
    assert_eq!(
        printable_name(&exact),
        exact,
        "a name at the limit is whole"
    );
    assert_eq!(
        printable_name(&format!("{exact}x")),
        format!("{exact}…"),
        "one past the limit is cut"
    );
    assert_eq!(printable_name("x\u{1b}[31m"), r"x\u{1b}[31m");
}
