use super::super::wire_dialect::WireDialect;
use crate::domain::inference::value_objects::attempt_diagnostics::*;
use sha2::{Digest, Sha256};

pub(super) fn headers<'a>(values: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<SafeHeader> {
    values
        .into_iter()
        .filter_map(|(name, value)| {
            let name = match name {
                "request-id" => HeaderName::RequestId,
                "x-request-id" => HeaderName::XRequestId,
                "retry-after" => HeaderName::RetryAfter,
                "retry-after-ms" => HeaderName::RetryAfterMs,
                "x-ratelimit-remaining-requests" => HeaderName::RemainingRequests,
                "x-ratelimit-remaining-tokens" => HeaderName::RemainingTokens,
                "x-ratelimit-limit-requests" => HeaderName::LimitRequests,
                "x-ratelimit-limit-tokens" => HeaderName::LimitTokens,
                "x-ratelimit-reset-requests" => HeaderName::ResetRequests,
                "x-ratelimit-reset-tokens" => HeaderName::ResetTokens,
                _ => return None,
            };
            let value = match name {
                HeaderName::RequestId | HeaderName::XRequestId
                    if (1..=256).contains(&value.len())
                        && value.bytes().all(|b| {
                            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')
                        }) =>
                {
                    HeaderValue::Sha256(format!("{:x}", Sha256::digest(value.as_bytes())))
                }
                HeaderName::RequestId | HeaderName::XRequestId => return None,
                _ if (1..=13).contains(&value.len())
                    && value.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    let number = value.parse::<u64>().ok()?;
                    if number <= 1_000_000_000_000 {
                        HeaderValue::Number(number)
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            Some(SafeHeader { name, value })
        })
        .take(10)
        .collect()
}
/// Record what the typed error fields of `value`, a payload of `wire`, say:
/// the last error name of `wire` its error objects carry — an object's
/// `type` over its `code`, as before (#2620 review) — and why its reply
/// stopped short. Only names the wire knows are kept, never values or
/// messages (#2609 review: each wire reads its own shapes).
pub(super) fn typed(d: &mut AttemptDiagnostics, wire: &dyn WireDialect, value: &serde_json::Value) {
    for error in wire.error_objects(value) {
        for field in ["code", "type"] {
            if let Some(known) = error[field].as_str().and_then(|name| wire.error_name(name)) {
                d.error_code = Some(known.code());
            }
        }
    }
    if let Some(reason) = wire.incomplete_reason(value) {
        d.incomplete_reason = Some(reason);
    }
}
pub(super) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "transport_diagnostics_tests.rs"]
mod tests;
