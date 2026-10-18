use super::super::{PyInt, PyObject};
use super::*;

fn written_str(text: &PyStr) -> String {
    let mut out = String::new();
    write_str(&mut out, text);
    out
}

#[test]
fn surrogates_and_supplementary_code_points_escape_as_utf16_units() {
    let text =
        PyStr::from_code_points(vec![0xDC00, 0x10_FFFF, 0x1_0000, 0xD83D]).expect("code points");

    assert_eq!(
        written_str(&text),
        r#""\udc00\udbff\udfff\ud800\udc00\ud83d""#
    );
}

#[test]
fn scientific_reads_every_ryu_layout() {
    assert_eq!(scientific("0.0"), ("0".to_owned(), 0));
    assert_eq!(scientific("0.001"), ("1".to_owned(), -3));
    assert_eq!(scientific("1.5e-7"), ("15".to_owned(), -7));
    assert_eq!(scientific("1e16"), ("1".to_owned(), 16));
    assert_eq!(scientific("123.45"), ("12345".to_owned(), 2));
    assert_eq!(scientific("1200.0"), ("12".to_owned(), 3));
}

#[test]
fn the_dumps_style_writes_nested_objects_with_python_separators() {
    let mut inner = PyObject::new();
    inner.insert(PyStr::from("y"), PyJson::Int(PyInt::from(-1_i64)));
    let mut outer = PyObject::new();
    outer.insert(PyStr::from("x"), PyJson::Object(inner));
    outer.insert(
        PyStr::from("a"),
        PyJson::List(vec![PyJson::Null, PyJson::Float(0.5)]),
    );
    let value = PyJson::Object(outer);

    assert_eq!(
        write(&value, Style::DUMPS).expect("writable"),
        r#"{"x": {"y": -1}, "a": [null, 0.5]}"#
    );
    assert_eq!(
        write(&value, Style::ENCODE).expect("writable"),
        r#"{"a":[null,0.5],"x":{"y":-1}}"#
    );
}

#[test]
fn exponential_writes_any_exponent_with_its_sign_and_two_digits() {
    // CPython: '%.1e' % 1.5, repr(1e-05), repr(1.5e16), repr(1e100)
    assert_eq!(exponential("15", 0), "1.5e+00");
    assert_eq!(exponential("1", -5), "1e-05");
    assert_eq!(exponential("15", 16), "1.5e+16");
    assert_eq!(exponential("1", 100), "1e+100");
}

#[test]
fn the_exponent_form_starts_where_python_repr_starts_it() {
    // CPython: repr of each value, either side of the -4..16 positional range
    for (value, expected) in [
        (9_999_999_999_999_998.0, "9999999999999998.0"),
        (1e16, "1e+16"),
        (-1e16, "-1e+16"),
        (0.0001, "0.0001"),
        (-0.0001, "-0.0001"),
        (0.000_099_999_999_999_999_99, "9.999999999999999e-05"),
    ] {
        assert_eq!(float_repr(value), expected, "{value:e}");
    }
}
