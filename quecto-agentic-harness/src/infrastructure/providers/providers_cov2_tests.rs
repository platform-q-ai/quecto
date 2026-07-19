use super::*;

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};

#[test]
fn create_openai_provider_rejects_remote_http_base() {
    let err = create_openai_provider_with_client(
        "sk-test".to_string(),
        Some("http://example.com/v1".to_string()),
        reqwest::Client::new(),
        true,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ProviderFactoryError::InvalidApiBase { ref provider, .. } if provider == "openai"
    ));
    assert!(err.to_string().contains("invalid api_base for openai"));
}

#[test]
fn create_openai_compatible_provider_accepts_loopback_http_and_names_provider() {
    let provider = create_openai_compatible_provider(
        "local-ai",
        "sk-test".to_string(),
        "http://127.0.0.1:54321/v1".to_string(),
        false,
        reqwest::Client::new(),
    )
    .unwrap();
    assert_eq!(provider.name(), "local-ai");
}

#[derive(Debug)]
struct DowncastInnerProvider;

impl LlmProvider for DowncastInnerProvider {
    fn name(&self) -> &str {
        "inner"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>,
    > {
        Box::pin(async { Err(DomainError::Provider("no network".into())) })
    }
}

fn test_request() -> ChatRequest<'static> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "stub",
        max_tokens: 8,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[tokio::test]
async fn downcast_inner_provider_trait_surface_defaults_are_exercised() {
    use crate::domain::provider::StreamEvent;

    let provider = DowncastInnerProvider;
    assert_eq!(provider.name(), "inner");
    assert!(
        provider
            .as_any()
            .downcast_ref::<DowncastInnerProvider>()
            .is_some()
    );
    let err = provider.chat(test_request()).await.unwrap_err();
    assert!(err.to_string().contains("no network"));
    let stream_err = provider.chat_stream(test_request()).await.unwrap_err();
    assert!(stream_err.to_string().contains("no network"));
    let mut rx = provider.chat_stream_incremental(test_request()).await;
    assert!(matches!(
        rx.recv().await,
        Some(StreamEvent::Error(message)) if message.contains("no network")
    ));
    assert!(rx.recv().await.is_none());
}

#[test]
#[serial_test::serial]
fn env_allows_custom_provider_hosts_when_truthy() {
    // SAFETY: this test mutates a process-wide environment variable and restores it before return.
    unsafe { std::env::set_var("QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS", "true") };
    assert!(super::allow_custom_provider_hosts());
    assert!(super::allowed_https_host("openai", "example.com"));
    // SAFETY: restore the test-owned process environment variable.
    unsafe { std::env::remove_var("QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS") };
}
