//! Unit tests for provider error classification.

use super::*;
use crate::domain::error::DomainError;

fn provider(msg: &str) -> DomainError {
    DomainError::Provider(msg.to_string())
}

#[test]
fn from_status_maps_all_groups() {
    use ProviderErrorClass::*;
    assert_eq!(ProviderErrorClass::from_status(401), Auth);
    assert_eq!(ProviderErrorClass::from_status(403), Auth);
    assert_eq!(ProviderErrorClass::from_status(429), RateLimit);
    for s in [400, 404, 405, 406, 409, 410, 422] {
        assert_eq!(ProviderErrorClass::from_status(s), Client, "status {s}");
    }
    for s in [500, 502, 503, 504, 529] {
        assert_eq!(ProviderErrorClass::from_status(s), Server, "status {s}");
    }
    for s in [200, 301, 418, 451, 599] {
        assert_eq!(ProviderErrorClass::from_status(s), Unknown, "status {s}");
    }
}

#[test]
fn is_retryable_only_for_transient_classes() {
    use ProviderErrorClass::*;
    assert!(RateLimit.is_retryable());
    assert!(Server.is_retryable());
    assert!(Network.is_retryable());
    assert!(!Auth.is_retryable());
    assert!(!Client.is_retryable());
    assert!(!Cancelled.is_retryable());
    assert!(!Unknown.is_retryable());
}

#[test]
fn as_str_and_display_match() {
    use ProviderErrorClass::*;
    let cases = [
        (RateLimit, "rate_limit"),
        (Auth, "auth"),
        (Server, "server"),
        (Client, "client"),
        (Network, "network"),
        (Cancelled, "cancelled"),
        (Unknown, "unknown"),
    ];
    for (class, text) in cases {
        assert_eq!(class.as_str(), text);
        assert_eq!(format!("{class}"), text);
    }
}

#[test]
fn classify_non_provider_error_is_unknown() {
    assert_eq!(
        classify_provider_error(&DomainError::Tool("boom".into())),
        ProviderErrorClass::Unknown
    );
}

#[test]
fn classify_prefers_http_status_over_keywords() {
    // Status wins even when a keyword is also present.
    assert_eq!(
        classify_provider_error(&provider("HTTP 429 rate limit")),
        ProviderErrorClass::RateLimit
    );
    assert_eq!(
        classify_provider_error(&provider("status: 500 internal server error")),
        ProviderErrorClass::Server
    );
    assert_eq!(
        classify_provider_error(&provider("unexpected code=404 from upstream")),
        ProviderErrorClass::Client
    );
    assert_eq!(
        classify_provider_error(&provider("auth failed (http/401)")),
        ProviderErrorClass::Auth
    );
}

#[test]
fn classify_cancelled_variants() {
    assert_eq!(
        classify_provider_error(&provider("request cancelled by user")),
        ProviderErrorClass::Cancelled
    );
    assert_eq!(
        classify_provider_error(&provider("Request Canceled")),
        ProviderErrorClass::Cancelled
    );
}

#[test]
fn classify_keyword_fallbacks() {
    use ProviderErrorClass::*;
    let cases = [
        ("rate limit exceeded", RateLimit),
        ("unauthorized access", Auth),
        ("invalid api key provided", Auth),
        ("authentication required", Auth),
        ("forbidden", Auth),
        ("internal server error", Server),
        ("bad gateway", Server),
        ("service unavailable", Server),
        ("gateway timeout", Server),
        ("overloaded_error: try later", Server),
        ("connection refused", Network),
        ("could not connect to host", Network),
        ("dns resolution failed", Network),
        ("operation timed out", Network),
        ("network is unreachable", Network),
        ("something completely unexpected", Unknown),
    ];
    for (msg, expected) in cases {
        assert_eq!(classify_provider_error(&provider(msg)), expected, "{msg}");
    }
}

#[test]
fn classify_status_marker_without_valid_number_falls_through() {
    // "http" marker present but no 3-digit code → keyword path used instead.
    assert_eq!(
        classify_provider_error(&provider("http error: connection reset")),
        ProviderErrorClass::Network
    );
    // single digit after marker is not a status code
    assert_eq!(
        classify_provider_error(&provider("http 4 rate limit")),
        ProviderErrorClass::RateLimit
    );
    // out-of-range 3-digit (>599) is rejected, keyword wins
    assert_eq!(
        classify_provider_error(&provider("http 999 unauthorized")),
        ProviderErrorClass::Auth
    );
    // leading-zero code (099 = 99) is out of range → falls through to unknown
    assert_eq!(
        classify_provider_error(&provider("status 099 weirdness")),
        ProviderErrorClass::Unknown
    );
}

#[test]
fn classify_status_separators_between_marker_and_code() {
    // separators (space, colon, equals, dash, slash) are skipped before the digits
    assert_eq!(
        classify_provider_error(&provider("HTTP://503")),
        ProviderErrorClass::Server
    );
    assert_eq!(
        classify_provider_error(&provider("status = 401")),
        ProviderErrorClass::Auth
    );
    // a non-separator, non-digit char right after the marker aborts parsing → keyword path
    assert_eq!(
        classify_provider_error(&provider("https rate limit")),
        ProviderErrorClass::RateLimit
    );
}
