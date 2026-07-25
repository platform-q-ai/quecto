//! #935: per-model output-cap clamp for the agent loop.
//!
//! The configured `max_tokens` is a global default; a model whose real output
//! limit is lower (e.g. Fireworks qwen3p7-plus = 65536) must never receive a
//! larger value or the provider rejects every request. These mutators carry the
//! per-model registry cap and the request builder uses `effective_max_tokens`.

use super::AgentLoopImpl;

impl AgentLoopImpl {
    /// Switch the active model, its per-model output cap, and its known
    /// context window together so they can never diverge. `model_max_tokens`
    /// is the new model's registry cap (or `None` for no clamp); the effective
    /// request tokens become `min(max_tokens, cap)` (#935).
    /// `model_context_window` is the new model's known context window (or
    /// `None` when unknown); the effective pruning budget becomes
    /// `min(max_context_tokens, window)` (#1044). Taking both as parameters
    /// (rather than separate setters) ensures every switch re-clamps with no
    /// fragile multi-call protocol.
    pub fn set_model(
        &mut self,
        model: String,
        model_max_tokens: Option<u32>,
        model_context_window: Option<usize>,
    ) {
        self.model = model;
        self.model_max_tokens = model_max_tokens;
        self.model_context_window = model_context_window;
        self.context_manager
            .set_model_context_window(model_context_window);
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

    /// Test builder: recent-turn tail-pin count (#1045). Production threads
    /// this through `AgentLoopConfig::pin_recent_turns` at construction.
    #[cfg(test)]
    pub fn with_pin_recent_turns(mut self, pin_recent_turns: u32) -> Self {
        self.context_manager.set_pin_recent_turns(pin_recent_turns);
        self
    }

    /// Test builder: conversation-message collapse threshold (#1046).
    /// Production threads this through
    /// `AgentLoopConfig::context_collapse_after_messages` at construction.
    #[cfg(test)]
    pub fn with_context_collapse_after_messages(mut self, max_messages: u32) -> Self {
        self.context_manager
            .set_context_collapse_after_messages(max_messages);
        self
    }

    /// Test builder: the model's known context window (#1044). Production
    /// threads this through `AgentLoopConfig::model_context_window` at
    /// construction; `set_model` re-derives it on a model switch.
    #[cfg(test)]
    pub fn with_model_context_window(mut self, window: Option<usize>) -> Self {
        self.model_context_window = window;
        self.context_manager.set_model_context_window(window);
        self
    }

    /// The effective context-token budget (#1044): the active model's known
    /// context window when it is smaller than the configured
    /// `max_context_tokens`; the config value is the override/fallback
    /// (unknown windows leave the configured budget untouched).
    ///
    /// The footer numerator is provider-reported prompt occupancy when usage is
    /// available, so it includes provider-side overhead such as tool schemas.
    /// This denominator intentionally remains the enforced hot-context budget;
    /// when no smaller model window is known, the percentage is a pruning-budget
    /// proximity indicator rather than a full-window percentage.
    pub fn effective_max_context_tokens(&self) -> usize {
        self.context_manager.effective_max_context_tokens()
    }
}
