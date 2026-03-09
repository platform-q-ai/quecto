use crate::domain::error::DomainError;

/// Classify a `DomainError` into an `ErrorClass`.
///
/// Inspects the error message for HTTP status codes and semantic keywords
/// (rate limit, auth, network, etc.). Only `DomainError::Provider` variants
/// are classified; all others return `ErrorClass::Unknown`.
pub fn classify_error(err: &DomainError) -> ErrorClass {
    let msg = match err {
        DomainError::Provider(msg) => msg,
        _ => return ErrorClass::Unknown,
    };

    if let Some(status) = extract_http_status(msg) {
        return ErrorClass::from_status(status);
    }

    let lowered = msg.to_ascii_lowercase();

    if lowered.contains("request cancelled") || lowered.contains("request canceled") {
        return ErrorClass::Cancelled;
    }

    if lowered.contains("rate limit") {
        ErrorClass::RateLimit
    } else if lowered.contains("auth")
        || lowered.contains("unauthorized")
        || lowered.contains("forbidden")
        || lowered.contains("invalid api key")
        || lowered.contains("authentication")
    {
        ErrorClass::Auth
    } else if lowered.contains("internal server error")
        || lowered.contains("bad gateway")
        || lowered.contains("service unavailable")
        || lowered.contains("gateway timeout")
        || lowered.contains("overloaded_error")
    {
        ErrorClass::Server
    } else if lowered.contains("connect")
        || lowered.contains("connection")
        || lowered.contains("timeout")
        || lowered.contains("timed out")
        || lowered.contains("network")
        || lowered.contains("dns")
    {
        ErrorClass::Network
    } else {
        ErrorClass::Unknown
    }
}

fn extract_http_status(msg: &str) -> Option<u16> {
    let lowered = msg.to_ascii_lowercase();

    for marker in ["http", "status", "code"] {
        let mut search_from = 0;
        while let Some(rel) = lowered[search_from..].find(marker) {
            let idx = search_from + rel + marker.len();
            if let Some(code) = parse_status_near(&lowered[idx..]) {
                return Some(code);
            }
            search_from = idx;
        }
    }

    None
}

fn parse_status_near(s: &str) -> Option<u16> {
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_digit()
                || !bytes[i + 2].is_ascii_digit()
            {
                return None;
            }
            let code = ((bytes[i] - b'0') as u16) * 100
                + ((bytes[i + 1] - b'0') as u16) * 10
                + ((bytes[i + 2] - b'0') as u16);
            return (100..=599).contains(&code).then_some(code);
        }

        if !(b.is_ascii_whitespace() || b == b':' || b == b'=' || b == b'-' || b == b'/') {
            return None;
        }
        i += 1;
    }

    None
}

/// Classification of provider errors for retry/fallback decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorClass {
    /// Rate limit exceeded (429)
    RateLimit,
    /// Authentication error (401, 403)
    Auth,
    /// Server error (500, 502, 503, 504, 529 overloaded)
    Server,
    /// Client error (400, 404, etc.)
    Client,
    /// Network/connection error
    Network,
    /// Request was cancelled by the caller
    Cancelled,
    /// Unknown error
    Unknown,
}

impl ErrorClass {
    /// Classify an HTTP status code into an error class.
    pub fn from_status(status: u16) -> Self {
        match status {
            401 | 403 => ErrorClass::Auth,
            429 => ErrorClass::RateLimit,
            400 | 404 | 405 | 406 | 409 | 410 | 422 => ErrorClass::Client,
            500 | 502 | 503 | 504 | 529 => ErrorClass::Server,
            _ => ErrorClass::Unknown,
        }
    }

    /// Whether this error class is retryable (should try fallback provider).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ErrorClass::RateLimit | ErrorClass::Server | ErrorClass::Network
        )
    }

    /// Human-readable name for the error class.
    pub fn as_str(&self) -> &str {
        match self {
            ErrorClass::RateLimit => "rate_limit",
            ErrorClass::Auth => "auth",
            ErrorClass::Server => "server",
            ErrorClass::Client => "client",
            ErrorClass::Network => "network",
            ErrorClass::Cancelled => "cancelled",
            ErrorClass::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_classification() {
        let class = ErrorClass::from_status(429);
        assert_eq!(class, ErrorClass::RateLimit);
        assert_eq!(class.as_str(), "rate_limit");
        assert!(class.is_retryable());
    }

    #[test]
    fn test_auth_classification() {
        let class = ErrorClass::from_status(401);
        assert_eq!(class, ErrorClass::Auth);
        assert_eq!(class.as_str(), "auth");
        assert!(!class.is_retryable());

        let class403 = ErrorClass::from_status(403);
        assert_eq!(class403, ErrorClass::Auth);
    }

    #[test]
    fn test_server_classification() {
        for status in [500, 502, 503, 504] {
            let class = ErrorClass::from_status(status);
            assert_eq!(class, ErrorClass::Server);
            assert!(class.is_retryable());
        }
    }

    #[test]
    fn test_529_is_server_and_retryable() {
        let class = ErrorClass::from_status(529);
        assert_eq!(class, ErrorClass::Server, "HTTP 529 should be Server");
        assert!(class.is_retryable(), "HTTP 529 should be retryable");
    }

    #[test]
    fn test_client_classification() {
        let class = ErrorClass::from_status(400);
        assert_eq!(class, ErrorClass::Client);
        assert!(!class.is_retryable());
    }

    #[test]
    fn test_network_is_retryable() {
        assert!(ErrorClass::Network.is_retryable());
    }

    #[test]
    fn test_unknown_status() {
        let class = ErrorClass::from_status(418);
        assert_eq!(class, ErrorClass::Unknown);
        assert!(!class.is_retryable());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ErrorClass::RateLimit), "rate_limit");
        assert_eq!(format!("{}", ErrorClass::Auth), "auth");
        assert_eq!(format!("{}", ErrorClass::Server), "server");
        assert_eq!(format!("{}", ErrorClass::Client), "client");
        assert_eq!(format!("{}", ErrorClass::Network), "network");
        assert_eq!(format!("{}", ErrorClass::Cancelled), "cancelled");
        assert_eq!(format!("{}", ErrorClass::Unknown), "unknown");
    }

    #[test]
    fn test_as_str_all_variants() {
        assert_eq!(ErrorClass::Client.as_str(), "client");
        assert_eq!(ErrorClass::Network.as_str(), "network");
        assert_eq!(ErrorClass::Cancelled.as_str(), "cancelled");
        assert_eq!(ErrorClass::Unknown.as_str(), "unknown");
    }

    #[test]
    fn test_cancelled_not_retryable() {
        assert!(!ErrorClass::Cancelled.is_retryable());
    }

    #[test]
    fn test_client_not_retryable() {
        assert!(!ErrorClass::Client.is_retryable());
        assert!(!ErrorClass::Unknown.is_retryable());
    }

    // ── classify_error() tests (migrated from fallback_tests.rs) ───────

    #[test]
    fn test_classify_rate_limit() {
        let err = DomainError::Provider("HTTP 429 rate limit".to_string());
        assert_eq!(classify_error(&err), ErrorClass::RateLimit);
    }

    #[test]
    fn test_classify_server_error() {
        let err = DomainError::Provider("HTTP 500 Internal Server Error".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Server);
    }

    #[test]
    fn test_classify_auth_error() {
        let err = DomainError::Provider("HTTP 401 Unauthorized".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Auth);
    }

    #[test]
    fn test_classify_network_error() {
        let err = DomainError::Provider("connection timeout".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Network);

        let err2 = DomainError::Provider("network unreachable".to_string());
        assert_eq!(classify_error(&err2), ErrorClass::Network);

        let err3 = DomainError::Provider("connect refused".to_string());
        assert_eq!(classify_error(&err3), ErrorClass::Network);
    }

    #[test]
    fn test_classify_cancelled_error() {
        let err = DomainError::Provider("request cancelled".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Cancelled);
        assert!(!ErrorClass::Cancelled.is_retryable());
    }

    #[test]
    fn test_classify_unknown_error() {
        let err = DomainError::Provider("something unexpected happened".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Unknown);
    }

    #[test]
    fn test_classify_403_as_auth() {
        let err = DomainError::Provider("HTTP 403 Forbidden".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Auth);
    }

    #[test]
    fn test_classify_502_503_504() {
        for code in ["502", "503", "504"] {
            let err = DomainError::Provider(format!("HTTP {} Bad Gateway", code));
            assert_eq!(
                classify_error(&err),
                ErrorClass::Server,
                "expected Server for {}",
                code
            );
        }
    }

    #[test]
    fn test_classify_529_as_server_and_retryable_from_message() {
        let err = DomainError::Provider(
            "HTTP 529 from Anthropic: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}".to_string(),
        );
        let class = classify_error(&err);
        assert_eq!(class, ErrorClass::Server);
        assert!(class.is_retryable());
    }

    #[test]
    fn test_classify_overloaded_error_keyword_alone_is_server() {
        let err =
            DomainError::Provider("Anthropic returned overloaded_error, please retry".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Server);
    }

    #[test]
    fn test_classify_bare_overloaded_word_is_unknown() {
        let err = DomainError::Provider("system is overloaded".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Unknown);
    }

    #[test]
    fn test_non_provider_errors_are_unknown() {
        let err = DomainError::Tool("HTTP 500 from subprocess".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Unknown);
    }

    #[test]
    fn test_classify_auth_by_semantic_message() {
        let err = DomainError::Provider("Authentication failed: invalid api key".to_string());
        assert_eq!(classify_error(&err), ErrorClass::Auth);
    }

    #[test]
    fn test_status_extraction_prefers_http_context() {
        let err = DomainError::Provider(
            "connect to 10.0.0.1:443 failed, HTTP 503 Service Unavailable".to_string(),
        );
        assert_eq!(classify_error(&err), ErrorClass::Server);
    }
}
