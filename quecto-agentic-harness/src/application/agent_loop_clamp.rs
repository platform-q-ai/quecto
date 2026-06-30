//! #935: per-model output-cap clamp for the agent loop.
//!
//! The configured `max_tokens` is a global default; a model whose real output
//! limit is lower (e.g. Fireworks qwen3p7-plus = 65536) must never receive a
//! larger value or the provider rejects every request. These mutators carry the
//! per-model registry cap and the request builder uses `effective_max_tokens`.

use super::AgentLoopImpl;

impl AgentLoopImpl {
    /// Switch the active model and its per-model output cap together so they can
    /// never diverge. `model_max_tokens` is the new model's registry cap (or
    /// `None` for no clamp); the effective request tokens become
    /// `min(max_tokens, cap)`. Taking the cap as a parameter (rather than a
    /// separate setter) ensures every switch re-clamps with no fragile two-call
    /// protocol (#935).
    pub fn set_model(&mut self, model: String, model_max_tokens: Option<u32>) {
        self.model = model;
        self.model_max_tokens = model_max_tokens;
    }

    /// Builder variant: set the per-model output cap at construction time.
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
