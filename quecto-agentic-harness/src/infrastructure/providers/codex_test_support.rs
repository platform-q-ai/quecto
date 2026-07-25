//! Test-only accessors for `CodexProvider`'s private request builders.
//!
//! Kept out of `codex.rs` so production code stays within the repo's
//! file-size budget; these are compiled only under `test` or the
//! `test-support` feature.

use super::*;

impl CodexProvider {
    /// Public accessor for `build_request_body` on the ChatGPT Codex (OAuth)
    /// backend (for BDD/integration tests).
    pub fn build_request_body_public_oauth(request: &ChatRequest<'_>) -> serde_json::Value {
        Self::build_request_body(
            request,
            &ResponsesAuth::ChatGptOAuth {
                account_id: "acct-test".to_string(),
            },
        )
    }

    /// Public accessor for `build_request_body` on the standard OpenAI
    /// Responses API (API-key) backend (for BDD/integration tests).
    pub fn build_request_body_public_api_key(request: &ChatRequest<'_>) -> serde_json::Value {
        Self::build_request_body(request, &ResponsesAuth::ApiKey)
    }

    /// Public accessor for `build_input` (for BDD/integration tests).
    pub fn build_input_public(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_input(messages)
    }
}
