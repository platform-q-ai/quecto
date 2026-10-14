//! #2481 M4b: a plan op's record, moved beside `recording_tests.rs` as
//! it grew past its room.
use serde_json::{Value, json};

use super::tests::{board, recorded};

/// #2481 M4b: a plan's op records the version, its task items and team
/// roles, and what the normaliser changed, by kind; a nested fault, or a
/// plan's own schema refusal, records the top field it lies under.
#[test]
fn a_plan_op_records_its_version_counts_and_normalised_kinds() {
    let board = board();
    let team = json!({"roles": [{"name": "w", "model": "m", "effort": "low", "members": 2}]});
    let lines = recorded(
        &board,
        &[
            json!({"op": "create", "task": "a", "title": "A"}).to_string(),
            json!({"op": "propose_plan", "task": "a", "approach": "Do.\r\n",
                   "items": [{"id": "i", "title": " I", "criteria": ["x\ty"]}], "team": team})
            .to_string(),
            json!({"op": "edit_plan", "task": "a", "version": 1, "team": null}).to_string(),
            json!({"op": "edit_plan", "task": "a", "version": 1, "items": [{"id": "i", "title": "Two\nlines"}]})
                .to_string(),
            json!({"op": "propose_plan", "task": "a", "approach": "", "items": [{"id": "i", "title": 7}]})
                .to_string(),
        ],
    );
    let facts = |line: &Value| {
        let field = |key: &str| line.get(key).cloned().unwrap_or(Value::Null);
        [
            "op",
            "access",
            "plan_version",
            "items",
            "roles",
            "normalised",
            "attempts",
        ]
        .map(field)
    };
    assert_eq!(
        facts(&lines[1]),
        [
            json!("propose_plan"),
            json!("write"),
            json!(1),
            json!(1),
            json!(1),
            json!({"approach": ["crlf", "edge_blank_lines"], "items": ["tab", "edge_space"]}),
            json!(1),
        ]
    );
    // An edit that gives no text tells nothing normalised.
    let edited = facts(&lines[2]);
    assert_eq!(edited[2..6], [json!(1), json!(1), json!(0), Value::Null]);
    let invalid = &lines[3];
    assert_eq!(
        [
            &invalid["code"],
            &invalid["rule"],
            &invalid["field"],
            &invalid["plan_version"]
        ],
        [
            &json!("rule"),
            &json!("invalid_plan"),
            &json!("items"),
            &json!(1)
        ],
        "{invalid}"
    );
    let wrong = &lines[4];
    assert_eq!(
        [&wrong["code"], &wrong["field"], &wrong["fault"]],
        [
            &json!("invalid_request"),
            &json!("items"),
            &json!("wrong_type")
        ],
        "{wrong}"
    );
}
