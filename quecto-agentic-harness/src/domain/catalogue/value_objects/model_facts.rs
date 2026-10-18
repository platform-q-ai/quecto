//! Model facts: cost, capabilities, and the context window rule (#2405).

/// Per-token cost figures, in USD per million tokens.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Capabilities and limits of a model, replacing the heuristics currently
/// scattered across consumers.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCapabilities {
    /// Input modalities (e.g. `text`, `image`).
    pub input_modalities: Vec<String>,
    /// Context window limit in tokens.
    pub context_window: u32,
    /// Output token limit.
    pub max_output_tokens: u32,
    /// Whether the limits were declared by the source (vs defaulted).
    pub context_window_explicit: bool,
    pub max_output_tokens_explicit: bool,
    /// Whether the model supports reasoning/effort controls.
    pub reasoning: bool,
    /// The reasoning-effort vocabulary this model accepts, in ascending
    /// order, as API string values (epic #1193, slice 6). Canonical
    /// capability metadata: every listing/selection surface projects this
    /// field instead of re-deriving a vocabulary of its own.
    pub effort_levels: Vec<String>,
    pub cost: ModelCost,
    /// How the provider bounds the prompt inside the window (#2405).
    pub prompt_limit: PromptLimit,
}

/// How a provider bounds the prompt inside a model's window (#2405).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptLimit {
    /// The provider checks the prompt plus the requested `max_tokens`
    /// against the window (Anthropic, generic OpenAI-compatible endpoints,
    /// local servers).
    #[default]
    SharedWithRequest,
    /// The provider fixes the input limit at the window less the declared
    /// output cap (OpenAI Responses and Codex: 400k = 272k + 128k).
    WindowLessOutputCap,
}

/// The percent of a fixed input limit kept free (#2405 review L5): the
/// estimate can run under the provider's count between calibrations, and a
/// prompt over a fixed input limit is refused outright. Codex keeps the
/// same margin (`effective_context_window_percent: 95` in openai/codex).
pub const FIXED_INPUT_HEADROOM_PERCENT: usize = 5;

/// Where the provider shares the window with the request, the prompt keeps
/// at least `1 / PROMPT_FLOOR_DIVISOR` of it whatever the reply reserves
/// (#2405 review M1).
const PROMPT_FLOOR_DIVISOR: usize = 2;

/// A model's declared context window and what its reply needs beside the
/// prompt (#2405): the one rule the context ceiling is computed from.
///
/// The prompt's room is the window less the reply's reserve, which depends
/// on how the provider bounds the prompt ([`PromptLimit`]). Under a fixed
/// input limit it is that limit less the headroom, never lifted past it.
/// Where the window is shared with the request it is never less than half
/// the window: a reserve past that is clamped, so the room never collapses
/// when a declared cap nearly fills the window. Either way the room never
/// falls as the window grows, and a reserve leaving the prompt under half
/// the window is reported ([`Self::reserve_clamped`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelWindow {
    /// The declared window in tokens, `None` when the model declares none.
    pub window: Option<usize>,
    /// How the provider bounds the prompt inside the window.
    pub prompt_limit: PromptLimit,
    /// The model's declared output cap, `None` when it declares none.
    pub output_cap: Option<usize>,
    /// The output limit a request asks for (`max_tokens` clamped to the cap).
    pub requested_output: usize,
}

impl ModelWindow {
    pub fn new(
        window: Option<usize>,
        prompt_limit: PromptLimit,
        output_cap: Option<usize>,
        requested_output: usize,
    ) -> Self {
        Self {
            window,
            prompt_limit,
            output_cap,
            requested_output,
        }
    }

    /// The room the window leaves the prompt; `None` when unknown.
    pub fn prompt_room(self) -> Option<usize> {
        let window = self.window?;
        let unfloored = self.unfloored_room(window);
        if self.fixed_input() {
            // The provider's own limit: the floor never lifts it (#2405
            // final review L1).
            debug_assert!(
                unfloored <= window.saturating_sub(self.reply_reserve()),
                "a fixed input ceiling stays within the provider's limit"
            );
            return Some(unfloored);
        }
        let floor = window / PROMPT_FLOOR_DIVISOR;
        let room = unfloored.max(floor);
        debug_assert!(
            floor <= room && room <= window,
            "a shared prompt keeps half the window, and no more than the window"
        );
        Some(room)
    }

    /// Whether the reply's reserve leaves the prompt under half the window:
    /// clamped where the window is shared, kept as the provider sets it
    /// under a fixed input limit; either way the declaration deserves a
    /// warning.
    pub fn reserve_clamped(self) -> bool {
        self.window
            .is_some_and(|window| self.unfloored_room(window) < window / PROMPT_FLOOR_DIVISOR)
    }

    /// Whether the provider fixes the input limit at the window less a
    /// declared output cap; without a declared cap that limit is unknown,
    /// and the reply reserves what a request asks for, as elsewhere.
    fn fixed_input(self) -> bool {
        self.prompt_limit == PromptLimit::WindowLessOutputCap && self.output_cap.is_some()
    }

    /// What the reply needs beside the prompt: the declared cap under a
    /// fixed input limit; elsewhere what a request can ask for, the
    /// effective limit or, after an output-limit cut-off, up to twice it
    /// within the cap (#2124).
    fn reply_reserve(self) -> usize {
        let requested = self.requested_output;
        match self.output_cap {
            Some(cap) if self.fixed_input() => cap,
            Some(cap) => cap.min(requested.saturating_mul(2)).max(requested),
            None => requested,
        }
    }

    /// The window less the reply's reserve (and, under a fixed input limit,
    /// its headroom), before the floor.
    fn unfloored_room(self, window: usize) -> usize {
        let room = window.saturating_sub(self.reply_reserve());
        if self.fixed_input() {
            // Exact in u128, and never more than `room`.
            let headroom = room as u128 * FIXED_INPUT_HEADROOM_PERCENT as u128 / 100;
            room - usize::try_from(headroom).unwrap_or(room)
        } else {
            room
        }
    }

    /// The context ceiling: the lower of the `configured` budget
    /// (`max_context_tokens`) and the prompt's room. An unknown window
    /// leaves the configured budget.
    pub fn ceiling(self, configured: usize) -> usize {
        self.prompt_room()
            .map_or(configured, |room| configured.min(room))
    }

    /// The configured budget clamped to the whole window, prompt and reply
    /// together (the room a raised output limit may use, #2124).
    pub fn budget(self, configured: usize) -> usize {
        self.window
            .map_or(configured, |window| configured.min(window))
    }
}

#[cfg(test)]
#[path = "model_facts_tests.rs"]
mod tests;
