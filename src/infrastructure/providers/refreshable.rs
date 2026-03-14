//! RefreshableProvider — decorator that retries on 401 after refreshing OAuth tokens.
//!
//! Wraps an inner `LlmProvider` and intercepts auth errors (401). When a 401 is
//! detected and the provider has an OAuth credential with a refresh token in the
//! credential store, it refreshes the token, rebuilds the inner provider with the
//! new token, and retries the request once.
//!
//! On the happy path (no auth error), the borrowed `ChatRequest` is forwarded
//! directly to the inner provider via a shallow clone (slice pointers and small
//! `Option` fields only — no deep clone of messages or tools).  The request data
//! is only deep-cloned into an `OwnedRequest` on the rare retry path (401 +
//! refreshable credential).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::infrastructure::auth::credential_store::{AuthMethod, CredentialStore};
use crate::infrastructure::providers::error::ErrorClass;

/// Async function that refreshes an OAuth token.
///
/// Takes the credential store and provider name, returns the new access token.
/// Responsible for persisting the refreshed credential in the store.
pub type RefreshFn = Arc<
    dyn Fn(
            Arc<CredentialStore>,
            &str,
        ) -> Pin<Box<dyn Future<Output = Result<String, DomainError>> + Send>>
        + Send
        + Sync,
>;

/// Factory function that rebuilds a provider with a new API key.
pub type ProviderFactory = Arc<dyn Fn(&str) -> Arc<dyn LlmProvider> + Send + Sync>;

/// Configuration for building a [`RefreshableProvider`].
pub struct RefreshableConfig {
    /// The initial inner provider.
    pub inner: Arc<dyn LlmProvider>,
    /// Credential store for checking/persisting tokens.
    pub store: Arc<CredentialStore>,
    /// Provider name (e.g. "anthropic", "openai").
    pub provider_name: String,
    /// Function to refresh the OAuth token.
    pub refresh_fn: RefreshFn,
    /// Function to rebuild the provider with a new API key.
    pub factory: ProviderFactory,
}

/// A provider decorator that intercepts 401 errors and attempts to refresh
/// the OAuth token before retrying the request with a rebuilt provider.
pub struct RefreshableProvider {
    inner: RwLock<Arc<dyn LlmProvider>>,
    store: Arc<CredentialStore>,
    provider_name: String,
    refresh_fn: RefreshFn,
    factory: ProviderFactory,
}

impl std::fmt::Debug for RefreshableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshableProvider")
            .field("provider_name", &self.provider_name)
            .finish()
    }
}

impl RefreshableProvider {
    /// Create a new refreshable provider from a config.
    pub fn new(config: RefreshableConfig) -> Self {
        Self {
            inner: RwLock::new(config.inner),
            store: config.store,
            provider_name: config.provider_name,
            refresh_fn: config.refresh_fn,
            factory: config.factory,
        }
    }

    /// Check if the error is a 401 that might be fixable by refreshing.
    fn is_refreshable_auth_error(err: &DomainError) -> bool {
        let class = crate::infrastructure::providers::error::classify_error(err);
        class == ErrorClass::Auth
    }

    /// Check if the provider has an OAuth credential with a refresh token.
    fn has_refreshable_credential(&self) -> bool {
        let creds = self.store.load_snapshot().unwrap_or_default();
        if let Some(cred) = creds.get(&self.provider_name) {
            cred.method == AuthMethod::OAuth && cred.refresh_token.is_some()
        } else {
            false
        }
    }
}

impl LlmProvider for RefreshableProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async move {
            self.try_with_refresh(request, |inner, req| inner.chat(req))
                .await
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async move {
            self.try_with_refresh(request, |inner, req| inner.chat_stream(req))
                .await
        })
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<
        Box<
            dyn Future<Output = tokio::sync::mpsc::Receiver<crate::domain::provider::StreamEvent>>
                + Send
                + 'a,
        >,
    > {
        // Delegate to the inner provider.  Token refresh is not attempted
        // here — auth errors come through the StreamEvent channel.
        Box::pin(async move {
            let inner = self.inner.read().await.clone();
            inner.chat_stream_incremental(request).await
        })
    }
}

/// Owned copies of borrowed `ChatRequest` fields, used only on the retry path.
///
/// When the first call returns a 401 and a token refresh succeeds, the rebuilt
/// provider needs its own copy of the request data because the new provider
/// `Arc` has a different lifetime than the original borrow.
struct OwnedRequest {
    messages: Vec<crate::domain::message::Message>,
    tools: Vec<crate::domain::tool::ToolDefinition>,
    model: String,
    max_tokens: u32,
    temperature: f32,
    session_id: Option<String>,
    tool_choice: Option<crate::domain::provider::ToolChoice>,
    metadata: Option<crate::domain::provider::RequestMetadata>,
    thinking_level: Option<crate::domain::provider::ThinkingLevel>,
    cancel_flag: Option<crate::domain::provider::CancelFlag>,
    effort: Option<crate::domain::provider::EffortLevel>,
}

impl OwnedRequest {
    fn from(r: &ChatRequest<'_>) -> Self {
        Self {
            messages: r.messages.to_vec(),
            tools: r.tools.to_vec(),
            model: r.model.to_string(),
            max_tokens: r.max_tokens,
            temperature: r.temperature,
            session_id: r.session_id.map(String::from),
            tool_choice: r.tool_choice.clone(),
            metadata: r.metadata.clone(),
            thinking_level: r.thinking_level,
            cancel_flag: r.cancel_flag.clone(),
            effort: r.effort,
        }
    }

    fn as_request(&self) -> ChatRequest<'_> {
        ChatRequest {
            messages: &self.messages,
            tools: &self.tools,
            model: &self.model,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            session_id: self.session_id.as_deref(),
            tool_choice: self.tool_choice.clone(),
            metadata: self.metadata.clone(),
            thinking_level: self.thinking_level,
            cancel_flag: self.cancel_flag.clone(),
            effort: self.effort,
        }
    }
}

impl RefreshableProvider {
    /// Try a provider call, refreshing the token on 401 and retrying once.
    ///
    /// On the happy path (no auth error), the borrowed `ChatRequest` is
    /// forwarded via a shallow clone (slice pointers + small `Option` fields
    /// only — no deep clone of messages or tools).  Only on the rare retry
    /// path (401 + OAuth credential) are the request fields deep-cloned
    /// into an `OwnedRequest` so the rebuilt provider can use them.
    async fn try_with_refresh<'a, F>(
        &'a self,
        request: ChatRequest<'a>,
        call: F,
    ) -> Result<LlmResponse, DomainError>
    where
        F: for<'b> Fn(
            &'b Arc<dyn LlmProvider>,
            ChatRequest<'b>,
        ) -> Pin<
            Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'b>,
        >,
    {
        let inner = self.inner.read().await.clone();
        // Happy path: shallow clone (copies slice pointers + small Option fields,
        // not the underlying message/tool vecs).
        let result = call(&inner, request.clone()).await;

        match result {
            Ok(resp) => Ok(resp),
            Err(err)
                if Self::is_refreshable_auth_error(&err) && self.has_refreshable_credential() =>
            {
                tracing::info!(
                    provider = self.provider_name.as_str(),
                    "401 from OAuth provider — attempting token refresh"
                );

                // Retry path: clone request data so the rebuilt provider
                // can use it independently of the original borrow.
                let owned = OwnedRequest::from(&request);

                match (self.refresh_fn)(self.store.clone(), &self.provider_name).await {
                    Ok(new_token) => {
                        tracing::info!(
                            provider = self.provider_name.as_str(),
                            "token refreshed — rebuilding provider"
                        );
                        let new_inner = (self.factory)(&new_token);
                        let result = call(&new_inner, owned.as_request()).await;
                        *self.inner.write().await = new_inner;
                        result
                    }
                    Err(refresh_err) => {
                        tracing::warn!(
                            provider = self.provider_name.as_str(),
                            error = %refresh_err,
                            "token refresh failed — returning original 401"
                        );
                        Err(err)
                    }
                }
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
#[path = "refreshable_tests.rs"]
mod tests;
