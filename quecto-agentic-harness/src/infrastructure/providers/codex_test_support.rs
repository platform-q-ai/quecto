//! Test-only accessors for `CodexProvider`'s private request builders.
//!
//! Kept out of `codex.rs` so production code stays within the repo's
//! file-size budget; these are compiled only under `test` or the
//! `test-support` feature.

use super::*;

impl CodexProvider {
    /// Bound a silent streaming reply by `limit` rather than the default
    /// stream idle limit (tests, #2210).
    pub fn with_stream_idle_limit(mut self, limit: std::time::Duration) -> Self {
        let total = self.stream_idle.total();
        self.stream_idle =
            crate::infrastructure::providers::stream_idle::StreamIdle::new(limit).with_total(total);
        self
    }

    /// Bound a whole non-streaming reply by `total` rather than the default
    /// reply total limit (tests, #2210 review).
    pub fn with_reply_total_limit(mut self, total: std::time::Duration) -> Self {
        self.stream_idle = self.stream_idle.with_total(total);
        self
    }

    /// Public accessor for `build_request_body` on the ChatGPT Codex (OAuth)
    /// backend (for BDD/integration tests).
    pub fn build_request_body_public_oauth(request: &ChatRequest<'_>) -> serde_json::Value {
        Self::build_request_body(
            request,
            &ResponsesAuth::ChatGptOAuth {
                account_id: "acct-test".to_string(),
            },
            "",
        )
    }

    /// Public accessor for `build_request_body` on the standard OpenAI
    /// Responses API (API-key) backend (for BDD/integration tests).
    pub fn build_request_body_public_api_key(request: &ChatRequest<'_>) -> serde_json::Value {
        Self::build_request_body(request, &ResponsesAuth::ApiKey, "")
    }

    /// Public accessor for `build_input` (for BDD/integration tests).
    pub fn build_input_public(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_input(messages)
    }
}
