use super::*;

#[test]
fn names_of_letters_digits_dash_underscore_and_dot_are_safe() {
    for name in [
        "env",
        "review-env",
        "tool_env",
        "l1-red-2359",
        "v1.2",
        "A",
        "9lives",
    ] {
        assert!(is_safe_environment_name(name), "{name}");
    }
}

#[test]
fn a_name_must_start_with_a_letter_or_digit() {
    for name in ["-env", "_env", ".env", ".."] {
        assert!(!is_safe_environment_name(name), "{name}");
    }
}

#[test]
fn whitespace_control_and_other_characters_are_refused() {
    for name in [
        "my env", "tab\tenv", "lf\nenv", "cr\renv", "nul\0env", "a/b", "a:b", "é", "a\u{7f}",
        "C1 C2",
    ] {
        assert!(!is_safe_environment_name(name), "{name:?}");
    }
}

#[test]
fn a_name_is_one_to_sixty_four_bytes() {
    assert!(!is_safe_environment_name(""));
    assert!(is_safe_environment_name(
        &"a".repeat(MAX_ENVIRONMENT_NAME_BYTES)
    ));
    assert!(!is_safe_environment_name(
        &"a".repeat(MAX_ENVIRONMENT_NAME_BYTES + 1)
    ));
    assert_eq!(MAX_ENVIRONMENT_NAME_BYTES, 64);
}
