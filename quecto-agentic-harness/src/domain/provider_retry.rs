//! Bounded retry policy shared by streaming and non-streaming orchestration.
use super::error::DomainError;
use std::time::Duration;

/// Respect a provider horizon, or return control when it exceeds the turn's wait budget.
pub fn bounded_delay(
    error: &DomainError,
    fallback: Duration,
    maximum: Duration,
) -> Option<Duration> {
    let hint = match error {
        DomainError::Provider(message) => parse_retry_after(&message.to_ascii_lowercase()),
        _ => None,
    };
    match hint {
        Some(delay) if delay <= maximum => Some(delay),
        Some(_) => None,
        None => Some(fallback.min(maximum)),
    }
}

/// Parse a `retry-after-ms` (milliseconds) or `retry-after` (seconds) hint from
/// an already-lowercased provider error string.
fn parse_retry_after(lowered: &str) -> Option<Duration> {
    if let Some(ms) = number_after(lowered, "retry-after-ms") {
        return Some(Duration::from_millis(ms));
    }
    if let Some(secs) = number_after(lowered, "retry-after") {
        return Some(Duration::from_secs(secs));
    }
    None
}

/// Find `marker` in `s`, then parse the run of digits that follows (skipping
/// separators like `:`, `=`, whitespace and quotes).
fn number_after(s: &str, marker: &str) -> Option<u64> {
    let rel = s.find(marker)?;
    let rest = &s[rel + marker.len()..];
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            let mut value: u64 = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                value = value
                    .saturating_mul(10)
                    .saturating_add((bytes[i] - b'0') as u64);
                i += 1;
            }
            return Some(value);
        }
        if matches!(b, b':' | b'=' | b'"' | b'\'' | b' ' | b'\t') {
            i += 1;
            continue;
        }
        return None;
    }
    None
}
