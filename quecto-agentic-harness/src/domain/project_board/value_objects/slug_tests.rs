use super::*;

#[test]
fn a_slug_is_lowercase_ascii_letters_digits_and_inner_hyphens() {
    for good in [
        "t1",
        "2476-s1a",
        "a",
        "console",
        "com10",
        &"x".repeat(MAX_SLUG_BYTES),
    ] {
        assert_eq!(
            Slug::parse(good).map(|slug| slug.as_str().to_string()),
            Ok(good.to_string())
        );
    }
    let long = "x".repeat(MAX_SLUG_BYTES + 1);
    for bad in [
        "",
        "T1",
        "-t1",
        "t1-",
        "t_1",
        "t 1",
        "t/1",
        "../t",
        "é",
        long.as_str(),
    ] {
        assert_eq!(
            Slug::parse(bad).map_err(|error| error.field),
            Err("id".into()),
            "{bad:?}"
        );
    }
}

#[test]
fn windows_device_names_are_refused() {
    for reserved in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
        assert!(Slug::parse(reserved).is_err(), "{reserved}");
    }
}
