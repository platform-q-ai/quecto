use super::*;
use crate::domain::message::LlmResponse;
use crate::domain::provider::ChatRequest;
use crate::infrastructure::auth::credential_store::Credential;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Barrier;

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
        "mock-retry"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        let fail_until = self.fail_until;
        Box::pin(async move {
            if count < fail_until {
                Err(DomainError::Provider("HTTP 401 unauthorized".to_string()))
            } else {
                Ok(LlmResponse {
                    content: Some("ok".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                    thinking_blocks: vec![],
                })
            }
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
        effort: None,
    }
}

#[tokio::test]
async fn concurrent_401s_coalesce_oauth_refresh() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "old-access".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("old-refresh".to_string()),
            account_id: None,
        })
        .unwrap();

    let refresh_calls = Arc::new(AtomicU32::new(0));
    let refresh_fn: RefreshFn = Arc::new({
        let refresh_calls = refresh_calls.clone();
        move |store, provider| {
            let provider = provider.to_string();
            let refresh_calls = refresh_calls.clone();
            Box::pin(async move {
                refresh_calls.fetch_add(1, Ordering::SeqCst);
                store
                    .store_refreshed(
                        Credential {
                            provider: provider.clone(),
                            token: "fresh-access".to_string(),
                            method: AuthMethod::OAuth,
                            expires_at: Some(9999999999),
                            refresh_token: Some("fresh-refresh".to_string()),
                            account_id: None,
                        },
                        "old-refresh",
                    )
                    .map_err(|e| DomainError::Provider(format!("store error: {e}")))?;
                Ok("fresh-access".to_string())
            })
        }
    });

    let calls = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(MockRetryProvider::new(calls.clone(), 2));
    let factory: ProviderFactory = Arc::new(move |_| {
        Arc::new(MockRetryProvider::new(calls.clone(), 1)) as Arc<dyn LlmProvider>
    });
    let provider = Arc::new(RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store.clone(),
        provider_name: "anthropic".to_string(),
        credential_provider: "anthropic".to_string(),
        refresh_fn,
        factory,
    }));

    let barrier = Arc::new(Barrier::new(2));
    let first = {
        let provider = provider.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            provider.chat(test_request()).await
        })
    };
    let second = {
        let provider = provider.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            provider.chat(test_request()).await
        })
    };

    assert!(first.await.unwrap().is_ok());
    assert!(second.await.unwrap().is_ok());
    assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
    let cred = store.load_snapshot().unwrap().remove("anthropic").unwrap();
    assert_eq!(cred.token, "fresh-access");
    assert_eq!(cred.refresh_token.as_deref(), Some("fresh-refresh"));
}
