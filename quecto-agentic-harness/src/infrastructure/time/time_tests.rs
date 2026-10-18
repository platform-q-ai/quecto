use super::*;

#[test]
fn test_unix_timestamp_secs_is_recent() {
    let ts = unix_timestamp_secs();
    // Should be after 2024-01-01 (1704067200) and before 2040-01-01
    assert!(ts > 1_704_067_200, "timestamp too old: {}", ts);
    assert!(ts < 2_208_988_800, "timestamp too far in future: {}", ts);
}

#[test]
fn test_format_local_datetime_structure() {
    let formatted = format_local_datetime();

    // Must contain a day-of-week
    let days = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    assert!(
        days.iter().any(|d| formatted.contains(d)),
        "should contain day-of-week, got: {}",
        formatted
    );

    // Must contain AM or PM
    assert!(
        formatted.contains("AM") || formatted.contains("PM"),
        "should contain AM/PM, got: {}",
        formatted
    );

    // Must contain colons (HH:MM:SS)
    let colon_count = formatted.chars().filter(|c| *c == ':').count();
    assert!(
        colon_count >= 2,
        "should have at least 2 colons for HH:MM:SS, got: {}",
        formatted
    );

    // Must contain "at" separator
    assert!(
        formatted.contains(" at "),
        "should contain ' at ', got: {}",
        formatted
    );
}

#[test]
fn test_format_local_datetime_contains_current_year() {
    let formatted = format_local_datetime();
    let ts = unix_timestamp_secs();
    let approx_year = 1970 + ts / 31_557_600;
    let year_str = approx_year.to_string();
    assert!(
        formatted.contains(&year_str),
        "should contain year {}, got: {}",
        year_str,
        formatted
    );
}

#[test]
fn test_civil_from_days_epoch() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
}

#[test]
fn test_civil_from_days_known_date() {
    // 2026-03-09 = day 20521 from epoch
    assert_eq!(civil_from_days(20_521), (2026, 3, 9));
}

#[test]
fn test_day_of_week_epoch() {
    // 1970-01-01 was Thursday (4)
    assert_eq!(day_of_week(0), 4);
}

#[test]
fn test_day_of_week_known_date() {
    // 2026-03-09 (day 20521) is Monday (1)
    assert_eq!(day_of_week(20_521), 1);
}

#[test]
fn test_utc_fallback_format() {
    let formatted = format_utc_datetime(unix_timestamp_secs());
    assert!(
        formatted.contains("UTC"),
        "fallback should contain UTC, got: {}",
        formatted
    );
    assert!(
        formatted.contains(" at "),
        "fallback should contain ' at ', got: {}",
        formatted
    );
}

#[test]
fn test_utc_datetime_epoch_zero() {
    let formatted = format_utc_datetime(0);
    assert_eq!(
        formatted, "Thursday, January 1 1970 at 12:00:00 AM UTC",
        "epoch zero should be midnight Jan 1 1970"
    );
}

#[test]
fn test_utc_datetime_leap_day() {
    // 2000-02-29 00:00:00 UTC = 951782400
    let formatted = format_utc_datetime(951_782_400);
    assert!(
        formatted.starts_with("Tuesday, February 29 2000"),
        "leap day 2000-02-29 should be Tuesday, got: {}",
        formatted
    );
}

#[test]
fn test_utc_datetime_noon() {
    // 1970-01-01 12:00:00 UTC = 43200
    let formatted = format_utc_datetime(43200);
    assert!(
        formatted.contains("12:00:00 PM"),
        "noon should be 12:00:00 PM, got: {}",
        formatted
    );
}
