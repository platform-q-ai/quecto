use super::*;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "only the parse failure is under test")]
struct Shape {
    key: String,
    list: Vec<u8>,
    #[serde(default)]
    mode: Option<Mode>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Token,
    Oauth,
}

#[test]
fn a_wrong_typed_field_is_named_by_path_and_never_quoted() {
    let error = from_str::<Shape>(r#"{"key": 12345678, "list": []}"#).unwrap_err();
    assert_eq!(
        error,
        "a field has the wrong type or value at key (line 1 column 16)"
    );
    let error = from_value::<Shape>(serde_json::json!({"key": "k", "list": [1, "fake-secret"]}))
        .unwrap_err();
    assert_eq!(error, "a field has the wrong type or value at list[1]");
}

#[test]
fn a_missing_or_duplicate_field_is_named_from_the_schema() {
    assert_eq!(
        from_value::<Shape>(serde_json::json!({"list": []})).unwrap_err(),
        "missing field `key`"
    );
    assert_eq!(
        from_str::<Shape>(r#"{"key": "a", "key": "b", "list": []}"#).unwrap_err(),
        "duplicate field `key` at line 1 column 18"
    );
}

#[test]
fn an_unknown_field_is_placed_never_named_by_its_own_words() {
    let error = from_value::<Shape>(serde_json::json!({"key": "k", "list": [], "sk-FAKE-KEY": 1}))
        .unwrap_err();
    assert_eq!(error, "unknown field at sk-FAKE-KEY");
    assert!(!error.contains("expected"), "{error}");
}

#[test]
fn an_unknown_variant_lists_the_schemas_values_never_the_files() {
    let error = from_value::<Shape>(
        serde_json::json!({"key": "k", "list": [], "mode": "sk-FAKE-MODE`, expected nothing"}),
    )
    .unwrap_err();
    assert_eq!(
        error,
        "unknown variant, expected `token` or `oauth` at mode"
    );
}

#[test]
fn a_key_is_printed_escaped_and_cut() {
    let hostile = format!("\u{1b}[31m{}", "x".repeat(200));
    let error = from_value::<std::collections::BTreeMap<String, u8>>(
        serde_json::json!({ hostile: "not a number" }),
    )
    .unwrap_err();
    assert!(!error.contains('\u{1b}'), "{error:?}");
    assert!(error.contains("\\u{1b}[31m"), "{error}");
    assert!(error.ends_with('…'), "{error}");
    assert!(error.chars().count() < 120, "{error}");
}

#[test]
fn a_root_of_the_wrong_type_is_placed_by_position() {
    assert_eq!(
        from_str::<Shape>("[]").unwrap_err(),
        "a field has the wrong type or value at line 1 column 2"
    );
}

#[test]
fn malformed_and_truncated_text_is_placed_by_line_and_column() {
    assert_eq!(
        from_str::<Shape>("{ nope").unwrap_err(),
        "malformed JSON at line 1 column 3"
    );
    assert_eq!(
        from_str::<Shape>("{\"key\": \"k\"").unwrap_err(),
        "the file ends early at line 1 column 11"
    );
    assert_eq!(
        from_str::<Shape>("{\"key\": \"k\", \"list\": []} trailing").unwrap_err(),
        "malformed JSON at line 1 column 26"
    );
}

/// One wording for a malformed document (#2617 review): the configuration
/// readers' describer and this helper say the same.
#[test]
fn the_configuration_readers_word_a_malformed_document_the_same() {
    use crate::application::configuration::services::document_parse::parse_document;
    for text in [
        "{ nope",
        "{\"a\": ",
        "[1, 2",
        "{\"a\": 1} trailing",
        "\"unterminated",
    ] {
        assert_eq!(
            parse_document(text.as_bytes()).unwrap_err(),
            describe(&serde_json::from_str::<serde_json::Value>(text).unwrap_err()),
            "{text}"
        );
    }
}

/// serde ends a parse-of-text message with its own " at line L column C";
/// the description places the position once, after the field.
#[test]
fn an_unknown_variant_in_text_is_placed_once() {
    let error = from_str::<Shape>(r#"{"key": "k", "list": [], "mode": "sk-FAKE"}"#).unwrap_err();
    assert_eq!(
        error,
        "unknown variant, expected `token` or `oauth` at mode (line 1 column 42)"
    );
}
