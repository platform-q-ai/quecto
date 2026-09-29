use serde_json::json;

use super::{python_equal, python_truthy};

#[test]
fn none_equals_only_none() {
    assert!(python_equal(&json!(null), &json!(null)));
    for other in [json!(0), json!(false), json!(""), json!([]), json!({})] {
        assert!(!python_equal(&json!(null), &other), "{other}");
        assert!(!python_equal(&other, &json!(null)), "{other}");
    }
}

#[test]
fn text_equals_only_the_same_text() {
    assert!(python_equal(&json!("5"), &json!("5")));
    assert!(!python_equal(&json!("5"), &json!("05")));
    assert!(!python_equal(&json!("5"), &json!(5)));
    assert!(!python_equal(&json!("1"), &json!(true)));
    assert!(!python_equal(&json!("t"), &json!(["t"])));
}

#[test]
fn numbers_compare_exactly_across_bool_int_and_float() {
    for (left, right) in [
        (json!(7), json!(7.0)),
        (json!(1), json!(true)),
        (json!(1.0), json!(true)),
        (json!(0), json!(false)),
        (json!(0), json!(-0.0)),
        (json!(u64::MAX), json!(u64::MAX)),
        (
            json!(9_007_199_254_740_992_i64),
            json!(9_007_199_254_740_992.0),
        ),
        (json!(i64::MIN), json!(-9_223_372_036_854_775_808.0)),
        // 2^63 is beyond i64 but within u64.
        (
            json!(9_223_372_036_854_775_808_u64),
            json!(9_223_372_036_854_775_808.0),
        ),
    ] {
        assert!(python_equal(&left, &right), "{left} == {right}");
        assert!(python_equal(&right, &left), "{right} == {left}");
    }
    for (left, right) in [
        (json!(7), json!(7.5)),
        (json!(7), json!(8)),
        (json!(2), json!(true)),
        (
            json!(9_007_199_254_740_993_i64),
            json!(9_007_199_254_740_992.0),
        ),
        // Round-1 review M2: 2^63 is not i64::MAX, and 2^64 is no u64.
        (json!(i64::MAX), json!(9_223_372_036_854_775_808.0)),
        (json!(u64::MAX), json!(18_446_744_073_709_551_616.0)),
        (json!(i64::MIN), json!(-18_446_744_073_709_551_616.0)),
        (json!(i64::MAX), json!(1e300)),
    ] {
        assert!(!python_equal(&left, &right), "{left} != {right}");
        assert!(!python_equal(&right, &left), "{right} != {left}");
    }
}

#[test]
fn containers_compare_item_by_item() {
    assert!(python_equal(&json!([1, "a"]), &json!([1.0, "a"])));
    assert!(!python_equal(&json!([1]), &json!([1, 2])));
    assert!(python_equal(
        &json!({"a": 1, "b": [true]}),
        &json!({"b": [1], "a": 1.0})
    ));
    assert!(!python_equal(&json!({"a": 1}), &json!({"a": 1, "b": 2})));
    assert!(!python_equal(&json!({"a": 1}), &json!({"b": 1})));
    assert!(!python_equal(&json!([]), &json!({})));
}

#[test]
fn truthiness_is_pythons() {
    for falsy in [
        json!(null),
        json!(false),
        json!(0),
        json!(0.0),
        json!(-0.0),
        json!(""),
        json!([]),
        json!({}),
    ] {
        assert!(!python_truthy(&falsy), "{falsy}");
    }
    for truthy in [
        json!(true),
        json!(5),
        json!(-1),
        json!(u64::MAX),
        json!(0.5),
        json!(" "),
        json!("0"),
        json!([0]),
        json!({"": null}),
    ] {
        assert!(python_truthy(&truthy), "{truthy}");
    }
}
