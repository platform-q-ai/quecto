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
    assert!(openai.openai() && !codex.openai() && !anthropic.openai());

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
        "400"
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
