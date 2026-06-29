use crate::domain::error::DomainError;

/// Stable provider error classes used by application-layer retry decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderErrorClass {
    RateLimit,
    Auth,
    Server,
    Client,
    Network,
    Cancelled,
    Unknown,
}

impl ProviderErrorClass {
    pub fn from_status(status: u16) -> Self {
        match status {
            401 | 403 => Self::Auth,
            429 => Self::RateLimit,
            400 | 404 | 405 | 406 | 409 | 410 | 422 => Self::Client,
            500 | 502 | 503 | 504 | 529 => Self::Server,
            _ => Self::Unknown,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimit | Self::Server | Self::Network)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::Auth => "auth",
            Self::Server => "server",
            Self::Client => "client",
            Self::Network => "network",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ProviderErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn classify_provider_error(err: &DomainError) -> ProviderErrorClass {
    let msg = match err {
        DomainError::Provider(msg) => msg.as_str(),
        _ => return ProviderErrorClass::Unknown,
    };

    let lowered = msg.to_ascii_lowercase();

    if let Some(status) = extract_http_status(&lowered) {
        return ProviderErrorClass::from_status(status);
    }

    if let Some(status) = extract_parenthesized_status(&lowered) {
        return ProviderErrorClass::from_status(status);
    }

    classify_keyword_paths(&lowered)
}

/// Recognise a parenthesised HTTP status such as `provider error (400): ...`,
/// which some error strings use instead of an `HTTP <code>` prefix.
fn extract_parenthesized_status(lowered: &str) -> Option<u16> {
    let mut search_from = 0;
    while let Some(rel) = lowered[search_from..].find('(') {
        let open = search_from + rel;
        let after = &lowered[open + 1..];
        let bytes = after.as_bytes();
        if bytes.len() >= 4
            && bytes[0].is_ascii_digit()
            && bytes[1].is_ascii_digit()
            && bytes[2].is_ascii_digit()
            && bytes[3] == b')'
        {
            let code = ((bytes[0] - b'0') as u16) * 100
                + ((bytes[1] - b'0') as u16) * 10
                + ((bytes[2] - b'0') as u16);
            if (100..=599).contains(&code) {
                return Some(code);
            }
        }
        search_from = open + 1;
    }
    None
}

fn classify_keyword_paths(lowered: &str) -> ProviderErrorClass {
    if lowered.contains("request cancelled") || lowered.contains("request canceled") {
        ProviderErrorClass::Cancelled
    } else if lowered.contains("rate limit") {
        ProviderErrorClass::RateLimit
    } else if lowered.contains("auth")
        || lowered.contains("unauthorized")
        || lowered.contains("forbidden")
        || lowered.contains("invalid api key")
        || lowered.contains("authentication")
    {
        ProviderErrorClass::Auth
    } else if lowered.contains("internal server error")
        || lowered.contains("bad gateway")
        || lowered.contains("service unavailable")
        || lowered.contains("gateway timeout")
        || lowered.contains("overloaded_error")
    {
        ProviderErrorClass::Server
    } else if lowered.contains("connect")
        || lowered.contains("connection")
        || lowered.contains("timeout")
        || lowered.contains("timed out")
        || lowered.contains("network")
        || lowered.contains("dns")
    {
        ProviderErrorClass::Network
    } else {
        ProviderErrorClass::Unknown
    }
}

fn extract_http_status(lowered: &str) -> Option<u16> {
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

#[cfg(test)]
#[path = "provider_error_tests.rs"]
mod tests;
