//! #935: per-model output-cap clamp for the agent loop.
//!
//! The configured `max_tokens` is a global default; a model whose real output
//! limit is lower (e.g. Fireworks qwen3p7-plus = 65536) must never receive a
//! larger value or the provider rejects every request. These mutators carry the
//! per-model registry cap and the request builder uses `effective_max_tokens`.

use super::AgentLoopImpl;

impl AgentLoopImpl {
    /// Switch the active model. Pair with [`Self::set_model_max_tokens`] so the
    /// per-model output cap is re-derived and a model switch re-clamps.
    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Set the per-model output cap used to clamp the effective request tokens
    /// to `min(max_tokens, cap)`. `None` clears (use `max_tokens` verbatim).
    pub fn set_model_max_tokens(&mut self, model_max_tokens: Option<u32>) {
        self.model_max_tokens = model_max_tokens;
    }

    /// Builder variant of [`Self::set_model_max_tokens`].
    pub fn with_model_max_tokens(mut self, model_max_tokens: Option<u32>) -> Self {
        self.model_max_tokens = model_max_tokens;
        self
    }

    /// The effective per-request output cap: configured `max_tokens` clamped
    /// down to the model's registry cap when one is known.
    pub fn effective_max_tokens(&self) -> u32 {
        match self.model_max_tokens {
            Some(cap) => self.max_tokens.min(cap),
            None => self.max_tokens,
        }
    }
}
