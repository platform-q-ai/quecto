use super::*;

#[derive(Debug)]
struct PauseAfterUnauthorized(Arc<AtomicU32>);
impl crate::domain::provider::RequestAdmission for PauseAfterUnauthorized {
    fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            if self.0.load(Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                Err(DomainError::Tool("swarm paused".into()))
            }
        })
    }
}
#[tokio::test]
async fn pause_after_unauthorized_response_prevents_refreshed_inference() {
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
    let factory = make_mock_factory(call_count.clone(), 1);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store.clone(),
        provider_name: "anthropic".to_string(),
        credential_provider: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("sk-ant-oat01-fresh"),
        factory,
    });

    let mut request = test_request();
    request.admission = Some(Arc::new(PauseAfterUnauthorized(call_count.clone())));
    let result = refreshable.chat(request).await;
    assert!(result.is_err(), "paused OAuth retry must be rejected");
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshable_delegates_chat_stream_incremental_to_inner_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let provider = RefreshableProvider::new(RefreshableConfig {
        inner: Arc::new(MockStreamingProvider),
        store: Arc::new(CredentialStore::new(tmp.path())),
        provider_name: "anthropic".to_string(),
        credential_provider: "anthropic".to_string(),
        refresh_fn: make_mock_refresh("unused"),
        factory: noop_factory(),
    });

    let mut rx = provider.chat_stream_incremental(test_request()).await;
    assert!(matches!(
        rx.recv().await,
        Some(crate::domain::provider::StreamEvent::TextDelta(text)) if text == "stream"
    ));
    assert!(matches!(
        rx.recv().await,
        Some(crate::domain::provider::StreamEvent::Done(done)) if done.content.as_deref() == Some("stream-done")
    ));
}

/// A pause that denies the resend must not discard the refreshed provider:
/// the next call after resume uses the fresh credential without another
/// 401 round trip.
#[tokio::test]
async fn denied_resend_keeps_the_refreshed_provider() {
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
    let refreshes = Arc::new(AtomicU32::new(0));
    // The stale inner keeps failing; only a factory-built provider succeeds.
    let inner = Arc::new(MockRetryProvider::new(call_count.clone(), u32::MAX));
    let factory = make_mock_factory(call_count.clone(), 0);
    let counted = refreshes.clone();
    let base = make_mock_refresh("sk-ant-oat01-fresh");
    let refresh_fn: RefreshFn = Arc::new(move |store, provider| {
        counted.fetch_add(1, Ordering::SeqCst);
        base(store, provider)
    });
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store,
        provider_name: "anthropic".to_string(),
        credential_provider: "anthropic".to_string(),
        refresh_fn,
        factory,
    });
    let mut request = test_request();
    request.admission = Some(Arc::new(PauseAfterUnauthorized(call_count.clone())));
    assert!(
        refreshable.chat(request).await.is_err(),
        "paused resend refused"
    );
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    let resumed = refreshable.chat(test_request()).await;
    assert!(resumed.is_ok(), "{resumed:?}");
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        1,
        "the refreshed provider was retained; no second refresh"
    );
}
