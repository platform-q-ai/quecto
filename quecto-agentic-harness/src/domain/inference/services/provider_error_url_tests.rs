//! #2653: a status is read only from a status position, never from inside a
//! URL, so a connect failure to an IP-address endpoint whose first octet has
//! three digits is `Network` (retried) with no status, not `Unknown` with 100.

use super::*;

fn provider(message: &str) -> DomainError {
    DomainError::Provider(message.to_string())
}

#[test]
fn a_connect_failure_to_an_ip_address_endpoint_is_network_with_no_status() {
    for message in [
        // The message from the issue (a Tailscale `100.x` endpoint).
        "provider error: HTTP error: error sending request for url \
         (http://100.83.173.59:8888/v1/chat/completions): client error (Connect): \
         tcp connect error: deadline has elapsed",
        "HTTP error: error sending request for url \
         (http://192.168.1.5:8080/v1/chat/completions): client error (Connect): \
         tcp connect error: Connection refused (os error 111)",
        "HTTP error: error sending request for url (https://172.16.0.4/v1/chat/completions)",
    ] {
        let error = provider(message);
        assert_eq!(
            classify_provider_error(&error),
            ProviderErrorClass::Network,
            "{message}"
        );
        assert_eq!(provider_http_status(&error), None, "{message}");
    }
}

#[test]
fn a_real_status_is_still_read_beside_an_ip_address_url() {
    let body = "HTTP 503 Service Unavailable: {\"error\":{\"message\":\"overloaded\"}}";
    assert_eq!(
        classify_provider_error(&provider(body)),
        ProviderErrorClass::Server
    );
    assert_eq!(provider_http_status(&provider(body)), Some(503));
    for message in [
        "HTTP 503 from http://192.168.1.5:8080/v1/chat/completions: upstream overloaded",
        "request to http://100.83.173.59:8888/v1/chat/completions failed: HTTP 503",
        "request to http://100.83.173.59:8888/status/200 failed with status: 503",
    ] {
        let error = provider(message);
        assert_eq!(provider_http_status(&error), Some(503), "{message}");
        assert_eq!(
            classify_provider_error(&error),
            ProviderErrorClass::Server,
            "{message}"
        );
    }
}

#[test]
fn a_status_is_exactly_three_digits() {
    for message in ["status 1000 items", "HTTP 5034 weirdness", "code: 4290"] {
        assert_eq!(provider_http_status(&provider(message)), None, "{message}");
    }
}
