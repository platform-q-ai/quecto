use super::super::{DECODE_MAX_DEPTH, PyJsonError, encode};
use super::*;

fn round_trip(text: &str) -> String {
    encode(&parse(text).expect("Python reads it")).expect("encodable")
}

fn syntax(text: &str) -> (String, usize, usize, usize) {
    match parse(text) {
        Err(PyJsonError::Syntax {
            message,
            line,
            column,
            offset,
        }) => (message, line, column, offset),
        other => panic!("{text:?} should be a syntax error, got {other:?}"),
    }
}

#[test]
fn whitespace_is_allowed_where_python_allows_it() {
    assert_eq!(
        round_trip(" \t\r\n{ \"a\" : [ 1 , 2 ] }\n "),
        r#"{"a":[1,2]}"#
    );
}

#[test]
fn escapes_read_like_python() {
    assert_eq!(
        round_trip(r#""\/\"\\\b\f\n\r\t\u00E9\uD83D\uDE00""#),
        r#""/\"\\\b\f\n\r\t\u00e9\ud83d\ude00""#
    );
}

#[test]
fn a_high_surrogate_before_a_non_low_escape_stays_lone() {
    assert_eq!(round_trip(r#""\ud800\u0041""#), r#""\ud800A""#);
    assert_eq!(
        round_trip(r#""\ud800\ud800\udc00""#),
        r#""\ud800\ud800\udc00""#
    );
}

#[test]
fn raw_non_ascii_and_del_are_read_as_is() {
    assert_eq!(
        round_trip("\"é\u{7f}\u{1f600}\""),
        r#""\u00e9\u007f\ud83d\ude00""#
    );
}

#[test]
fn syntax_errors_carry_python_messages_and_positions() {
    let cases: [(&str, &str, usize, usize, usize); 14] = [
        ("", "Expecting value", 1, 1, 0),
        (
            "[1,]",
            "Illegal trailing comma before end of array",
            1,
            3,
            2,
        ),
        (
            r#"{"a":1,}"#,
            "Illegal trailing comma before end of object",
            1,
            7,
            6,
        ),
        ("01", "Extra data", 1, 2, 1),
        ("1.", "Extra data", 1, 2, 1),
        ("1e", "Extra data", 1, 2, 1),
        ("-NaN", "Expecting value", 1, 1, 0),
        ("nul", "Expecting value", 1, 1, 0),
        ("[1 2]", "Expecting ',' delimiter", 1, 4, 3),
        (r#"{"a" 1}"#, "Expecting ':' delimiter", 1, 6, 5),
        (
            "{1:2}",
            "Expecting property name enclosed in double quotes",
            1,
            2,
            1,
        ),
        ("\"a\u{1}\"", "Invalid control character at", 1, 3, 2),
        ("\"abc", "Unterminated string starting at", 1, 1, 0),
        ("[\n\"é\",\nx]", "Expecting value", 3, 1, 7),
    ];

    for (text, message, line, column, offset) in cases {
        assert_eq!(
            syntax(text),
            (message.to_owned(), line, column, offset),
            "{text:?}"
        );
    }
}

#[test]
fn bad_escapes_and_a_bom_are_refused() {
    // Python names the escape form: a backslash, `u` and four X letters.
    let bad_unicode = format!("Invalid \\u{} escape", "X".repeat(4));
    let cases: [(&str, &str, usize); 4] = [
        (r#""\x""#, "Invalid \\escape", 1),
        (r#""\u12G4""#, &bad_unicode, 2),
        (r#""\ud800\u12G4""#, &bad_unicode, 8),
        (
            "\u{feff}1",
            "Unexpected UTF-8 BOM (decode using utf-8-sig)",
            0,
        ),
    ];

    for (text, message, offset) in cases {
        assert_eq!(
            syntax(text),
            (message.to_owned(), 1, offset + 1, offset),
            "{text:?}"
        );
    }
}

#[test]
fn nesting_counts_objects_and_arrays_alike() {
    let mixed = format!(
        "{}1{}",
        r#"[{"a":"#.repeat(DECODE_MAX_DEPTH / 2),
        "}]".repeat(DECODE_MAX_DEPTH / 2)
    );
    let too_deep = format!(
        "[{}1{}]",
        "{\"a\":".repeat(DECODE_MAX_DEPTH),
        "}".repeat(DECODE_MAX_DEPTH)
    );

    assert!(parse(&mixed).is_ok());
    assert!(matches!(parse(&too_deep), Err(PyJsonError::TooDeep { .. })));
}
