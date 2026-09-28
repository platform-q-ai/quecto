//! `BoardEncoding` on `PyJsonEncoding` (#2270): Python's
//! `json.dumps(v, sort_keys=True, separators=(',', ':'))`, the text the
//! board bounds and stores, so an argument's size limit is Python's.
use quecto::application::swarm::ports::BoardEncoding;
use quecto::infrastructure::persistence::swarm_board::encoding::PyJsonEncoding;
use serde_json::json;

#[test]
fn encoding_is_sorted_compact_ascii_with_python_floats() {
    let encoding: &dyn BoardEncoding = &PyJsonEncoding;
    let cases = [
        (
            json!({"b": 1, "a": [true, null]}),
            r#"{"a":[true,null],"b":1}"#,
        ),
        (json!(["é"]), r#"["\u00e9"]"#),
        (json!([1e16, 0.1, 3.0]), "[1e+16,0.1,3.0]"),
        (json!("😀"), r#""\ud83d\ude00""#),
    ];
    for (value, expected) in cases {
        assert_eq!(encoding.encode(&value).unwrap(), expected, "{value}");
    }
    // A bound is on the encoded text: `é` costs six bytes.
    assert_eq!(encoding.encode(&json!("é")).unwrap().len(), 8);
}
