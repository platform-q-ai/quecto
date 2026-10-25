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

/// Kinds are additive (#2568 review round 2): a kind no longer emitted
/// (the start's `deadline_passed` and `deadline_too_soon`, retired when a
/// plan's deadline became seconds from Start) stays known, so records
/// written before keep their name.
#[test]
fn a_retired_kind_stays_known() {
    for retired in ["deadline_passed", "deadline_too_soon"] {
        assert_eq!(StaticKind::new(retired).as_str(), retired);
    }
}

/// The board server's clock gate (#2705) refuses as `clock_unsynced`, a
/// code its records name.
#[test]
fn the_clock_gates_refusal_is_a_known_kind() {
    assert_eq!(StaticKind::new("clock_unsynced").as_str(), "clock_unsynced");
}

/// #2663: a board id is 16 lowercase hex digits, and nothing else.
#[test]
fn a_board_id_holds_only_its_hex() {
    assert_eq!(
        BoardId::of_hex("0123456789abcdef").map(|id| id.as_str().to_string()),
        Some("0123456789abcdef".to_string())
    );
    for refused in [
        "",
        "0123456789abcde",
        "0123456789abcdef0",
        "0123456789ABCDEF",
        "0123456789abcdeg",
        "https://x.example",
    ] {
        assert_eq!(BoardId::of_hex(refused), None, "{refused:?}");
    }
}
