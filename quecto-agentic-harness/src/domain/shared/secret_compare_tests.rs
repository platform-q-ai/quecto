use super::constant_time_eq;

#[test]
fn constant_time_eq_matches_only_identical_strings() {
    assert!(constant_time_eq("", ""));
    assert!(constant_time_eq("abc", "abc"));
    assert!(!constant_time_eq("abc", "abd"));
    assert!(!constant_time_eq("abc", "ab"));
    assert!(!constant_time_eq("ab", "abc"));
    assert!(!constant_time_eq("", "a"));
}
