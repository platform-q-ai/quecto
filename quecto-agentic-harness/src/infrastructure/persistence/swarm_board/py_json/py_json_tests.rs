use serde_json::json;

use super::*;

/// `json.loads(text)`.
fn loads(text: &str) -> PyJson {
    decode(text).expect("test input is JSON Python reads")
}

/// `encode(json.loads(text))`.
fn encoded(text: &str) -> String {
    encode(&loads(text)).expect("encodable")
}

/// `json.dumps(json.loads(text))`.
fn dumped(text: &str) -> String {
    dumps(&loads(text)).expect("dumpable")
}

/// A `PyJson` from a serde_json value.
fn from_value(value: &serde_json::Value) -> PyJson {
    PyJson::try_from(value).expect("a shallow serde_json value converts")
}

/// `[[...[]...]]` nested `depth` arrays deep, built without a parser.
fn nested_lists(depth: usize) -> PyJson {
    let mut value = PyJson::List(Vec::new());
    for _ in 1..depth {
        value = PyJson::List(vec![value]);
    }
    value
}

#[test]
fn encode_sorts_keys_recursively_and_is_compact() {
    assert_eq!(
        encoded(r#"{"z":1,"a":{"y":[1,{"b":2,"a":1}],"x":null}}"#),
        r#"{"a":{"x":null,"y":[1,{"a":1,"b":2}]},"z":1}"#
    );
}

#[test]
fn encode_sorts_keys_by_code_point_like_python() {
    // "é" (U+00E9) sorts after "z".
    assert_eq!(
        encoded(r#"{"é":2,"z":1,"a":0}"#),
        r#"{"a":0,"z":1,"\u00e9":2}"#
    );
}

#[test]
fn encode_sorts_keys_by_code_point_not_by_utf16_unit() {
    // U+FFFF < U+1F600 by code point, though the pair's first UTF-16 unit
    // (0xD83D) is below 0xFFFF; a lone surrogate sorts by its own value.
    assert_eq!(
        encoded(r#"{"\ud83d\ude00":2,"\uffff":1,"\ud800":3}"#),
        r#"{"\ud800":3,"\uffff":1,"\ud83d\ude00":2}"#
    );
}

#[test]
fn dumps_keeps_insertion_order_with_python_separators() {
    let text = r#"{"z":1,"a":[1,2,{"k":"v"}],"m":{},"n":[],"t":true,"f":false,"x":null}"#;

    assert_eq!(
        dumped(text),
        r#"{"z": 1, "a": [1, 2, {"k": "v"}], "m": {}, "n": [], "t": true, "f": false, "x": null}"#
    );
}

#[test]
fn ensure_ascii_escapes_control_del_and_non_ascii() {
    let value = PyJson::Str(PyStr::from(
        "\u{7f} é \u{1f600} \u{1} \" \\ \n\r\t\u{8}\u{c} \u{1f} ~",
    ));
    let expected = r#""\u007f \u00e9 \ud83d\ude00 \u0001 \" \\ \n\r\t\b\f \u001f ~""#;

    assert_eq!(encode(&value).expect("encodable"), expected);
    assert_eq!(dumps(&value).expect("dumpable"), expected);
}

#[test]
fn keys_are_escaped_like_values() {
    let value = from_value(&json!({"é\n": "\u{1f600}"}));

    assert_eq!(
        encode(&value).expect("encodable"),
        r#"{"\u00e9\n":"\ud83d\ude00"}"#
    );
    assert_eq!(
        dumps(&value).expect("dumpable"),
        r#"{"\u00e9\n": "\ud83d\ude00"}"#
    );
}

#[test]
fn lone_surrogates_survive_decode_and_re_encoding() {
    let text = r#"["\ud800","x\ude00y","\ud83d\ude00","\ude00\ud83d","\udbff"]"#;

    assert_eq!(encoded(text), text);
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
        let number = PyJson::Float(value);
        assert_eq!(encode(&number).expect("encodable"), expected);
        assert_eq!(dumps(&number).expect("dumpable"), expected);
    }
}

#[test]
fn a_tie_between_two_shortest_digit_strings_rounds_to_even_like_python() {
    // Both `…254.2` and `…254.3` round-trip to 1059438285926254.25 and lie
    // equally close; Python's `repr` keeps the even digit (std's `{:e}`
    // would write `…254.3`). Each tie is an integer plus an exact binary
    // fraction, built by addition so the literals stay within f64 precision.
    assert_eq!(
        float_repr(1_059_438_285_926_254.0 + 0.25),
        "1059438285926254.2"
    );
    assert_eq!(
        float_repr(-(1_425_502_010_969_177.0 + 0.25)),
        "-1425502010969177.2"
    );
    assert_eq!(
        float_repr(26_363_981_746_409.0 + 0.3125),
        "26363981746409.312"
    );
}

#[test]
fn non_finite_floats_read_and_write_as_python_json_tokens() {
    assert_eq!(float_repr(f64::INFINITY), "Infinity");
    assert_eq!(float_repr(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(float_repr(f64::NAN), "NaN");
    assert_eq!(
        encoded("[NaN,Infinity,-Infinity]"),
        "[NaN,Infinity,-Infinity]"
    );
    assert_eq!(
        dumped("{\"a\": NaN, \"b\": -Infinity}"),
        "{\"a\": NaN, \"b\": -Infinity}"
    );
}

#[test]
fn integers_are_kept_exactly_at_any_size() {
    let text = "[0,-1,9223372036854775807,-9223372036854775808,18446744073709551615,\
                18446744073709551616,100000000000000000000000000000,\
                -1000000000000000000000000000000000000000]";

    assert_eq!(encoded(text), text);
}

#[test]
fn an_integer_of_4300_digits_is_read_and_one_more_digit_is_refused() {
    let longest = format!("-{}", "9".repeat(INT_MAX_STR_DIGITS));
    let too_long = "9".repeat(INT_MAX_STR_DIGITS + 1);

    assert_eq!(encoded(&longest), longest);
    assert_eq!(
        decode(&too_long).err(),
        Some(PyJsonError::IntegerDigits { digits: 4301 })
    );
}

#[test]
fn negative_zero_integer_reads_as_zero() {
    assert_eq!(encoded("[-0,-0.0,0]"), "[0,-0.0,0]");
}

#[test]
fn a_float_beyond_the_f64_range_reads_as_infinity() {
    assert_eq!(encoded("[1e400,-1e400,1e-400]"), "[Infinity,-Infinity,0.0]");
}

#[test]
fn an_integral_float_from_text_stays_a_float_end_to_end() {
    let value = loads(r#"{"a":1.0,"b":1,"c":1E2}"#);

    assert_eq!(
        encode(&value).expect("encodable"),
        r#"{"a":1.0,"b":1,"c":100.0}"#
    );
    assert_eq!(
        dumps(&value).expect("dumpable"),
        r#"{"a": 1.0, "b": 1, "c": 100.0}"#
    );
}

#[test]
fn decode_reads_a_float_exactly_so_its_python_text_survives_re_encoding() {
    // serde_json's default (inexact) float parser reads this one ULP off,
    // which then re-encodes as `-3.8225971226343834e-90`.
    let text = "[-3.822597122634383e-90]";

    assert_eq!(encoded(text), text);
}

#[test]
fn a_repeated_key_keeps_its_first_position_and_its_last_value() {
    assert_eq!(dumped(r#"{"b":1,"a":2,"b":3}"#), r#"{"b": 3, "a": 2}"#);
}

#[test]
fn empty_containers_and_scalars_render_like_python() {
    assert_eq!(encoded("{}"), "{}");
    assert_eq!(dumped("[]"), "[]");
    assert_eq!(dumped("null"), "null");
    assert_eq!(encoded("true"), "true");
    assert_eq!(dumped("[[], {}]"), "[[], {}]");
}

#[test]
fn decode_reads_what_encode_and_dumps_write() {
    let value = loads(r#"{"b": [1.5, "é", NaN, 18446744073709551616], "a": {"c": null}}"#);

    let from_encode = decode(&encode(&value).expect("encodable")).expect("encode output");
    let from_dumps = decode(&dumps(&value).expect("dumpable")).expect("dumps output");

    assert!(from_encode == value, "encode output decodes to the value");
    assert!(from_dumps == value, "dumps output decodes to the value");
}

#[test]
fn decode_reads_python_depth_and_refuses_one_level_more() {
    let deepest = format!(
        "{}{}",
        "[".repeat(DECODE_MAX_DEPTH),
        "]".repeat(DECODE_MAX_DEPTH)
    );
    let too_deep = format!("[{deepest}]");

    assert!(
        decode(&deepest).is_ok(),
        "Python reads {DECODE_MAX_DEPTH} levels"
    );
    assert!(matches!(
        decode(&too_deep),
        Err(PyJsonError::TooDeep {
            limit: DECODE_MAX_DEPTH,
            ..
        })
    ));
}

#[test]
fn deep_objects_decode_without_overflowing_the_stack() {
    let depth = DECODE_MAX_DEPTH;
    let text = format!("{}1{}", r#"{"a":"#.repeat(depth), "}".repeat(depth));

    assert!(decode(&text).is_ok());
}

#[test]
fn writers_accept_python_depth_and_refuse_one_level_more() {
    let deepest = nested_lists(ENCODE_MAX_DEPTH);
    let too_deep = nested_lists(ENCODE_MAX_DEPTH + 1);

    let text = encode(&deepest).expect("Python writes this deep");
    assert_eq!(text.len(), ENCODE_MAX_DEPTH * 2);
    assert!(dumps(&deepest).is_ok());
    assert!(matches!(
        encode(&too_deep),
        Err(PyJsonError::TooDeep {
            limit: ENCODE_MAX_DEPTH,
            ..
        })
    ));
    assert!(matches!(
        dumps(&too_deep),
        Err(PyJsonError::TooDeep {
            limit: ENCODE_MAX_DEPTH,
            ..
        })
    ));
}

/// #2279: a member's text as a `Value`, refused where a `Value` cannot
/// hold what Python read.
#[test]
fn decode_value_reads_as_json_loads_or_refuses() {
    assert_eq!(
        super::decode_value(r#"{"a": -0, "b": [1E2]}"#).unwrap(),
        serde_json::json!({"a": 0, "b": [100.0]})
    );
    for text in [
        "[1e400]",
        "[NaN]",
        "[18446744073709551616]",
        "{",
        r#"["\ud800"]"#,
    ] {
        assert!(super::decode_value(text).is_err(), "{text}");
    }
}

/// #2279: an object's text field, read even when another field is
/// refused as a `Value`.
#[test]
fn object_text_reads_one_text_field() {
    let op = |text| super::object_text(text, "op");
    assert_eq!(
        op(r#"{"op": "claim", "task_id": 1e400}"#).as_deref(),
        Some("claim")
    );
    assert_eq!(op(r#"{"op": "caf\u00e9"}"#).as_deref(), Some("caf\u{e9}"));
    for text in [
        r#"{"op": 1}"#,
        r#"{"other": "claim"}"#,
        r#"["op"]"#,
        r#"{"op": "\ud800"}"#,
        "{",
    ] {
        assert_eq!(op(text), None, "{text}");
    }
}

/// #2279: a `Value` written as plain `json.dumps` writes it.
#[test]
fn dumps_value_writes_as_json_dumps() {
    let value = serde_json::json!({"z": 1e16, "a": [0.1, "\u{e9}"], "n": null});
    assert_eq!(
        super::dumps_value(&value).unwrap(),
        r#"{"z": 1e+16, "a": [0.1, "\u00e9"], "n": null}"#
    );
}
