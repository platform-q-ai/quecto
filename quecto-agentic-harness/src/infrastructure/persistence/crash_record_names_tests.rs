use super::*;

fn digest() -> String {
    "0123456789abcdef".repeat(4)
}

#[test]
fn each_record_shape_is_told_apart() {
    let stem = digest();
    let fatal = format!("{stem}.crash");
    let provisional = format!("{stem}.crash.provisional.42.7");
    assert_eq!(
        RecordName::parse(&fatal),
        Some(RecordName::Fatal { stem: &stem })
    );
    assert_eq!(
        RecordName::parse(&provisional),
        Some(RecordName::Provisional {
            stem: &stem,
            pid: 42,
            scope: 7
        })
    );
    for temporary in [
        format!(".{fatal}.00000000000000ff.tmp"),
        format!(".{provisional}.0123456789abcdef.tmp"),
    ] {
        assert_eq!(
            RecordName::parse(&temporary),
            Some(RecordName::Temporary { stem: &stem }),
            "{temporary}"
        );
    }
}

/// Only a record's exact shapes: anything else in the directory is left
/// by a clear and by the sweep.
#[test]
fn no_other_name_is_a_records() {
    let stem = digest();
    let upper = stem.to_uppercase();
    let short = &stem[1..];
    for name in [
        String::new(),
        "crash".to_string(),
        format!("{upper}.crash"),
        format!("{short}.crash"),
        format!("{stem}0.crash"),
        format!("{stem}.crash.x"),
        format!("{stem}.crashy"),
        format!("{stem}.crash.provisional.1"),
        format!("{stem}.crash.provisional.1.2.3"),
        format!("{stem}.crash.provisional.1.x"),
        format!("{stem}.crash.provisional.+1.2"),
        format!("{stem}.crash.provisional.1.2x"),
        format!("{stem}.jsonl"),
        "cli_abc.crash".to_string(),
        format!(".{stem}.crash.tmp"),
        format!(".{stem}.crash.00000000000000FF.tmp"),
        format!(".{stem}.crash.0000000000000ff.tmp"),
        format!(".{stem}.crash.00000000000000ff.tmpx"),
        format!("{stem}.crash.00000000000000ff.tmp"),
        format!(".{stem}.jsonl.00000000000000ff.tmp"),
        format!("..{stem}.crash.00000000000000ff.tmp"),
        format!("{}é.crash", &stem[..63]),
    ] {
        assert_eq!(RecordName::parse(&name), None, "{name:?}");
    }
}

#[test]
fn a_decimal_is_digits_only() {
    assert_eq!(decimal("0"), Some(0));
    assert_eq!(decimal("18446744073709551615"), Some(u64::MAX));
    for text in ["", "18446744073709551616", "1x", "-1", "+1", " 1"] {
        assert_eq!(decimal(text), None, "{text:?}");
    }
}
