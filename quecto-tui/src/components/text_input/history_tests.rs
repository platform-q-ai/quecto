//! Unit tests for [`super::InputHistory`].

use super::*;

#[test]
fn push_ignores_empty_and_resets_index() {
    let mut h = InputHistory::new();
    h.push("a");
    let _ = h.navigate_up("");
    h.push("");
    assert_eq!(h.index, -1);
    assert_eq!(h.entries, vec!["a".to_string()]);
}

#[test]
fn push_skips_duplicate_last() {
    let mut h = InputHistory::new();
    h.push("a");
    h.push("a");
    h.push("b");
    assert_eq!(h.entries, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn push_caps_at_max() {
    let mut h = InputHistory::new();
    for i in 0..MAX_HISTORY + 1 {
        h.push(&format!("e{i}"));
    }
    assert_eq!(h.entries.len(), MAX_HISTORY);
    assert_eq!(h.entries[0], "e1");
    assert_eq!(h.entries[MAX_HISTORY - 1], format!("e{MAX_HISTORY}"));
}
