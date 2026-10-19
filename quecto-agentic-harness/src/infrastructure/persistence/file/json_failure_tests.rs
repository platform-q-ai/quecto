use super::*;

#[derive(Debug, serde::Deserialize)]
#[expect(dead_code, reason = "only the parse failure is under test")]
struct Shape {
    key: String,
    list: Vec<u8>,
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
