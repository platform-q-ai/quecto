use super::*;

#[test]
fn a_slug_is_lowercase_ascii_letters_digits_and_inner_hyphens() {
    for good in ["t1", "2476-s1a", "a", &"x".repeat(MAX_SLUG_BYTES)] {
        assert_eq!(Slug::parse(good).map(String::from), Ok(good.to_string()));
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
        assert!(Slug::parse(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn a_slug_is_read_and_written_as_a_json_string() {
    let slug: Slug = serde_json::from_str("\"t-1\"").unwrap();
    assert_eq!(serde_json::to_string(&slug).unwrap(), "\"t-1\"");
    assert!(serde_json::from_str::<Slug>("\"T-1\"").is_err());
}
