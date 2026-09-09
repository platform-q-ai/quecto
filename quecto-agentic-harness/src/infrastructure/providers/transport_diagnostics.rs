use crate::domain::attempt_diagnostics::*;
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
pub(super) fn typed(d: &mut AttemptDiagnostics, value: &serde_json::Value) {
    for error in [&value["error"], &value["response"]["error"]] {
        for field in ["code", "type"] {
            let code = match error[field].as_str() {
                Some("rate_limit_exceeded" | "rate_limit_error") => ErrorCode::RateLimitExceeded,
                Some("overloaded_error") => ErrorCode::Overloaded,
                Some("server_error") => ErrorCode::ServerError,
                Some("insufficient_quota") => ErrorCode::InsufficientQuota,
                Some("usage_limit_reached") => ErrorCode::UsageLimitReached,
                Some("authentication_error") => ErrorCode::AuthenticationError,
                Some("permission_error") => ErrorCode::PermissionError,
                Some("invalid_request_error") => ErrorCode::InvalidRequestError,
                Some("invalid_api_key") => ErrorCode::InvalidApiKey,
                _ => continue,
            };
            d.error_code = Some(code);
        }
    }
    d.incomplete_reason = match value["response"]["incomplete_details"]["reason"].as_str() {
        Some("max_output_tokens") => Some(IncompleteReason::MaxOutputTokens),
        Some("content_filter") => Some(IncompleteReason::ContentFilter),
        _ => d.incomplete_reason,
    };
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
