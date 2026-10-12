//! Epic #2481 T1: a leaf holds only a kind, or a redacted, capped slug.
use super::*;

#[test]
fn a_kind_is_bounded_snake_case_only() {
    for kind in ["a", "not_holder", "x9", &"k".repeat(MAX_KIND_BYTES)] {
        assert!(kind_shaped(kind), "{kind}");
    }
    let long = "k".repeat(MAX_KIND_BYTES + 1);
    for text in [
        "", "Not", "a b", "a-b", "a.b", "é", "sk-ant-x", "a@b.c", &long,
    ] {
        assert!(!kind_shaped(text), "{text:?}");
    }
}

#[test]
fn a_task_ref_is_its_slug_redacted_then_capped() {
    let long = "a".repeat(MAX_TASK_REF_CHARS);
    assert_eq!(TaskRef::of(&Slug::parse(&long).unwrap()).as_str(), long);
    let secret = TaskRef::of(&Slug::parse("sk-ant-api03-abcdefghijklmnopqrstuvwx").unwrap());
    assert!(
        !secret.as_str().contains("abcdefghijklmnop"),
        "{}",
        secret.as_str()
    );
    assert!(
        secret.as_str().contains("[REDACTED]"),
        "{}",
        secret.as_str()
    );
}

/// A kind is one the code knows (#2532 round-3 review): a caller's text
/// finds the table's entry, whose `&'static str` the kind holds.
#[test]
fn a_static_kind_is_a_known_kind_or_other() {
    let unknown = [
        "not_a_known_kind",
        "fix_index_rebuild",
        "hunter2",
        "Not a kind",
        "",
    ];
    for text in unknown {
        assert_eq!(StaticKind::new(text), StaticKind::OTHER, "{text:?}");
    }
    let sent = String::from("stale");
    let kind = StaticKind::new(&sent);
    assert_eq!(kind.as_str(), "stale");
}
