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
}
