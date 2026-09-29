use serde_json::json;

use super::PyJsonEncoding;
use crate::application::swarm::ports::BoardEncoding;

#[test]
fn encoding_is_the_boards_sorted_compact_ascii_json() {
    let encoded = PyJsonEncoding
        .encode(&json!({"b": [1, 2.5], "a": "é"}))
        .unwrap();
    assert_eq!(encoded, r#"{"a":"\u00e9","b":[1,2.5]}"#);
}

#[test]
fn a_value_too_deep_to_encode_is_refused() {
    let mut deep = json!(0);
    for _ in 0..200 {
        deep = json!([deep]);
    }
    let refused = PyJsonEncoding.encode(&deep).unwrap_err();
    assert!(refused.0.contains("nesting deeper than"), "{refused}");
}
