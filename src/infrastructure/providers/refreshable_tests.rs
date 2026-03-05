use super::*;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// A mock provider that returns 401 on the first N calls, then succeeds.
#[derive(Debug)]
struct MockRetryProvider {
    call_count: Arc<AtomicU32>,
    fail_until: u32,
}

impl MockRetryProvider {
    fn new(call_count: Arc<AtomicU32>, fail_until: u32) -> Self {
        Self {
            call_count,
            fail_until,
        }
    }
}

impl LlmProvider for MockRetryProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if count < self.fail_until {
                Err(DomainError::Provider(
                    "provider error (401): unauthorized".to_string(),
                ))
            } else {
                Ok(LlmResponse {
                    content: Some("success".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                })
            }
        })
    }
}

/// A mock provider that always returns 500.
#[derive(Debug)]
struct MockServerErrorProvider;

impl LlmProvider for MockServerErrorProvider {
    fn name(&self) -> &str {
        "mock-500"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Err(DomainError::Provider(
                "provider error (500): internal server error".to_string(),
            ))
        })
    }
}

/// A mock provider that always succeeds.
#[derive(Debug)]
struct MockSuccessProvider;

impl LlmProvider for MockSuccessProvider {
    fn name(&self) -> &str {
        "mock-ok"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("ok".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
            })
        })
    }
}

fn test_request() -> ChatRequest<'static> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    }
}

fn make_mock_refresh(new_token: &str) -> RefreshFn {
    let token = new_token.to_string();
    Arc::new(move |store, provider_name| {
        let token = token.clone();
        let store = store.clone();
        let provider_name = provider_name.to_string();
        Box::pin(async move {
            store
                .store(Credential {
                    provider: provider_name,
                    token: token.clone(),
                    method: AuthMethod::OAuth,
                    expires_at: Some(i64::MAX),
                    refresh_token: Some("rt-new".to_string()),
                    account_id: None,
                })
                .map_err(|e| DomainError::Provider(format!("store error: {}", e)))?;
            Ok(token)
        })
    })
}

/// Factory that creates a MockRetryProvider sharing the same call counter.
fn make_mock_factory(call_count: Arc<AtomicU32>, fail_until: u32) -> ProviderFactory {
    Arc::new(move |_new_token| {
        Arc::new(MockRetryProvider::new(call_count.clone(), fail_until)) as Arc<dyn LlmProvider>
    })
}

fn noop_factory() -> ProviderFactory {
    Arc::new(|_| Arc::new(MockSuccessProvider) as Arc<dyn LlmProvider>)
}

#[tokio::test]
async fn test_refreshable_retries_on_401() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));

    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt-old".to_string()),
            account_id: None,
        })
        .unwrap();

    let call_count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(MockRetryProvider::new(call_count.clone(), 1));
    let factory = make_mock_factory(call_count, 1);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store.clone(),
        provider_name: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("sk-ant-oat01-fresh"),
        factory,
    });

    let result = refreshable.chat(test_request()).await;
    assert!(result.is_ok(), "should succeed after refresh: {:?}", result);
    assert_eq!(result.unwrap().content.unwrap(), "success");

    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("anthropic").unwrap();
    assert_eq!(cred.token, "sk-ant-oat01-fresh");
}

#[tokio::test]
async fn test_refreshable_passes_through_non_401_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));

    let inner = Arc::new(MockServerErrorProvider);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store,
        provider_name: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("unused"),
        factory: noop_factory(),
    });

    let result = refreshable.chat(test_request()).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("500"), "expected 500 error, got: {}", err);
}

#[tokio::test]
async fn test_refreshable_passes_through_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));

    let inner = Arc::new(MockSuccessProvider);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store,
        provider_name: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("unused"),
        factory: noop_factory(),
    });

    let result = refreshable.chat(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().content.unwrap(), "ok");
}

#[tokio::test]
async fn test_refreshable_does_not_retry_when_no_oauth_credential() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));

    let call_count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(MockRetryProvider::new(call_count.clone(), 999));
    let factory = make_mock_factory(call_count, 999);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store,
        provider_name: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("unused"),
        factory,
    });

    let result = refreshable.chat(test_request()).await;
    assert!(result.is_err(), "should fail when no credential to refresh");
}

#[tokio::test]
async fn test_refreshable_rebuilds_provider_with_new_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));

    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "old-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();

    let captured_token = Arc::new(std::sync::Mutex::new(String::new()));
    let captured = captured_token.clone();
    let factory: ProviderFactory = Arc::new(move |new_token| {
        *captured.lock().unwrap() = new_token.to_string();
        Arc::new(MockSuccessProvider) as Arc<dyn LlmProvider>
    });

    let call_count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(MockRetryProvider::new(call_count, 1));
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store.clone(),
        provider_name: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("new-api-token"),
        factory,
    });

    let result = refreshable.chat(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(*captured_token.lock().unwrap(), "new-api-token");
}
