use super::lenient_u64;
use serde_json::json;

#[test]
fn any_number_is_brought_into_range_and_only_a_number_is_a_number() {
    assert_eq!(lenient_u64(&json!(null)), Ok(None));
    assert_eq!(lenient_u64(&json!(0)), Ok(Some(0)));
    assert_eq!(lenient_u64(&json!(u64::MAX)), Ok(Some(u64::MAX)));
    assert_eq!(lenient_u64(&json!(-1)), Ok(Some(0)));
    assert_eq!(lenient_u64(&json!(i64::MIN)), Ok(Some(0)));
    assert_eq!(lenient_u64(&json!(1.5)), Ok(Some(1)));
    assert_eq!(lenient_u64(&json!(-0.5)), Ok(Some(0)));
    assert_eq!(lenient_u64(&json!(1e300)), Ok(Some(u64::MAX)));
    let beyond: serde_json::Value = serde_json::from_str("18446744073709551616").unwrap();
    assert_eq!(lenient_u64(&beyond), Ok(Some(u64::MAX)));
    for other in [json!("7"), json!(true), json!([1]), json!({"n": 1})] {
        assert_eq!(lenient_u64(&other), Err(()), "{other}");
    }
}
