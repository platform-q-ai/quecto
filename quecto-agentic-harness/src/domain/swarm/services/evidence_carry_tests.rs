use serde_json::json;

use super::{EvidenceCarry, evidence_carry, full_commit_id};

const BASE: &str = "db03de82dd7a7222b51a301bb9e8f0a01bada0f3";
const COMMIT: &str = "eb4fe3fd96771037e7d9162013d22036fa42d82a";
const OTHER: &str = "42b27d46167fc11e281dd005a97859b2ada5f48d";

#[test]
fn only_full_lowercase_hex_ids_are_commit_ids() {
    let sha256 = "a".repeat(64);
    for id in [BASE, sha256.as_str()] {
        assert!(full_commit_id(id), "{id}");
    }
    let refs = [
        String::new(),
        "HEAD".to_owned(),
        "main".to_owned(),
        "R1".to_owned(),
        "db03de8".to_owned(),
        BASE[..39].to_owned(),
        BASE.to_uppercase(),
        "--all".to_owned(),
        "g".repeat(40),
        "a".repeat(41),
    ];
    for text in &refs {
        assert!(!full_commit_id(text), "{text}");
    }
}

/// Every entry is carried to the verified commit, extra keys and order
/// kept; each earlier commit is named once, in order of first mention,
/// and an entry already at the commit is no source.
#[test]
fn every_entry_is_carried_to_the_verified_commit() {
    let evidence = json!([
        {"artifact": "a", "revision": BASE, "note": "é"},
        {"artifact": "b", "revision": COMMIT},
        {"artifact": "c", "revision": OTHER},
        {"artifact": "d", "revision": BASE},
    ]);
    assert_eq!(
        evidence_carry(&evidence, &json!(COMMIT)),
        Some(EvidenceCarry {
            sources: vec![BASE.to_owned(), OTHER.to_owned()],
            evidence: json!([
                {"artifact": "a", "revision": COMMIT, "note": "é"},
                {"artifact": "b", "revision": COMMIT},
                {"artifact": "c", "revision": COMMIT},
                {"artifact": "d", "revision": COMMIT},
            ]),
        })
    );
}

/// Nothing is carried unless every revision is a full commit id, the
/// evidence a nonempty list of objects, and some entry names another
/// commit.
#[test]
fn nothing_else_is_carried() {
    let good = json!([{"artifact": "a", "revision": BASE}]);
    for (evidence, revision) in [
        (good.clone(), json!("R2")),
        (good.clone(), json!("HEAD")),
        (good.clone(), json!(null)),
        (good.clone(), json!(7)),
        (json!([{"artifact": "a", "revision": "R1"}]), json!(COMMIT)),
        (json!([{"artifact": "a", "revision": 7}]), json!(COMMIT)),
        (json!([{"artifact": "a"}]), json!(COMMIT)),
        (json!([BASE]), json!(COMMIT)),
        (json!([]), json!(COMMIT)),
        (json!({"revision": BASE}), json!(COMMIT)),
        (json!(null), json!(COMMIT)),
        (
            json!([{"artifact": "a", "revision": BASE}, {"artifact": "b", "revision": "R1"}]),
            json!(COMMIT),
        ),
        (
            json!([{"artifact": "a", "revision": COMMIT}]),
            json!(COMMIT),
        ),
    ] {
        assert_eq!(
            evidence_carry(&evidence, &revision),
            None,
            "{evidence} {revision}"
        );
    }
}
