use super::{Profile, Surface, Vendor, openai_stream_error};
use crate::domain::error::DomainError;

/// A real `reqwest::Error` of the send kind: a connection to a closed
/// loopback port is refused deterministically.
async fn send_error() -> reqwest::Error {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap()
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .expect_err("connection to closed port must fail")
}

fn message(error: DomainError) -> String {
    match error {
        DomainError::Provider(message) => message,
        other => panic!("expected a provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn each_vendor_keeps_its_own_send_and_read_error_wording() {
    let error = send_error().await;
    let openai = Profile::new(Vendor::OpenAi, Surface::Chat, Default::default());
    let codex = Profile::new(Vendor::Codex, Surface::Incremental, Default::default());
    let anthropic = Profile::new(Vendor::Anthropic, Surface::Assembled, Default::default());
    assert_eq!(openai.name(), "OpenAI");
    assert_eq!(codex.name(), "Codex");
    assert_eq!(anthropic.name(), "Anthropic");

    let sent = message(openai.send_error(&error));
    assert!(sent.starts_with("HTTP error: "), "{sent}");
    assert!(
        sent.len() > format!("HTTP error: {error}").len(),
        "the source chain is appended for OpenAI: {sent}"
    );
    let sent = message(codex.send_error(&error));
    assert!(sent.starts_with("Codex request failed: "), "{sent}");
    let sent = message(anthropic.send_error(&error));
    assert_eq!(
        sent,
        format!("HTTP error: {error}"),
        "Anthropic keeps the bare message"
    );

    // Only Anthropic's assembled surface calls the body a stream.
    assert_eq!(
        message(anthropic.read_error(&error)),
        format!("failed to read stream: {error}")
    );
    assert_eq!(
        message(
            Profile::new(Vendor::Anthropic, Surface::Chat, Default::default()).read_error(&error)
        ),
        format!("failed to read response: {error}")
    );
    assert_eq!(
        message(codex.read_error(&error)),
        format!("failed to read response: {error}")
    );
}

#[test]
fn the_chat_surfaces_of_openai_and_anthropic_are_strict_about_error_bodies() {
    assert!(Profile::new(Vendor::OpenAi, Surface::Chat, Default::default()).strict_error_body());
    assert!(Profile::new(Vendor::Anthropic, Surface::Chat, Default::default()).strict_error_body());
    assert!(!Profile::new(Vendor::Codex, Surface::Chat, Default::default()).strict_error_body());
    assert!(
        !Profile::new(Vendor::OpenAi, Surface::Incremental, Default::default()).strict_error_body()
    );
    assert!(
        !Profile::new(Vendor::Anthropic, Surface::Assembled, Default::default())
            .strict_error_body()
    );
}

#[test]
fn the_openai_stream_error_status_comes_from_typed_fields_never_the_message() {
    let status = |value: serde_json::Value| {
        openai_stream_error(&value)
            .strip_prefix("HTTP ")
            .and_then(|rest| rest.split(' ').next())
            .map(str::to_string)
            .unwrap()
    };
    assert_eq!(
        status(
            serde_json::json!({"error": {"type": "authentication_error", "message": "rate limit"}})
        ),
        "401"
    );
    assert_eq!(
        status(serde_json::json!({"error": {"code": "invalid_api_key"}})),
        "401"
    );
    assert_eq!(
        status(serde_json::json!({"error": {"type": "invalid_request_error"}})),
        "400"
    );
    assert_eq!(
        status(serde_json::json!({"error": {"type": "overloaded_error"}})),
        "529"
    );
    assert_eq!(
        status(serde_json::json!({"error": {"type": "rate_limit_error"}})),
        "429"
    );
    assert_eq!(
        status(
            serde_json::json!({"error": {"type": "server_error", "message": "401 unauthorized"}})
        ),
        "500"
    );
    let rendered = openai_stream_error(&serde_json::json!({"error": {"type": "rate_limit_error"}}));
    assert!(
        rendered.ends_with(r#"OpenAI stream error: {"error":{"type":"rate_limit_error"}}"#),
        "{rendered}"
    );
}

/// #2210: only a streaming reply's steps are bounded, by the provider's own
/// bound; a whole non-streaming reply sends nothing until complete.
#[tokio::test(start_paused = true)]
async fn only_streaming_surfaces_bound_a_silent_step() {
    use crate::infrastructure::providers::stream_idle::{STREAM_IDLE_LIMIT, StreamIdle};
    use std::time::Duration;
    let bound = StreamIdle::new(Duration::from_secs(5));
    for (surface, streams) in [
        (Surface::Chat, false),
        (Surface::Assembled, true),
        (Surface::Incremental, true),
    ] {
        let profile = Profile::new(Vendor::Codex, surface, Default::default());
        assert_eq!(profile.idle.limit(), STREAM_IDLE_LIMIT);
        let profile = Profile::new(Vendor::Codex, surface, bound);
        assert_eq!(profile.idle, bound);
        assert_eq!(profile.streams(), streams);
        let late = tokio::time::sleep(Duration::from_secs(6));
        assert_eq!(profile.within(late).await.is_ok(), !streams, "{streams}");
    }
}

/// The status an OpenAI-compatible mid-stream error chunk is rendered with.
fn chunk_status(value: serde_json::Value) -> u16 {
    openai_stream_error(&value)
        .strip_prefix("HTTP ")
        .and_then(|rest| rest.split(' ').next())
        .and_then(|status| status.parse().ok())
        .unwrap()
}

/// #2155: a numeric `error.code` (OpenRouter's shape) is the status when the
/// retry classifier knows it; anything else is no status at all.
#[test]
fn a_numeric_error_code_is_the_status_only_when_it_is_an_error_status() {
    for code in [400, 401, 403, 404, 422, 429, 500, 502, 503, 504, 529] {
        assert_eq!(
            chunk_status(serde_json::json!({"error": {"code": code, "message": "m"}})),
            code,
            "{code}"
        );
    }
    // A typed client error wins over a numeric status.
    assert_eq!(
        chunk_status(serde_json::json!({"error": {"code": 503, "type": "invalid_request_error"}})),
        400
    );
    for code in [
        serde_json::json!(0),
        serde_json::json!(99),
        serde_json::json!(100),
        serde_json::json!(200),
        serde_json::json!(399),
        serde_json::json!(600),
        serde_json::json!(4_294_967_798_u64),
        serde_json::json!(-400),
        serde_json::json!(502.5),
        serde_json::json!("502"),
    ] {
        assert_eq!(
            chunk_status(serde_json::json!({"error": {"code": code}})),
            502,
            "an unusable code {code} is an unknown error"
        );
    }
}

/// #2155: provider-side error types are server errors (retryable), and an
/// error of a type nobody knows is a bad gateway: retryable, never a
/// client error.
#[test]
fn server_side_and_unknown_error_types_are_retryable_server_statuses() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
    let cases = [
        (serde_json::json!({"type": "server_error"}), 500),
        (serde_json::json!({"code": "server_error"}), 500),
        (serde_json::json!({"type": "api_error"}), 500),
        (serde_json::json!({"type": "internal_error"}), 500),
        (serde_json::json!({"code": "internal_server_error"}), 500),
        (serde_json::json!({"type": "service_unavailable"}), 503),
        (
            serde_json::json!({"code": "service_unavailable_error"}),
            503,
        ),
        (serde_json::json!({"type": "timeout_error"}), 504),
        (serde_json::json!({"type": "never_heard_of_it"}), 502),
        (serde_json::json!({"message": "no type at all"}), 502),
        (serde_json::json!({}), 502),
    ];
    for (error, expected) in cases {
        let value = serde_json::json!({ "error": error });
        assert_eq!(chunk_status(value.clone()), expected, "{value}");
        let class = classify_provider_error(&DomainError::Provider(openai_stream_error(&value)));
        assert_eq!(class, ProviderErrorClass::Server, "{value}");
        assert!(class.is_retryable(), "{value}");
    }
}

/// #2155: only the allowlisted client error types are client statuses,
/// which are never retried; a billing error stays billing.
#[test]
fn known_client_error_types_are_client_statuses_that_are_never_retried() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
    let cases = [
        (
            serde_json::json!({"type": "invalid_request_error"}),
            400,
            ProviderErrorClass::Client,
        ),
        (
            serde_json::json!({"code": "context_length_exceeded"}),
            400,
            ProviderErrorClass::Client,
        ),
        (
            serde_json::json!({"type": "authentication_error"}),
            401,
            ProviderErrorClass::Auth,
        ),
        (
            serde_json::json!({"code": "invalid_api_key"}),
            401,
            ProviderErrorClass::Auth,
        ),
        (
            serde_json::json!({"type": "permission_error"}),
            403,
            ProviderErrorClass::Auth,
        ),
        (
            serde_json::json!({"type": "not_found_error"}),
            404,
            ProviderErrorClass::Client,
        ),
        (
            serde_json::json!({"code": "model_not_found"}),
            404,
            ProviderErrorClass::Client,
        ),
        (
            serde_json::json!({"type": "request_too_large"}),
            413,
            ProviderErrorClass::Unknown,
        ),
        (
            serde_json::json!({"type": "insufficient_quota"}),
            402,
            ProviderErrorClass::Billing,
        ),
        (
            serde_json::json!({"type": "billing_error"}),
            402,
            ProviderErrorClass::Billing,
        ),
        (
            serde_json::json!({"type": "rate_limit_error", "code": "insufficient_quota"}),
            402,
            ProviderErrorClass::Billing,
        ),
        (
            serde_json::json!({"type": "invalid_request_error", "code": "invalid_api_key"}),
            401,
            ProviderErrorClass::Auth,
        ),
        (
            serde_json::json!({"type": "server_error", "code": "invalid_request_error"}),
            400,
            ProviderErrorClass::Client,
        ),
    ];
    for (error, expected, class) in cases {
        let value = serde_json::json!({ "error": error });
        assert_eq!(chunk_status(value.clone()), expected, "{value}");
        let classified =
            classify_provider_error(&DomainError::Provider(openai_stream_error(&value)));
        assert_eq!(classified, class, "{value}");
        assert!(!classified.is_retryable(), "{value}");
    }
    // Throttles keep their retryable statuses.
    assert_eq!(
        chunk_status(serde_json::json!({"error": {"code": "rate_limit_exceeded"}})),
        429
    );
    assert_eq!(
        chunk_status(serde_json::json!({"error": {"type": "overloaded_error"}})),
        529
    );
}

/// #2155 review: a chunk throttles when its status is 429 or 529, typed or
/// numeric, unless it declares a billing error.
#[test]
fn a_throttle_chunk_is_a_429_or_529_that_is_not_billing() {
    use super::is_throttle_chunk;
    for (error, throttle) in [
        (serde_json::json!({"code": 429}), true),
        (serde_json::json!({"code": 529}), true),
        (serde_json::json!({"code": "rate_limit_exceeded"}), true),
        (serde_json::json!({"type": "overloaded_error"}), true),
        (
            serde_json::json!({"code": 429, "type": "insufficient_quota"}),
            false,
        ),
        (
            serde_json::json!({"code": 429, "code_": "x", "type": "usage_limit_reached"}),
            false,
        ),
        (
            serde_json::json!({"code": 529, "type": "billing_hard_limit_reached"}),
            false,
        ),
        (
            serde_json::json!({"code": 429, "type": "billing_error"}),
            false,
        ),
        (serde_json::json!({"code": 428}), false),
        (serde_json::json!({"code": 530}), false),
        (serde_json::json!({"code": 503}), false),
        (serde_json::json!({"type": "server_error"}), false),
    ] {
        let value = serde_json::json!({ "error": error });
        assert_eq!(is_throttle_chunk(&value), throttle, "{value}");
    }
}

/// #2155 review: a numeric code the classifier does not know is never left
/// as a terminal unknown status when it is a provider failure: a 408
/// (OpenRouter's timeout) is a gateway timeout and an unknown 5xx
/// (Cloudflare's 520-524) a bad gateway, both retried end to end. An unknown
/// 4xx lets the typed server rules speak, then stands as itself: terminal.
#[test]
fn unknown_numeric_codes_map_to_what_the_classifier_retries() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
    let class = |value: &serde_json::Value| {
        classify_provider_error(&DomainError::Provider(openai_stream_error(value)))
    };
    for (code, status) in [(408, 504), (520, 502), (521, 502), (524, 502), (599, 502)] {
        let value = serde_json::json!({"error": {"code": code, "message": "a timeout occurred"}});
        assert_eq!(chunk_status(value.clone()), status, "{value}");
        assert_eq!(class(&value), ProviderErrorClass::Server, "{value}");
        assert!(class(&value).is_retryable(), "{value}");
    }
    for code in [402, 413, 499] {
        let value = serde_json::json!({"error": {"code": code, "message": "m"}});
        assert_eq!(chunk_status(value.clone()), code, "{value}");
        assert!(!class(&value).is_retryable(), "{value}");
    }
    // An unknown 4xx that names a server error is that server error.
    let value = serde_json::json!({"error": {"code": 499, "type": "server_error"}});
    assert_eq!(chunk_status(value.clone()), 500);
    assert!(class(&value).is_retryable());
}

/// #2155 review: a typed throttle or client type wins over a numeric code,
/// so a throttle is never classified as a malformed request, nor a
/// malformed request as a throttle.
#[test]
fn typed_fields_win_over_a_numeric_code() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
    for (error, status, class) in [
        (
            serde_json::json!({"code": 400, "type": "overloaded_error"}),
            529,
            ProviderErrorClass::Server,
        ),
        (
            serde_json::json!({"code": 400, "type": "rate_limit_error"}),
            429,
            ProviderErrorClass::RateLimit,
        ),
        (
            serde_json::json!({"code": 429, "type": "invalid_request_error"}),
            400,
            ProviderErrorClass::Client,
        ),
        (
            serde_json::json!({"code": 429, "type": "insufficient_quota"}),
            402,
            ProviderErrorClass::Billing,
        ),
    ] {
        let value = serde_json::json!({ "error": error });
        assert_eq!(chunk_status(value.clone()), status, "{value}");
        assert_eq!(
            classify_provider_error(&DomainError::Provider(openai_stream_error(&value))),
            class,
            "{value}"
        );
    }
}

/// #2155 review: every layer reads the one billing list: each name is a
/// billing chunk (402), never a throttle, and classified Billing.
#[test]
fn every_billing_name_is_billing_everywhere() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{
        BILLING_ERROR_NAMES, ProviderErrorClass, classify_provider_error,
    };
    for name in BILLING_ERROR_NAMES {
        for field in ["type", "code"] {
            let mut error = serde_json::json!({"type": "rate_limit_error", "code": 429});
            error[field] = serde_json::json!(name);
            let value = serde_json::json!({ "error": error });
            assert_eq!(chunk_status(value.clone()), 402, "{value}");
            assert!(!super::is_throttle_chunk(&value), "{value}");
            assert!(
                !super::super::admission_feedback::is_typed_throttle(&value),
                "{value}"
            );
            assert_eq!(
                classify_provider_error(&DomainError::Provider(openai_stream_error(&value))),
                ProviderErrorClass::Billing,
                "{value}"
            );
        }
    }
}

/// #2155 PR review: a typed server error wins over a known numeric 4xx, so
/// the chunk is retried; a client type still wins over a numeric 5xx (#935).
#[test]
fn a_typed_server_error_wins_over_a_numeric_client_code() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
    for (error, status, class) in [
        (
            serde_json::json!({"type": "server_error", "code": 400}),
            500,
            ProviderErrorClass::Server,
        ),
        (
            serde_json::json!({"type": "api_error", "code": 404}),
            500,
            ProviderErrorClass::Server,
        ),
        (
            serde_json::json!({"type": "timeout_error", "code": 422}),
            504,
            ProviderErrorClass::Server,
        ),
        (
            serde_json::json!({"type": "invalid_request_error", "code": 503}),
            400,
            ProviderErrorClass::Client,
        ),
    ] {
        let value = serde_json::json!({ "error": error });
        assert_eq!(chunk_status(value.clone()), status, "{value}");
        let classified =
            classify_provider_error(&DomainError::Provider(openai_stream_error(&value)));
        assert_eq!(classified, class, "{value}");
        assert_eq!(
            classified.is_retryable(),
            class == ProviderErrorClass::Server,
            "{value}"
        );
    }
}

/// #2155 PR review: a bare numeric 402 (OpenRouter's "Insufficient
/// credits") is a billing failure: never retried.
#[test]
fn a_bare_numeric_402_chunk_is_billing() {
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
    let value = serde_json::json!({"error": {"code": 402, "message": "Insufficient credits"}});
    assert_eq!(chunk_status(value.clone()), 402);
    assert!(!super::is_throttle_chunk(&value));
    assert_eq!(
        classify_provider_error(&DomainError::Provider(openai_stream_error(&value))),
        ProviderErrorClass::Billing
    );
}

/// #2236: a bare string `error` (Ollama's native shape) is an error chunk
/// nobody typed: the unknown 502, classified exactly as the object form of
/// an unknown error, so the retry owner retries it before output and keeps
/// it terminal after. Only an object or a string `error` is an error chunk.
#[test]
fn a_bare_string_error_chunk_is_classified_as_the_unknown_object_form() {
    use super::is_stream_error_chunk;
    use crate::domain::error::DomainError;
    use crate::domain::provider_error::classify_provider_error;
    let string = serde_json::json!({"error": "model crashed"});
    let object = serde_json::json!({"error": {"message": "model crashed"}});
    assert_eq!(chunk_status(string.clone()), 502);
    assert_eq!(chunk_status(object.clone()), 502);
    let class = |value: &serde_json::Value| {
        classify_provider_error(&DomainError::Provider(openai_stream_error(value)))
    };
    assert_eq!(class(&string), class(&object));
    assert!(class(&string).is_retryable());
    assert!(openai_stream_error(&string).contains("model crashed"));
    for (value, chunk) in [
        (string, true),
        (object, true),
        (serde_json::json!({"error": "x"}), true),
        (serde_json::json!({"error": {"code": 500}}), true),
        (serde_json::json!({"error": ""}), false),
        // Any object is an error (#2249 review round 3): `{}` names no
        // cause, so it is the unknown, retryable 502.
        (serde_json::json!({"error": {}}), true),
        (serde_json::json!({"error": null}), false),
        (serde_json::json!({"error": 42}), false),
        (serde_json::json!({"error": true}), false),
        (serde_json::json!({"error": ["x"]}), false),
        (serde_json::json!({"message": "no error field"}), false),
        (serde_json::json!("error"), false),
    ] {
        assert_eq!(is_stream_error_chunk(&value), chunk, "{value}");
    }
}

/// #2236: only an error chunk renders as a stream error; rendering any
/// other chunk is a caller bug, refused loudly.
#[test]
#[should_panic(expected = "only an error chunk renders as a stream error")]
fn a_chunk_without_an_error_never_renders_as_a_stream_error() {
    let _ = openai_stream_error(&serde_json::json!({"error": null}));
}
