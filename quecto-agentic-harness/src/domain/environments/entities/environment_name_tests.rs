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

/// A name shaped like a ref would be read as the ref by `quecto container
/// kill <ref|name>`: refused. Only `C` followed by digits is a ref.
#[test]
fn a_ref_shaped_name_is_refused() {
    // Judged by shape, so a number too large to parse is a ref too.
    for name in ["C7", "C07", "C0", "C123456", "C99999999999999999999"] {
        assert!(!is_safe_environment_name(name), "{name}");
    }
    for name in ["c7", "C7a", "C", "C-7", "CC7", "7C"] {
        assert!(is_safe_environment_name(name), "{name}");
    }
}

#[test]
fn an_environment_name_is_built_only_from_a_safe_name() {
    let name = EnvironmentName::try_from("review-env".to_string()).unwrap();
    assert_eq!(name.as_str(), "review-env");
    assert_eq!(String::from(name), "review-env");
    for unsafe_name in ["", "a b", "lf\nx", "C7", &"a".repeat(65)] {
        let refusal = EnvironmentName::try_from(unsafe_name.to_string()).unwrap_err();
        assert_eq!(refusal, UnsafeEnvironmentName);
    }
}

#[test]
fn the_refusal_states_the_rule() {
    assert_eq!(
        UnsafeEnvironmentName.to_string(),
        "must be 1-64 characters, each an ASCII letter, digit, '-', '_' or '.', starting with an ASCII letter or digit, and not shaped like a ref (C followed by digits)"
    );
}
