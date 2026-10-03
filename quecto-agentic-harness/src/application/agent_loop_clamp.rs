//! #935: per-model output-cap clamp for the agent loop, and what else the
//! active model declares: how its prompt is bounded (#2405) and whether it
//! takes images (#2421).
//!
//! The configured `max_tokens` is a global default; a model whose real output
//! limit is lower (e.g. Fireworks qwen3p7-plus = 65536) must never receive a
//! larger value or the provider rejects every request. These mutators carry the
//! per-model registry cap and the request builder uses `effective_max_tokens`;
//! the loop's requests go through `request_max_tokens`, which may raise one
//! request after an output-limit cut-off (#2124).

use super::AgentLoopImpl;
use crate::application::catalogue::dto::ModelLimits;
use crate::application::catalogue::ports::ModelRuntime;
use crate::domain::catalogue::ModelWindow;
use crate::domain::conversation::image_input::{ImageInput, SentConversation};

/// Tokens kept free of the context window when a retry raises the output limit.
const OUTPUT_ROOM_MARGIN: usize = 1024;

impl ModelRuntime for AgentLoopImpl {
    fn model(&self) -> &str {
        &self.model
    }

    fn apply_model(&mut self, model: String, limits: ModelLimits) {
        self.switch_model(model, limits);
    }

    fn route_check(&self, model: &str) -> crate::application::providers::ports::RouteCheck {
        self.provider.route_check(model)
    }
}

impl AgentLoopImpl {
    /// Switch the active model and its limits together so they can never
    /// diverge: the output cap clamps each request to `min(max_tokens, cap)`
    /// (#935), and the window and its prompt limit bound the context
    /// ceiling (#1044, #2405). The change-active-model use case is the only
    /// caller (#1847), through the [`ModelRuntime`] port.
    fn switch_model(&mut self, model: String, limits: ModelLimits) {
        self.model = model;
        self.set_model_limits(limits);
        // #2212: another model may tokenise differently.
        self.context_manager.forget_calibration();
    }

    fn set_model_limits(&mut self, limits: ModelLimits) {
        self.model_max_tokens = limits.max_output_tokens;
        self.model_context_window = limits.context_window;
        self.model_traits = ModelTraits {
            prompt_limit: limits.prompt_limit,
            image_input: limits.image_input,
        };
        self.sync_context_limits();
    }

    /// The conversation as the active model is sent it (#2421), while the
    /// result lives: each image the model does not take is a marker in its
    /// message. Dropping it gives the conversation every image back.
    pub(super) fn conversation_for_model<'m>(
        &self,
        messages: &'m mut [crate::domain::message::Message],
    ) -> SentConversation<'m> {
        SentConversation::new(messages, &self.model, self.model_traits.image_input)
    }

    /// Builder variant: the startup model's limits (#2405), which the
    /// composition reads from the published catalogue.
    pub fn with_model_limits(mut self, limits: ModelLimits) -> Self {
        self.set_model_limits(limits);
        self
    }

    /// Builder variant: set the per-model output cap at construction time.
    pub fn with_model_max_tokens(mut self, model_max_tokens: Option<u32>) -> Self {
        self.model_max_tokens = model_max_tokens;
        self.sync_context_limits();
        self
    }

    /// Hand the context manager the model's window, how its provider bounds
    /// the prompt, its declared output cap and what a request asks for
    /// (#2405), from which it computes the ceiling. A window the ceiling
    /// cannot use as declared (none, or a cap that leaves the prompt under
    /// half the window) is noted at the next request, once per model and
    /// kind of note.
    pub(super) fn sync_context_limits(&mut self) {
        let tokens = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
        let window = ModelWindow::new(
            self.model_context_window,
            self.model_traits.prompt_limit,
            self.model_max_tokens.map(tokens),
            tokens(self.effective_max_tokens()),
        );
        self.context_manager.set_model_window(window);
        let note = match window.window {
            None => Some(WindowNote::NoWindow),
            Some(_) if window.reserve_clamped() => Some(WindowNote::ReserveClamped),
            Some(_) => None,
        };
        let notes = self
            .context_notes
            .get_mut()
            .unwrap_or_else(|e| e.into_inner());
        notes.pending = note.filter(|note| !notes.noted.contains(&(self.model.clone(), *note)));
    }

    /// Log the active model's pending note (#2405), once per model, with
    /// the ceiling in force at the request: after a swarm member's cap.
    pub(super) fn log_pending_window_note(&self) {
        let mut notes = self.context_notes.lock().unwrap_or_else(|e| e.into_inner());
        let Some(note) = notes.pending.take() else {
            return;
        };
        if !notes.noted.insert((self.model.clone(), note)) {
            return;
        }
        let max_context_tokens = self.context_manager.effective_max_context_tokens();
        match note {
            WindowNote::NoWindow => tracing::info!(
                target: "context_ceiling",
                model = %self.model,
                max_context_tokens,
                "the model declares no context window; the context ceiling is the configured \
                 max_context_tokens"
            ),
            WindowNote::ReserveClamped => tracing::warn!(
                target: "context_ceiling",
                model = %self.model,
                context_window = self.model_context_window,
                max_output_tokens = self.model_max_tokens,
                max_context_tokens,
                "the model's declared output cap leaves the prompt under half its context window; \
                 a shared window's reserve is clamped so the prompt keeps half, a fixed input \
                 limit is kept as the provider sets it"
            ),
        }
    }

    /// The output limit for the next request (#2124). After an output-limit
    /// cut-off one request may go above the effective limit, up to the
    /// model's declared cap, twice the effective limit (the configured
    /// `max_tokens` is also a cost ceiling), and what the context window has
    /// left beside the prompt (the window budget: a swarm member's pruning
    /// cap bounds what it keeps, not the model's room, #2349 review M2);
    /// never below the effective limit. The value is
    /// remembered so the reply is judged against what the request asked for.
    pub(super) fn request_max_tokens(&self, estimated_context_tokens: usize) -> u32 {
        use std::sync::atomic::Ordering::Relaxed;
        let boosted = self.output_boost.swap(false, Relaxed);
        // #2212: the room beside the prompt at its provider-calibrated size.
        let scale = self.context_manager.estimate_scale();
        let context_tokens = scale.calibrated(estimated_context_tokens);
        let effective = self.effective_max_tokens();
        let limit = match (boosted, self.model_max_tokens) {
            (true, Some(cap)) if cap > effective => {
                let room = self
                    .context_manager
                    .window_budget_tokens()
                    .saturating_sub(context_tokens)
                    .saturating_sub(OUTPUT_ROOM_MARGIN);
                let room = u32::try_from(room).unwrap_or(u32::MAX);
                cap.min(effective.saturating_mul(2))
                    .min(room)
                    .max(effective)
            }
            _ => effective,
        };
        self.last_request_max_tokens.store(limit, Relaxed);
        limit
    }

    /// The output limit the last request was sent with (the effective limit
    /// before any request).
    pub(super) fn last_request_max_tokens(&self) -> u32 {
        match self
            .last_request_max_tokens
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => self.effective_max_tokens(),
            sent => sent,
        }
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

    /// Test builder: the model's known context window (#1044). Production
    /// threads this through `AgentLoopConfig::model_context_window` at
    /// construction; `apply_model` re-derives it on a model switch.
    #[cfg(test)]
    pub fn with_model_context_window(mut self, window: Option<usize>) -> Self {
        self.model_context_window = window;
        self.sync_context_limits();
        self
    }

    /// The effective context-token budget (#1044, #2405): what the active
    /// model's known context window leaves beside the reply's reserve, when
    /// that is smaller than the configured `max_context_tokens`; the config
    /// value is the override/fallback (unknown windows leave the configured
    /// budget untouched).
    ///
    /// The footer numerator is provider-reported prompt occupancy when usage is
    /// available, so it includes provider-side overhead such as tool schemas.
    /// This denominator intentionally remains the enforced hot-context budget;
    /// when no smaller model window is known, the percentage is a pruning-budget
    /// proximity indicator rather than a full-window percentage.
    pub fn effective_max_context_tokens(&self) -> usize {
        self.context_manager.effective_max_context_tokens()
    }

    /// The handle through which the composition lowers this loop's pruning
    /// ceiling after construction (#2342): a swarm member's, once its
    /// process joins a swarm. The effective budget is the lowest of the
    /// configured budget, the model's window and this cap.
    pub fn context_ceiling_cap(&self) -> crate::application::context::ContextCeilingCap {
        self.context_manager.ceiling_cap()
    }
}

/// What the active model declares beside its token limits: how its
/// provider bounds the prompt (#2405) and whether it takes images (#2421).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ModelTraits {
    pub(super) prompt_limit: crate::domain::catalogue::PromptLimit,
    pub(super) image_input: ImageInput,
}

/// Why the active model's window is worth a line in the log (#2405).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum WindowNote {
    /// The model declares no window: the configured budget stands.
    NoWindow,
    /// The declared cap would leave the prompt under its floor.
    ReserveClamped,
}

/// The note awaiting the next request, and the (model, note) pairs already
/// logged: each kind of note once per model.
#[derive(Debug, Default)]
pub(super) struct ContextNotes {
    pending: Option<WindowNote>,
    noted: std::collections::HashSet<(String, WindowNote)>,
}
