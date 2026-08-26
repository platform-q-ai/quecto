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
use tokio::sync::{Mutex, RwLock};

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

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
    /// The OAuth token `inner` was built from. Supplied by the composition that
    /// built it, so the pair is consistent by construction: reading it back from
    /// the store here could observe a token another process rotated in between,
    /// and a later 401 would then refresh unnecessarily.
    pub initial_token: Option<String>,
    /// Credential store for checking/persisting tokens.
    pub store: Arc<CredentialStore>,
    /// Router-facing provider name (e.g. "anthropic", "anthropic-oauth").
    /// This is the routing prefix returned by `name()`.
    pub provider_name: String,
    /// Credential-store identity used to look up and refresh the OAuth token
    /// (e.g. "anthropic"). For built-in slots this equals `provider_name`; for
    /// registry providers it is the referenced kernel OAuth provider, which may
    /// differ from the router prefix.
    pub credential_provider: String,
    /// Function to refresh the OAuth token.
    pub refresh_fn: RefreshFn,
    /// Function to rebuild the provider with a new API key.
    pub factory: ProviderFactory,
}

/// A provider decorator that intercepts 401 errors and attempts to refresh
/// the OAuth token before retrying the request with a rebuilt provider.
pub struct RefreshableProvider {
    /// The active provider together with the OAuth token it was built from.
    /// They move as one so a 401 cannot compare a new provider against an old
    /// token (or the reverse) and take the wrong recovery branch.
    inner: RwLock<(Arc<dyn LlmProvider>, Option<String>)>,
    store: Arc<CredentialStore>,
    provider_name: String,
    credential_provider: String,
    refresh_fn: RefreshFn,
    factory: ProviderFactory,
    refresh_lock: Mutex<()>,
}

impl std::fmt::Debug for RefreshableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshableProvider")
            .field("provider_name", &self.provider_name)
            .field("credential_provider", &self.credential_provider)
            .finish()
    }
}

impl RefreshableProvider {
    /// Create a new refreshable provider from a config.
    pub fn new(config: RefreshableConfig) -> Self {
        let inner_token = config.initial_token;
        Self {
            inner: RwLock::new((config.inner, inner_token)),
            store: config.store,
            provider_name: config.provider_name,
            credential_provider: config.credential_provider,
            refresh_fn: config.refresh_fn,
            factory: config.factory,
            refresh_lock: Mutex::new(()),
        }
    }

    /// Install a rebuilt inner provider together with the token it was built
    /// from, so both move as one state change.
    async fn install_inner(&self, provider: Arc<dyn LlmProvider>, token: String) {
        *self.inner.write().await = (provider, Some(token));
    }

    /// Check if the error is a 401 that might be fixable by refreshing.
    fn is_refreshable_auth_error(err: &DomainError) -> bool {
        classify_provider_error(err) == ProviderErrorClass::Auth
    }

    /// Check if the provider has an OAuth credential with a refresh token.
    fn has_refreshable_credential(&self) -> bool {
        let creds = self.store.load_snapshot().unwrap_or_default();
        if let Some(cred) = creds.get(&self.credential_provider) {
            cred.method == AuthMethod::OAuth && cred.refresh_token.is_some()
        } else {
            false
        }
    }

    /// Check if the provider has a refreshable OAuth credential that is
    /// expired (or within the persisted expiry margin).
    fn credential_needs_refresh(&self) -> bool {
        let creds = self.store.load_snapshot().unwrap_or_default();
        if let Some(cred) = creds.get(&self.credential_provider) {
            cred.method == AuthMethod::OAuth && cred.refresh_token.is_some() && cred.is_expired()
        } else {
            false
        }
    }

    fn current_credential(&self) -> Option<Credential> {
        self.store.load_snapshot().ok().and_then(|creds| {
            creds
                .get(&self.credential_provider)
                .filter(|cred| cred.method == AuthMethod::OAuth)
                .cloned()
        })
    }

    /// Pre-emptively refresh the token and rebuild the inner provider when the
    /// stored credential is expired.
    ///
    /// Used by the streaming path, which cannot retry mid-stream: a 401 surfaces
    /// as a `StreamEvent` error after the stream is already open, so there is no
    /// reactive retry like [`try_with_refresh`] performs. Instead we refresh
    /// ahead of time when the credential is known to be expired.
    ///
    /// Best-effort: if the refresh fails, the inner provider is left unchanged
    /// and the stream proceeds with the existing token (which will surface the
    /// underlying auth error to the caller, preserving prior behaviour).
    async fn refresh_if_expired(&self) {
        if !self.credential_needs_refresh() {
            return;
        }

        tracing::info!(
            provider = self.provider_name.as_str(),
            "stored OAuth token expired before stream — attempting pre-emptive refresh"
        );

        let _guard = self.refresh_lock.lock().await;
        if !self.credential_needs_refresh() {
            // Someone else refreshed while this task waited for the lock. The
            // stored credential is current, but this provider may still hold the
            // superseded token, and a stream cannot retry once it is open.
            if let Some(stored) = self.current_credential().map(|cred| cred.token)
                && self.inner.read().await.1.as_ref() != Some(&stored)
            {
                let new_inner = (self.factory)(&stored);
                self.install_inner(new_inner, stored).await;
            }
            return;
        }

        match (self.refresh_fn)(self.store.clone(), &self.credential_provider).await {
            Ok(new_token) => {
                tracing::info!(
                    provider = self.provider_name.as_str(),
                    "token refreshed — rebuilding provider for stream"
                );
                let new_inner = (self.factory)(&new_token);
                self.install_inner(new_inner, new_token).await;
            }
            Err(refresh_err) => {
                tracing::warn!(
                    provider = self.provider_name.as_str(),
                    error = %refresh_err,
                    "pre-emptive token refresh failed — proceeding with existing token"
                );
            }
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

    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        // The streaming path cannot retry mid-stream — a 401 surfaces as a
        // StreamEvent error after the stream is already open. So instead of the
        // reactive retry used by the non-streaming paths, refresh pre-emptively
        // when the stored token is already expired before opening the stream.
        Box::pin(async move {
            self.refresh_if_expired().await;
            let inner = self.inner.read().await.0.clone();
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
        // Provider and token are read together: on a 401 the token compared
        // against the store is exactly the one this attempt used, which
        // distinguishes "my token is stale" from "another task already
        // refreshed" without reading the credential store on the happy path.
        let (inner, attempted_token) = self.inner.read().await.clone();
        // Happy path: shallow clone (copies slice pointers + small Option fields,
        // not the underlying message/tool vecs). No credential I/O happens here;
        // the store is only read once a 401 proves the token needs attention.
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

                // The lock serialises obtaining a fresh token and installing the
                // provider built from it — both, so a task that adopted an older
                // token cannot install over a newer one. It is released before
                // the retried request, which would otherwise block every other
                // 401 for the length of a full generation.
                let refreshed = {
                    let _guard = self.refresh_lock.lock().await;
                    // An expired stored token is not an adoption candidate:
                    // taking it would spend this turn on a second 401 before
                    // refreshing anyway.
                    let stored = self
                        .current_credential()
                        .filter(|cred| !cred.is_expired())
                        .map(|cred| cred.token);
                    let token = match stored {
                        // Another task refreshed while this request was in
                        // flight: adopt its token instead of refreshing again.
                        // Compared against the token this attempt used, not the
                        // provider's current one, which a concurrent refresh may
                        // already have advanced.
                        Some(token) if Some(&token) != attempted_token.as_ref() => Ok(token),
                        _ => (self.refresh_fn)(self.store.clone(), &self.credential_provider)
                            .await
                            .inspect(|_| {
                                tracing::info!(
                                    provider = self.provider_name.as_str(),
                                    "token refreshed — rebuilding provider"
                                );
                            }),
                    };
                    match token {
                        Ok(new_token) => {
                            let new_inner = (self.factory)(&new_token);
                            self.install_inner(new_inner.clone(), new_token).await;
                            Ok(new_inner)
                        }
                        Err(refresh_err) => Err(refresh_err),
                    }
                };

                match refreshed {
                    Ok(new_inner) => call(&new_inner, owned.as_request()).await,
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

#[cfg(test)]
#[path = "refreshable_concurrency_tests.rs"]
mod concurrency_tests;
