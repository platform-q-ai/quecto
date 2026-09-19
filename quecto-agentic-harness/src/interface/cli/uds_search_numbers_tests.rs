use super::number;
use serde_json::json;

#[test]
fn a_limit_is_any_number_brought_into_range_and_only_a_number_is_a_number() {
    assert_eq!(number(&json!(null), false), Ok(None));
    assert_eq!(number(&json!(0), false), Ok(Some(0)));
    assert_eq!(number(&json!(u64::MAX), false), Ok(Some(u64::MAX)));
    assert_eq!(number(&json!(-1), false), Ok(Some(0)));
    assert_eq!(number(&json!(i64::MIN), false), Ok(Some(0)));
    assert_eq!(number(&json!(1.5), false), Ok(Some(1)));
    assert_eq!(number(&json!(-0.5), false), Ok(Some(0)));
    assert_eq!(number(&json!(1e300), false), Ok(Some(u64::MAX)));
    assert_eq!(number(&json!(f64::MIN), false), Ok(Some(0)));
    let beyond: serde_json::Value = serde_json::from_str("18446744073709551616").unwrap();
    assert_eq!(number(&beyond, false), Ok(Some(u64::MAX)));
    for other in [json!("7"), json!(true), json!([1]), json!({"n": 1})] {
        assert_eq!(number(&other, false), Err(()), "{other}");
        assert_eq!(number(&other, true), Err(()), "{other}");
    }
}

/// R2-H10: a generation is echoed exactly or refused — never rounded.
#[test]
fn a_generation_is_an_exact_integer_or_nothing() {
    assert_eq!(number(&json!(null), true), Ok(None));
    for exact in [0, 1, (1 << 53) + 1, u64::MAX] {
        assert_eq!(number(&json!(exact), true), Ok(Some(exact)));
    }
    let beyond: serde_json::Value = serde_json::from_str("18446744073709551616").unwrap();
    let minus_zero: serde_json::Value = serde_json::from_str("-0").unwrap();
    for inexact in [
        json!(-1),
        json!(7.0),
        json!(7.9),
        json!(9007199254740993.0),
        beyond,
        minus_zero,
    ] {
        assert_eq!(number(&inexact, true), Err(()), "{inexact}");
    }
}
