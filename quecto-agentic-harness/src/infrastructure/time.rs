//! Lightweight time utilities that replace `chrono`.
//!
//! Uses `std::time::SystemTime` for UTC epoch seconds and
//! the system `date` command for local-time formatting.

use std::time::{SystemTime, UNIX_EPOCH};

/// Return the current UTC time as seconds since the Unix epoch.
pub fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_secs() as i64
}

/// Format the current local date and time (e.g. for human-facing displays).
///
/// Output example: `"Saturday, March 1, 2026 at 10:30:15 AM GMT"`
///
/// Uses the system `date` command for local-time formatting (with timezone).
/// Falls back to a pure-Rust UTC-only format if `date` is unavailable or
/// produces unexpected output.
///
/// **Note:** Spawns a child process (`/usr/bin/date`). Call once per session,
/// not on a hot path.
pub fn format_local_datetime() -> String {
    // Use absolute path to avoid PATH-based injection.
    // Format uses GNU %-d (no leading zero); BSD date may not support it.
    if let Ok(output) = std::process::Command::new("/usr/bin/date")
        .env_clear()
        .env("TZ", std::env::var("TZ").unwrap_or_default())
        .env("LANG", std::env::var("LANG").unwrap_or_default())
        .arg("+%A, %B %-d, %Y at %I:%M:%S %p %Z")
        .output()
    {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout);
            let trimmed = s.trim();
            // Guard: reject output containing literal '%' (BSD didn't expand the format)
            if !trimmed.is_empty() && !trimmed.contains('%') {
                return trimmed.replace(" am ", " AM ").replace(" pm ", " PM ");
            }
        }
    }

    // Fallback: UTC-only, no local timezone
    format_utc_datetime(unix_timestamp_secs())
}

/// Format a UTC date string from epoch seconds.
///
/// Used as fallback when the system `date` command is unavailable,
/// and exposed for deterministic testing.
fn format_utc_datetime(secs: i64) -> String {
    // Convert epoch seconds to date components (UTC)
    let days = secs / 86400;
    let time_of_day = secs % 86400;

    let hours = (time_of_day / 3600) as u32;
    let minutes = ((time_of_day % 3600) / 60) as u32;
    let seconds = (time_of_day % 60) as u32;

    // Civil date from day count (algorithm from Howard Hinnant)
    let (year, month, day) = civil_from_days(days);

    let dow = day_of_week(days);
    let day_name = DAY_NAMES[dow as usize];
    let month_name = MONTH_NAMES[(month - 1) as usize];

    let (hour12, ampm) = match hours {
        0 => (12, "AM"),
        1..=11 => (hours, "AM"),
        12 => (12, "PM"),
        _ => (hours - 12, "PM"),
    };

    format!(
        "{}, {} {} {} at {:02}:{:02}:{:02} {} UTC",
        day_name, month_name, day, year, hour12, minutes, seconds, ampm
    )
}

/// Convert a day count (from Unix epoch) to (year, month, day).
/// Algorithm by Howard Hinnant.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Day of week: 0 = Sunday, 6 = Saturday.
fn day_of_week(days: i64) -> u32 {
    ((days % 7 + 4 + 7) % 7) as u32 // Unix epoch (1970-01-01) was Thursday (4)
}

const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[cfg(test)]
#[path = "time_tests.rs"]
mod tests;
