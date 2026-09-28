use serde_json::{Value, json};

use super::*;

/// Parses `text` with serde_json, keeping its insertion order.
fn parsed(text: &str) -> Value {
    serde_json::from_str(text).expect("test input is JSON")
}

#[test]
fn encode_sorts_keys_recursively_and_is_compact() {
    let value = parsed(r#"{"z":1,"a":{"y":[1,{"b":2,"a":1}],"x":null}}"#);

    assert_eq!(
        encode(&value),
        r#"{"a":{"x":null,"y":[1,{"a":1,"b":2}]},"z":1}"#
    );
}

#[test]
fn encode_sorts_keys_by_code_point_like_python() {
    // Python sorts `str` keys by code point; "é" (U+00E9) sorts after "z".
    let value = parsed(r#"{"é":2,"z":1,"a":0}"#);

    assert_eq!(encode(&value), r#"{"a":0,"z":1,"\u00e9":2}"#);
}

#[test]
fn dumps_keeps_insertion_order_with_python_separators() {
    let value = parsed(r#"{"z":1,"a":[1,2,{"k":"v"}],"m":{},"n":[],"t":true,"f":false,"x":null}"#);

    assert_eq!(
        dumps(&value),
        r#"{"z": 1, "a": [1, 2, {"k": "v"}], "m": {}, "n": [], "t": true, "f": false, "x": null}"#
    );
}

#[test]
fn ensure_ascii_escapes_control_del_and_non_ascii() {
    let value = Value::String("\u{7f} é 😀 \u{1} \" \\ \n\r\t\u{8}\u{c} \u{1f} ~".to_owned());
    let expected = r#""\u007f \u00e9 \ud83d\ude00 \u0001 \" \\ \n\r\t\b\f \u001f ~""#;

    assert_eq!(encode(&value), expected);
    assert_eq!(dumps(&value), expected);
}

#[test]
fn keys_are_escaped_like_values() {
    let value = json!({"é\n": "😀"});

    assert_eq!(encode(&value), r#"{"\u00e9\n":"\ud83d\ude00"}"#);
    assert_eq!(dumps(&value), r#"{"\u00e9\n": "\ud83d\ude00"}"#);
}

#[test]
fn floats_print_as_python_repr() {
    let table: [(f64, &str); 18] = [
        (0.1, "0.1"),
        (1.0, "1.0"),
        (1e16, "1e+16"),
        (1e-05, "1e-05"),
        (1.5e-7, "1.5e-07"),
        (1_790_000_000.123_456, "1790000000.123456"),
        (123_456_789_012_345_680.0, "1.2345678901234568e+17"),
        (-0.0, "-0.0"),
        (0.0, "0.0"),
        (5e-324, "5e-324"),
        (0.0001, "0.0001"),
        (1e22, "1e+22"),
        (-2.5, "-2.5"),
        (1e15, "1000000000000000.0"),
        (123.0, "123.0"),
        (9_999_999_999_999_998.0, "9999999999999998.0"),
        (f64::MAX, "1.7976931348623157e+308"),
        (-1.5e-7, "-1.5e-07"),
    ];

    for (value, expected) in table {
        assert_eq!(float_repr(value), expected, "repr of {value:e}");
        let number = Value::from(value);
        assert_eq!(encode(&number), expected, "encode of {value:e}");
        assert_eq!(dumps(&number), expected, "dumps of {value:e}");
    }
}

#[test]
fn non_finite_floats_print_as_python_json_tokens() {
    // A `Value` cannot hold these, but `float_repr` is Python's `json`
    // float writer, which spells them this way.
    assert_eq!(float_repr(f64::INFINITY), "Infinity");
    assert_eq!(float_repr(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(float_repr(f64::NAN), "NaN");
}

#[test]
fn integers_print_as_is_up_to_the_u64_range() {
    let value = json!([0, -1, i64::MAX, i64::MIN, u64::MAX]);

    assert_eq!(
        encode(&value),
        "[0,-1,9223372036854775807,-9223372036854775808,18446744073709551615]"
    );
}

#[test]
fn an_integral_float_from_text_stays_a_float_end_to_end() {
    let value = decode(r#"{"a":1.0,"b":1}"#).expect("JSON");

    assert!(value["a"].is_f64(), "1.0 decodes as a float");
    assert!(value["b"].is_i64(), "1 decodes as an integer");
    assert_eq!(encode(&value), r#"{"a":1.0,"b":1}"#);
    assert_eq!(dumps(&value), r#"{"a": 1.0, "b": 1}"#);
}

#[test]
fn empty_containers_and_scalars_render_like_python() {
    assert_eq!(encode(&json!({})), "{}");
    assert_eq!(dumps(&json!([])), "[]");
    assert_eq!(dumps(&json!(null)), "null");
    assert_eq!(encode(&json!(true)), "true");
    assert_eq!(dumps(&json!([[], {}])), "[[], {}]");
}

#[test]
fn decode_reads_what_encode_and_dumps_write() {
    let value = json!({"b": [1.5, "é"], "a": {"c": null}});

    assert_eq!(decode(&encode(&value)).expect("encode output"), value);
    assert_eq!(decode(&dumps(&value)).expect("dumps output"), value);
}

#[test]
fn decode_refuses_the_non_finite_tokens_python_accepts() {
    // Documented divergence: `json.loads` accepts these, serde does not.
    // The board never writes them.
    assert!(decode("NaN").is_err());
    assert!(decode("Infinity").is_err());
}
