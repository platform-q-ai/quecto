use super::*;

#[test]
fn a_timestamp_is_rfc3339_utc_to_the_second_from_1970() {
    for good in [
        "2026-10-10T09:30:00Z",
        "2024-02-29T23:59:59Z",
        "1970-01-01T00:00:00Z",
        LAST,
    ] {
        assert_eq!(
            Timestamp::parse(good).map(|time| time.as_str().to_string()),
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
        "2016-12-31T23:59:60Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert!(Timestamp::parse(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn timestamps_order_chronologically() {
    let early = Timestamp::parse("2026-09-30T23:59:59Z").unwrap();
    let late = Timestamp::parse("2026-10-01T00:00:00Z").unwrap();
    assert!(early < late);
}

#[test]
fn a_timestamp_moves_forward_by_whole_seconds_up_to_the_year_9999() {
    let late = Timestamp::parse("2024-02-28T23:00:00Z").unwrap();
    assert_eq!(
        late.plus_seconds(7200)
            .map(|time| time.as_str().to_string()),
        Ok("2024-02-29T01:00:00Z".into())
    );
    let end = Timestamp::parse("9999-12-31T22:59:59Z").unwrap();
    assert_eq!(
        end.plus_seconds(3600).map(|time| time.as_str().to_string()),
        Ok(LAST.into())
    );
    assert!(
        end.plus_seconds(3601).is_err(),
        "past the year 9999 is refused"
    );
}

/// Epic #2481 T4: the signed seconds between two moments, for a claim's
/// age and time left; whole range, either way round.
#[test]
fn the_seconds_between_two_moments_are_signed() {
    let at = |text: &str| Timestamp::parse(text).unwrap();
    let (early, late) = (at("2026-10-10T09:00:00Z"), at("2026-10-10T11:00:01Z"));
    assert_eq!(late.seconds_since(&early), 7201);
    assert_eq!(early.seconds_since(&late), -7201);
    assert_eq!(early.seconds_since(&early), 0);
    let (first, last) = (at("1970-01-01T00:00:00Z"), at(LAST));
    assert_eq!(last.seconds_since(&first), 253_402_300_799);
    assert_eq!(first.seconds_since(&last), -253_402_300_799);
}

/// #2705 B9 review round 4: a moment `seconds` earlier, refused before
/// 1970, as a refusal names when a clock bound clears.
#[test]
fn a_moment_seconds_earlier_is_spelt_and_refused_before_1970() {
    let at = Timestamp::parse("2026-10-10T23:00:00Z").unwrap();
    assert_eq!(
        at.minus_seconds(60),
        Ok(Timestamp::parse("2026-10-10T22:59:00Z").unwrap())
    );
    let first = Timestamp::parse("1970-01-01T00:00:00Z").unwrap();
    assert!(first.minus_seconds(1).is_err());
    assert_eq!(first.minus_seconds(0), Ok(first));
}
