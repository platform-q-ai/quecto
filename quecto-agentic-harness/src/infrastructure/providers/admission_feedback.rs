//! Typed HTTP throttle hints; never parses or rewrites provider error prose.
//! Capture wall and authority-monotonic time together at response receipt.
use std::time::SystemTime;
#[cfg(test)]
use std::time::{Duration, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CooldownHint {
    /// Missing, malformed or negative advice; fallback needs typed confirmation.
    Fallback,
    /// Absolute authority-monotonic deadline, merged by maximum by the owner.
    Until(u64),
    /// Valid advice exceeds policy, or its arithmetic is unrepresentable.
    Unavailable,
}

/// Normalize all supported header occurrences, retaining the longest valid hint.
/// `headers` contains header names and UTF-8 values (invalid UTF-8 may be omitted).
/// HTTP 429 and 529 are only throttle candidates; other statuses return `None`.
/// This helper normalizes hints ONLY, not provider error classification. A valid
/// candidate-response hint may establish cooldown immediately at receipt; a later
/// terminal billing body must not retract that validated cooldown. `Fallback`
/// requires separate typed throttle confirmation before the caller applies any
/// fallback delay. Billing errors must not be retried; retry classification and
/// ownership remain outside this helper.
/// Receipt feedback is non-terminal and must not release or complete an attempt.
/// Zero maximum is invalid configuration and fails closed for candidate responses.
pub fn normalize_throttle<'a>(
    status: u16,
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    received_wall: SystemTime,
    received_mono_ms: u64,
    maximum_ms: u64,
) -> Option<CooldownHint> {
    if !matches!(status, 429 | 529) {
        return None;
    }
    if maximum_ms == 0 {
        return Some(CooldownHint::Unavailable);
    }

    let mut longest = None;
    for (name, value) in headers {
        let seconds = if name.eq_ignore_ascii_case("retry-after") {
            true
        } else if name.eq_ignore_ascii_case("retry-after-ms") {
            false
        } else {
            continue;
        };
        let delay = match delay_ms(value, seconds, received_wall) {
            Ok(Some(delay)) if delay <= maximum_ms => delay,
            Ok(None) => continue,
            _ => return Some(CooldownHint::Unavailable),
        };
        let Some(deadline) = received_mono_ms.checked_add(delay) else {
            return Some(CooldownHint::Unavailable);
        };
        longest = Some(longest.unwrap_or(deadline).max(deadline));
    }
    Some(longest.map_or(CooldownHint::Fallback, CooldownHint::Until))
}

/// Invalid syntax is absent advice; numeric or clock overflow fails closed.
fn delay_ms(value: &str, seconds: bool, received_wall: SystemTime) -> Result<Option<u64>, ()> {
    let value = value.trim_matches([' ', '\t']);
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let number = value.parse::<u64>().map_err(|_| ())?;
        return number
            .checked_mul(if seconds { 1000 } else { 1 })
            .map(Some)
            .ok_or(());
    }
    if !seconds {
        return Ok(None);
    }
    // The parser validates the calendar and weekday in all three HTTP-date forms.
    // httpdate accepts 1970..=9999 and maps RFC850 years to 1970..=2069,
    // not a moving 50-year window. Unsupported dates conservatively fall back.
    // Reject controls rather than letting its broad whitespace trim accept them.
    if value.bytes().any(|byte| byte.is_ascii_control()) {
        return Ok(None);
    }
    let Ok(date) = httpdate::parse_http_date(value) else {
        return Ok(None);
    };
    let delay = date.duration_since(received_wall).unwrap_or_default();
    // Round up fractional milliseconds so conversion cannot allow an early retry.
    let millis = delay.as_nanos().div_ceil(1_000_000);
    u64::try_from(millis).map(Some).map_err(|_| ())
}

#[cfg(test)]
#[path = "admission_feedback_tests.rs"]
mod tests;

/// Classify structured HTTP/SSE error fields independently of display text.
/// A terminal billing marker takes precedence over a throttle marker, including
/// when an envelope contains root, direct, and Responses-nested error objects.
/// Root fields are error metadata only for an explicit `type: "error"` event.
pub fn is_typed_throttle(value: &serde_json::Value) -> bool {
    let errors = [
        (value.get("type").and_then(serde_json::Value::as_str) == Some("error")).then_some(value),
        value.get("error"),
        value
            .get("response")
            .and_then(|response| response.get("error")),
    ];
    let mut throttle = false;
    for error in errors.into_iter().flatten() {
        for field in ["type", "code"] {
            match error.get(field).and_then(serde_json::Value::as_str) {
                Some("insufficient_quota" | "billing_hard_limit_reached") => return false,
                Some("rate_limit_error" | "overloaded_error" | "rate_limit_exceeded") => {
                    throttle = true
                }
                _ => {}
            }
        }
    }
    throttle
}
