//! Wall-clock formatting helpers for session/timestamp display. Split out of
//! `app_methods` (which sits at the source line cap) and re-exported from it,
//! so existing `app_methods::format_utc_minutes` call sites are unchanged.

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM` in **local** time, falling
/// back to UTC if the platform's local-time conversion is unavailable.
pub(super) fn format_unix_minutes(secs: u64) -> String {
    format_local_minutes(secs).unwrap_or_else(|| format_utc_minutes(secs))
}

/// Local time via `libc::localtime_r`. Returns `None` if the conversion fails.
fn format_local_minutes(secs: u64) -> Option<String> {
    let t = secs.try_into().ok()?;
    // SAFETY: `libc::tm` is plain-old-data; an all-zero value is a valid initial state for libc to fill.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `&t`/`&mut tm` point to live locals; localtime_r fills `tm` and returns null on failure (checked next).
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    ))
}

/// UTC fallback (pure arithmetic) when local-time conversion is unavailable.
pub(super) fn format_utc_minutes(secs: u64) -> String {
    let secs = secs as i64;
    let days = secs.div_euclid(86_400);
    let mut rem = secs.rem_euclid(86_400);
    let hour = rem / 3_600;
    rem %= 3_600;
    let minute = rem / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

pub(super) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}
