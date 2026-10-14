use super::*;
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error, model_refusal,
};

#[test]
fn finite_http_payload_family() {
    for vendor in ["OpenAI", "Codex", "Anthropic"] {
        for (status, body, class) in [
            (
                402,
                r#"{"error":{"message":"secret-canary"}}"#,
                ProviderErrorClass::Billing,
            ),
            (
                500,
                r#"{"error":{"type":"invalid_request_error","message":"secret-canary"}}"#,
                ProviderErrorClass::Client,
            ),
            (
                429,
                r#"{"error":{"code":"insufficient_quota","message":"secret-canary"}}"#,
                ProviderErrorClass::Billing,
            ),
            (
                403,
                r#"{"detail":"Access to the chat endpoint is denied secret-canary"}"#,
                ProviderErrorClass::Auth,
            ),
            (500, "secret-canary malformed", ProviderErrorClass::Server),
        ] {
            let text = http(status, vendor, body);
            assert!(!text.contains("secret-canary"), "wire data escaped");
            assert!(text.len() <= BODY_LIMIT);
            assert_eq!(
                classify_provider_error(&DomainError::Provider(text.clone())),
                class
            );
            assert_eq!(model_refusal(&DomainError::Provider(text)), None);
        }
        let oversized = format!(
            r#"{{"error":{{"code":"model_not_found","message":"{}"}}}}"#,
            "s".repeat(BODY_LIMIT)
        );
        assert_eq!(
            http(404, vendor, &oversized),
            format!("HTTP 404 from {vendor}: class=client; refusal=none")
        );
    }
}

#[test]
fn finite_sse_shape_family() {
    for vendor in ["OpenAI", "Codex", "Anthropic"] {
        for (name, status) in [
            ("api_error", 500),
            ("internal_error", 500),
            ("service_unavailable_error", 503),
            ("timeout_error", 504),
            ("permission_error", 403),
            ("billing_error", 402),
            ("rate_limit_exceeded", 429),
        ] {
            for payload in [
                serde_json::json!({"error":{"code":name,"message":"secret-canary"}}),
                serde_json::json!({"type":"error","code":name,"message":"secret-canary"}),
                serde_json::json!({"type":"error","error":null,"code":name,"message":"secret-canary"}),
                serde_json::json!({"type":"response.failed","response":{"error":{"type":name,"message":"secret-canary"}}}),
            ] {
                let diagnostic = stream(vendor, &payload);
                assert_eq!(
                    diagnostic,
                    format!(
                        "HTTP {status} from {vendor}: class={}; refusal=none",
                        ProviderErrorClass::from_status(status)
                    )
                );
            }
        }
        let ignored_top = serde_json::json!({"type":"response.failed","code":"insufficient_quota","message":"secret-canary"});
        assert_eq!(
            stream(vendor, &ignored_top),
            format!("HTTP 502 from {vendor}: class=server; refusal=none")
        );
        let huge = serde_json::json!({"error":{"code":"insufficient_quota","message":"s".repeat(BODY_LIMIT)}});
        assert_eq!(
            stream(vendor, &huge),
            format!("HTTP 502 from {vendor}: class=server; refusal=none")
        );
    }
    assert_eq!(
        stream(
            "Codex",
            &serde_json::json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}})
        ),
        "Codex stream incomplete"
    );
}

#[test]
fn finite_payload_boundaries_and_refusal_controls() {
    for vendor in ["OpenAI", "Codex", "Anthropic"] {
        let core = r#"{"error":{"code":"insufficient_quota"}}"#;
        let at_limit = format!("{core}{}", " ".repeat(BODY_LIMIT - core.len()));
        assert_eq!(
            http(429, vendor, &at_limit),
            format!("HTTP 429 from {vendor}: class=billing; refusal=none")
        );
        assert_eq!(
            http(429, vendor, &(at_limit.clone() + " ")),
            format!("HTTP 429 from {vendor}: class=rate_limit; refusal=none")
        );
        for body in [
            r#"{"error":{"code":"insufficient_quota"}}junk"#,
            r#"{"error":{"code":"insufficient_quota"}"#,
        ] {
            assert_eq!(
                http(429, vendor, body),
                format!("HTTP 429 from {vendor}: class=rate_limit; refusal=none")
            );
        }
        for suffix in [" ", "\t", "\r", "\n"] {
            assert_eq!(
                stream_raw(vendor, &format!("{core}{suffix}")),
                format!("HTTP 402 from {vendor}: class=billing; refusal=none")
            );
        }
        for suffix in ["\u{00a0}", "\u{2003}", "\u{202f}"] {
            assert_eq!(
                stream_raw(vendor, &format!("{core}{suffix}")),
                format!("HTTP 502 from {vendor}: class=server; refusal=none")
            );
        }
        assert_eq!(
            stream_raw(vendor, &at_limit),
            format!("HTTP 402 from {vendor}: class=billing; refusal=none")
        );
        assert_eq!(
            stream_raw(vendor, &(at_limit + " ")),
            format!("HTTP 502 from {vendor}: class=server; refusal=none")
        );
    }
    for (status, vendor, body) in [
        (
            400,
            "Codex",
            r#"{"detail":"The 'm' model is not supported when using Codex with a ChatGPT account. secret-canary"}"#,
        ),
        (
            404,
            "OpenAI",
            r#"{"error":{"code":"model_not_found","message":"secret-canary"}}"#,
        ),
        (
            404,
            "Anthropic",
            r#"{"error":{"type":"not_found_error","message":"model: secret-canary"}}"#,
        ),
    ] {
        let err = DomainError::Provider(http(status, vendor, body));
        let reason = model_refusal(&err).unwrap();
        assert!(!reason.contains("secret-canary"));
        assert_eq!(
            model_refusal(&DomainError::Provider(http(500, vendor, body))),
            None
        );
        assert_eq!(
            model_refusal(&DomainError::Provider(http(401, vendor, body))),
            None
        );
        assert_eq!(
            model_refusal(&DomainError::Provider(http(
                status,
                vendor,
                &(body.to_string() + &" ".repeat(BODY_LIMIT))
            ))),
            None
        );
    }
    for (status, vendor, body) in [
        (
            404,
            "OpenAI",
            r#"{"error":{"type":"authentication_error","code":"model_not_found"}}"#,
        ),
        (
            404,
            "Anthropic",
            r#"{"error":{"type":"not_found_error","code":"insufficient_quota","message":"model: secret-canary"}}"#,
        ),
        (
            400,
            "Codex",
            r#"{"error":{"type":"permission_error","message":"The 'm' model is not supported when using Codex with a ChatGPT account."}}"#,
        ),
    ] {
        assert_eq!(
            model_refusal(&DomainError::Provider(http(status, vendor, body))),
            None
        );
    }
    let encrypted = r#"{"error":{"code":"invalid_encrypted_content","message":"secret-canary"}}"#;
    assert_eq!(
        http(400, "Codex", encrypted),
        "HTTP 400 from Codex: class=client; refusal=encrypted"
    );
    for (status, vendor, body) in [
        (500, "Codex", encrypted),
        (400, "OpenAI", encrypted),
        (
            400,
            "Codex",
            r#"{"error":{"message":"The encrypted content could not be verified"}}"#,
        ),
        (
            400,
            "Codex",
            r#"{"error":{"code":"unknown","message":"encrypted secret-canary"}}"#,
        ),
    ] {
        assert!(http(status, vendor, body).ends_with("refusal=none"));
    }
}

#[test]
fn finite_context_limit_family() {
    for vendor in ["OpenAI", "Codex", "Anthropic"] {
        let body = r#"{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"secret-canary"}}"#;
        for status in [400, 500] {
            let diagnostic = http(status, vendor, body);
            assert_eq!(
                diagnostic,
                format!(
                    "HTTP {status} from {vendor}: class=client; refusal=none; limit=context length"
                )
            );
            let error = DomainError::Provider(diagnostic);
            assert_eq!(classify_provider_error(&error), ProviderErrorClass::Client);
            assert_eq!(model_refusal(&error), None);
        }
        for (status, payload) in [
            (401, body),
            (403, body),
            (404, body),
            (
                400,
                r#"{"error":{"message":"context length exceeded secret-canary"}}"#,
            ),
            (500, r#"{"error":{"code":"context_length_exceeded"}}"#),
        ] {
            let diagnostic = http(status, vendor, payload);
            assert!(diagnostic.ends_with("refusal=none"));
            assert!(!diagnostic.contains("limit="));
        }
        let value: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            stream(vendor, &value),
            format!("HTTP 400 from {vendor}: class=client; refusal=none; limit=context length")
        );
    }
}

#[test]
fn provider_diagnostics_transport_nonstandard_status_is_unknown() {
    for status in [600, 999] {
        for vendor in ["OpenAI", "Codex", "Anthropic"] {
            for body in [String::new(), "{not json secret-canary".into(), "x".repeat(BODY_LIMIT + 1), r#"{"error":{"type":"unknown","code":"model_not_found","message":"secret-canary"}}"#.into()] {
                let diagnostic = http(status, vendor, &body);
                assert_eq!(diagnostic, format!("HTTP {status} from {vendor}: class=unknown; refusal=none"));
                assert_eq!(classify_provider_error(&DomainError::Provider(diagnostic)), ProviderErrorClass::Unknown);
            }
            // Existing exact billing authority stays finite and terminal, regardless
            // of HTTP class; nonstandard statuses cannot establish model refusal.
            let diagnostic = http(
                status,
                vendor,
                r#"{"error":{"code":"insufficient_quota","message":"secret-canary"}}"#,
            );
            assert_eq!(
                diagnostic,
                format!("HTTP {status} from {vendor}: class=billing; refusal=none")
            );
            let error = DomainError::Provider(diagnostic);
            assert_eq!(classify_provider_error(&error), ProviderErrorClass::Billing);
            assert_eq!(model_refusal(&error), None);
        }
    }
}

#[test]
fn provider_diagnostics_each_recognized_declaration_must_allow_refusal() {
    for incompatible in [
        "server_error",
        "api_error",
        "internal_error",
        "internal_server_error",
        "service_unavailable",
        "service_unavailable_error",
        "timeout_error",
        "rate_limit_error",
        "rate_limit_exceeded",
        "overloaded_error",
        "authentication_error",
        "permission_error",
        "insufficient_quota",
        "billing_error",
        "billing_hard_limit_reached",
        "usage_limit_reached",
        "invalid_api_key",
        "request_too_large",
    ] {
        for (vendor, status, refusal) in [
            ("OpenAI", 404, "model_not_found"),
            ("Anthropic", 404, "not_found_error"),
            ("Codex", 400, "invalid_request_error"),
        ] {
            for (kind, code) in [(incompatible, refusal), (refusal, incompatible)] {
                let body = serde_json::json!({"detail":"model is not supported when using Codex with a ChatGPT account", "error":{"type":kind,"code":code,"message":"model: secret-canary"}}).to_string();
                assert_eq!(
                    model_refusal(&DomainError::Provider(http(status, vendor, &body))),
                    None,
                    "{body}"
                );
            }
        }
    }
    for (kind, code) in [
        ("unknown", "model_not_found"),
        ("model_not_found", "unknown"),
    ] {
        let body = serde_json::json!({"error":{"type":kind,"code":code}}).to_string();
        assert!(model_refusal(&DomainError::Provider(http(404, "OpenAI", &body))).is_some());
    }
}
