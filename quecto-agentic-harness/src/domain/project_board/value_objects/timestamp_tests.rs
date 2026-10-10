use super::*;

#[test]
fn a_timestamp_is_rfc3339_utc_to_the_second() {
    for good in [
        "2026-10-10T09:30:00Z",
        "2024-02-29T23:59:59Z",
        "1970-01-01T00:00:00Z",
    ] {
        assert_eq!(
            Timestamp::parse(good).map(String::from),
            Ok(good.to_string())
        );
    }
    for bad in [
        "",
        "2026-10-10",
        "2026-10-10T09:30:00",
        "2026-10-10T09:30:00+01:00",
        "2026-10-10T09:30:00.5Z",
        "2026-10-10 09:30:00Z",
        "2026-13-10T09:30:00Z",
        "2025-02-29T00:00:00Z",
        "2026-10-10T24:00:00Z",
        "２026-10-10T09:30:00Z",
    ] {
        assert!(Timestamp::parse(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn timestamps_order_chronologically_and_travel_as_json_strings() {
    let early = Timestamp::parse("2026-09-30T23:59:59Z").unwrap();
    let late = Timestamp::parse("2026-10-01T00:00:00Z").unwrap();
    assert!(early < late);
    assert_eq!(
        serde_json::to_string(&late).unwrap(),
        "\"2026-10-01T00:00:00Z\""
    );
    assert!(serde_json::from_str::<Timestamp>("\"2026-10-01\"").is_err());
}
