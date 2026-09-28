//! `bounded` and `_criteria`, messages asserted by string.
use serde_json::{Value, json};

use super::*;
use crate::domain::swarm_board::records::CriterionKind;

fn refusal<T: std::fmt::Debug>(result: Result<T, BoardError>) -> String {
    result.expect_err("validation refuses").to_string()
}

#[test]
fn bounded_counts_utf8_bytes_not_chars() {
    let four = "é".repeat(4); // 8 bytes, 4 chars
    assert_eq!(bounded_text(&four, "goal", 8), Ok(four.as_str()));
    assert_eq!(
        refusal(bounded_text(&four, "goal", 7)),
        "goal must be nonempty and at most 7 bytes"
    );
    assert_eq!(bounded(&json!(four), "goal", 8), Ok(four.as_str()));
    assert_eq!(
        refusal(bounded(&json!("€€"), "goal", 5)),
        "goal must be nonempty and at most 5 bytes"
    );
    let exact = "a".repeat(8192);
    assert_eq!(bounded_text(&exact, "goal", 8192), Ok(exact.as_str()));
    assert_eq!(
        refusal(bounded_text(&format!("{exact}a"), "goal", 8192)),
        "goal must be nonempty and at most 8192 bytes"
    );
}

#[test]
fn bounded_rejects_whitespace_only_values_as_python_strip_does() {
    let refused = "criterion id must be nonempty and at most 128 bytes";
    // Python `str.isspace` whitespace, including U+001C..U+001F that Rust's
    // `char::is_whitespace` does not count, and U+0085/U+00A0 that both count.
    for blank in [
        "",
        " ",
        "\t\n\r\x0b\x0c",
        "\u{1c}\u{1d}\u{1e}\u{1f}",
        "\u{85}",
        "\u{a0}",
        "\u{3000}\u{2028}",
    ] {
        assert_eq!(
            refusal(bounded_text(blank, "criterion id", 128)),
            refused,
            "{blank:?}"
        );
    }
    // U+200B (zero width space) is not whitespace to Python.
    assert_eq!(
        bounded_text("\u{200b}", "criterion id", 128),
        Ok("\u{200b}")
    );
    assert_eq!(bounded_text(" x ", "criterion id", 128), Ok(" x "));
    for not_text in [
        json!(null),
        json!(5),
        json!(true),
        json!(["x"]),
        json!({"x": 1}),
    ] {
        assert_eq!(
            refusal(bounded(&not_text, "criterion id", 128)),
            refused,
            "{not_text}"
        );
    }
}

fn criterion(id: Value, kind: Value, description: Value) -> Value {
    json!({"id": id, "kind": kind, "description": description})
}

#[test]
fn criteria_messages_in_python_check_order() {
    let good = criterion(json!("test"), json!("command"), json!("tests pass"));
    let long_id = "i".repeat(129);
    let long_description = "d".repeat(8193);
    let rows: Vec<(Value, usize, &str)> = vec![
        (json!(null), 2, "explicit evidence criteria required"),
        (json!([]), 2, "explicit evidence criteria required"),
        (
            json!({"id": "test"}),
            2,
            "explicit evidence criteria required",
        ),
        (json!("test"), 2, "explicit evidence criteria required"),
        (
            json!(["test"]),
            2,
            "criteria distinguish command checks from parent-reviewed requirements",
        ),
        (
            json!([{"id": "test", "description": "d"}]),
            2,
            "criteria distinguish command checks from parent-reviewed requirements",
        ),
        (
            json!([criterion(json!("test"), json!("manual"), json!("d"))]),
            2,
            "criteria distinguish command checks from parent-reviewed requirements",
        ),
        // The kind is checked before the id and description.
        (
            json!([criterion(json!(null), json!("other"), json!(null))]),
            2,
            "criteria distinguish command checks from parent-reviewed requirements",
        ),
        (
            json!([{"kind": "command", "description": "d"}]),
            2,
            "criterion id must be nonempty and at most 128 bytes",
        ),
        (
            json!([criterion(json!(" "), json!("review"), json!(null))]),
            2,
            "criterion id must be nonempty and at most 128 bytes",
        ),
        (
            json!([criterion(json!(long_id), json!("review"), json!("d"))]),
            2,
            "criterion id must be nonempty and at most 128 bytes",
        ),
        (
            json!([criterion(json!(7), json!("review"), json!("d"))]),
            2,
            "criterion id must be nonempty and at most 128 bytes",
        ),
        (
            json!([criterion(json!("test"), json!("review"), json!(""))]),
            2,
            "criterion description must be nonempty and at most 8192 bytes",
        ),
        (
            json!([criterion(
                json!("test"),
                json!("review"),
                json!(long_description)
            )]),
            2,
            "criterion description must be nonempty and at most 8192 bytes",
        ),
        // A duplicate is found only after its own id and description pass.
        (
            json!([
                good.clone(),
                criterion(json!("test"), json!("review"), json!(""))
            ]),
            2,
            "criterion description must be nonempty and at most 8192 bytes",
        ),
        (
            json!([
                good.clone(),
                criterion(json!("test"), json!("review"), json!("again"))
            ]),
            2,
            "duplicate criterion id",
        ),
        // Every entry is checked before the encoded size.
        (
            json!([good.clone(), "late"]),
            16385,
            "criteria distinguish command checks from parent-reviewed requirements",
        ),
        (
            json!([good.clone()]),
            16385,
            "criteria must be nonempty and at most 16384 bytes",
        ),
    ];
    for (value, encoded_len, expected) in rows {
        assert_eq!(refusal(criteria(&value, encoded_len)), expected, "{value}");
    }
}

#[test]
fn criteria_accepts_distinct_command_and_review_entries() {
    let value = json!([
        {"id": "test", "kind": "command", "description": "tests pass", "extra": true},
        {"id": "docs", "kind": "review", "description": "docs read well"},
    ]);
    let parsed = criteria(&value, 16384).expect("valid criteria");
    assert_eq!(
        parsed,
        vec![
            Criterion {
                id: "test".into(),
                kind: CriterionKind::Command,
                description: "tests pass".into()
            },
            Criterion {
                id: "docs".into(),
                kind: CriterionKind::Review,
                description: "docs read well".into()
            },
        ]
    );
}
