use serde_json::json;

use super::super::decode;
use super::*;

fn loads(text: &str) -> PyJson {
    decode(text).expect("test input is JSON Python reads")
}

#[test]
fn a_serde_value_converts_losslessly_and_keeps_its_order() {
    let value = json!({"z": [1, -2, 1.5, 18_446_744_073_709_551_615_u64], "a": {"s": "é", "n": null, "t": true}});

    let converted = PyJson::try_from(&value).expect("converts");

    assert!(
        converted
            == loads(r#"{"z":[1,-2,1.5,18446744073709551615],"a":{"s":"é","n":null,"t":true}}"#)
    );
    assert_eq!(converted.to_value().expect("back to serde"), value);
    assert_eq!(
        super::super::dumps(&converted).expect("dumpable"),
        r#"{"z": [1, -2, 1.5, 18446744073709551615], "a": {"s": "\u00e9", "n": null, "t": true}}"#
    );
}

#[test]
fn an_integral_serde_float_stays_a_float() {
    let converted = PyJson::try_from(&json!(1.0)).expect("converts");

    assert!(matches!(converted, PyJson::Float(value) if value.to_bits() == 1.0_f64.to_bits()));
}

#[test]
fn to_value_refuses_what_serde_cannot_hold() {
    for text in [
        "NaN",
        "Infinity",
        "-Infinity",
        "18446744073709551616",
        "-9223372036854775809",
        r#""\ud800""#,
    ] {
        assert!(
            matches!(
                loads(text).to_value(),
                Err(PyJsonError::NotRepresentable(_))
            ),
            "{text} is refused"
        );
    }
}

#[test]
fn conversions_refuse_nesting_beyond_serde_depth() {
    let deepest = format!(
        "{}{}",
        "[".repeat(SERDE_MAX_DEPTH),
        "]".repeat(SERDE_MAX_DEPTH)
    );
    let too_deep = format!("[{deepest}]");

    assert!(loads(&deepest).to_value().is_ok());
    assert!(matches!(
        loads(&too_deep).to_value(),
        Err(PyJsonError::NotRepresentable(_))
    ));

    let mut value = json!([]);
    for _ in 0..SERDE_MAX_DEPTH {
        value = json!([value]);
    }
    assert!(matches!(
        PyJson::try_from(&value),
        Err(PyJsonError::NotRepresentable(_))
    ));
}

#[test]
fn equality_is_structural_identity() {
    assert!(
        loads(r#"{"a":1,"b":2}"#) == loads(r#"{"b":2,"a":1}"#),
        "object order is ignored"
    );
    assert!(loads("5") != loads("5.0"), "an int is not a float");
    assert!(loads("0.0") != loads("-0.0"), "floats compare by bits");
    assert!(loads("NaN") == loads("NaN"), "NaN is itself");
    assert!(loads("[1,2]") != loads("[1,2,3]"));
    assert!(loads(r#"{"a":1}"#) != loads(r#"{"b":1}"#));
    assert!(loads(r#"["\ud800"]"#) == loads(r#"["\ud800"]"#));
}

#[test]
fn py_int_parse_is_canonical_and_refuses_non_json_integers() {
    assert_eq!(PyInt::parse("-0").expect("int").as_str(), "0");
    assert_eq!(PyInt::parse("-12").expect("int").as_i64(), Some(-12));
    assert_eq!(
        PyInt::parse("18446744073709551615").expect("int").as_u64(),
        Some(u64::MAX)
    );
    for text in ["", "-", "01", "+1", "1.0", "1e3", " 1", "--1"] {
        assert_eq!(
            PyInt::parse(text),
            Err(PyJsonError::InvalidInteger(text.to_owned())),
            "{text:?}"
        );
    }
}

#[test]
fn py_str_keeps_lone_surrogates_and_orders_by_code_point() {
    let lone = PyStr::from_code_points(vec![0x61, 0xD800]).expect("code points");
    let plain = PyStr::from_code_points(vec![0x61, 0xE9]).expect("code points");

    assert_eq!(lone.as_str(), None);
    assert_eq!(plain.as_str(), Some("aé"));
    assert_eq!(plain, PyStr::from("aé"));
    assert!(plain < lone, "U+00E9 sorts before U+D800");
    assert!(PyStr::from("\u{ffff}") < PyStr::from("\u{1f600}"));
    assert_eq!(
        PyStr::from_code_points(vec![0x11_0000]),
        Err(PyJsonError::InvalidCodePoint(0x11_0000))
    );
}

#[test]
fn a_dict_insert_replaces_in_place() {
    let mut object = PyObject::new();
    assert!(object.insert(PyStr::from("b"), PyJson::Null).is_none());
    assert!(
        object
            .insert(PyStr::from("a"), PyJson::Bool(true))
            .is_none()
    );
    assert!(
        object
            .insert(PyStr::from("b"), PyJson::Bool(false))
            .is_some()
    );

    let keys: Vec<Option<&str>> = object.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, [Some("b"), Some("a")]);
    assert!(matches!(
        object.get(&PyStr::from("b")),
        Some(PyJson::Bool(false))
    ));
    assert_eq!(object.len(), 2);
    assert!(!object.is_empty());
}

#[test]
fn a_very_deep_value_drops_and_compares_without_overflowing_the_stack() {
    let build = || {
        let mut value = PyJson::List(Vec::new());
        for _ in 0..200_000 {
            let mut object = PyObject::new();
            object.insert(PyStr::from("k"), value);
            value = PyJson::List(vec![PyJson::Object(object)]);
        }
        value
    };

    assert!(build() == build());
}

#[test]
fn from_code_points_refuses_a_high_surrogate_followed_by_a_low_one() {
    // Written out, D83D DE00 reads back as U+1F600, so a PyStr holding the
    // pair as two lone surrogates would not survive the board, and two keys
    // [D83D, DE00] and "\u{1f600}" would collide. `decode` never makes one.
    assert_eq!(
        PyStr::from_code_points(vec![0x61, 0xD83D, 0xDE00]),
        Err(PyJsonError::JoinableSurrogates { index: 1 })
    );
    assert!(PyStr::from_code_points(vec![0xDE00, 0xD83D]).is_ok());
    assert!(PyStr::from_code_points(vec![0xD800, 0xD800]).is_ok());
    assert!(PyStr::from_code_points(vec![0xDC00, 0xDC00]).is_ok());
}

#[test]
fn text_strings_order_by_code_point_as_python_orders_str() {
    // python3: sorted(['é', 'z', 'ab', 'a', '\U0001f600', '￿', 'Z'])
    let mut keys: Vec<PyStr> = ["é", "z", "ab", "a", "\u{1f600}", "\u{ffff}", "Z"]
        .into_iter()
        .map(PyStr::from)
        .collect();
    keys.sort();

    let sorted: Vec<Option<&str>> = keys.iter().map(PyStr::as_str).collect();
    assert_eq!(
        sorted,
        [
            Some("Z"),
            Some("a"),
            Some("ab"),
            Some("z"),
            Some("é"),
            Some("\u{ffff}"),
            Some("\u{1f600}")
        ]
    );
    assert_eq!(PyStr::from("a").cmp(&PyStr::from("a")), Ordering::Equal);
}

#[test]
fn an_object_is_empty_only_without_entries() {
    let mut object = PyObject::new();
    assert!(object.is_empty());

    object.insert(PyStr::from("k"), PyJson::Null);
    assert!(!object.is_empty());
}

#[test]
fn a_float_equals_another_only_by_bits_or_when_both_are_nan() {
    assert!(loads("NaN") != loads("1.0"), "NaN is not a number");
    assert!(loads("1.0") != loads("NaN"), "a number is not NaN");
    assert!(loads("1.5") == loads("1.5"));
}

#[test]
fn debug_prints_the_dumps_text() {
    // python3: json.dumps({'a': [1, {}], 'b': 'x'})
    let value = loads(r#"{"a":[1,{}],"b":"x"}"#);

    assert_eq!(format!("{value:?}"), r#"{"a": [1, {}], "b": "x"}"#);
}
