//! Provider-calibrated token estimates (#2212).
//!
//! The local estimate (`domain::token_estimate`) prices prose, digit-bearing
//! runs, high-entropy runs and non-ASCII text at their own rates, so dense tool output is no
//! longer counted at half its size. What it still misses is a residual: a
//! tokeniser's own quirks, message framing, and the two gaps below. A
//! provider that reports its prompt size (`context_input_tokens`) says how
//! far off the estimate was for the transcript it was sent. [`EstimateScale`]
//! is that observed ratio, bounded, as an integer permille so the arithmetic
//! is exact and platform-independent.
//!
//! Known gaps in the estimate, which the ratio absorbs or cannot see:
//!
//! - Replayed reasoning is not estimated. Anthropic drops earlier turns'
//!   thinking from the context, so counting it would over-prune there; the
//!   Responses API replays encrypted reasoning (#2162), which the provider
//!   counts but whose ciphertext length says nothing about its tokens. The
//!   ratio carries it as residual.
//! - Images are estimated by their pixel size at the dearest current rate
//!   (#2420), and an unreadable one at the most an image costs. A provider
//!   that charges less reports below the estimate, so the ratio floors at
//!   1x and the estimate (erring towards pruning) decides.
//!
//! The clamp stays 1x..4x. The floor keeps an over-estimate (images, a
//! tokeniser packing prose tighter than 4 a token) from loosening the ceiling. The
//! cap is kept wide although the density classes bring dense output to
//! about 1x: replayed reasoning can be a large share of a prompt, and a
//! ratio that high on a transcript also means the estimate is badly wrong
//! somewhere, where trusting the provider is the safe side. Above 4x is a
//! provider quirk or fixed overhead on a tiny transcript.
//!
//! Pure: no clock, no I/O, no locking. The application layer decides when an
//! observation applies and when it has gone stale.

/// The observed ratio of provider-reported tokens to estimated tokens, in
/// permille, clamped to [`Self::MIN_PERMILLE`]..=[`Self::MAX_PERMILLE`].
///
/// The ratio scales every per-message estimate alike, so it is applied to
/// the budget instead: a transcript fits `budget` calibrated tokens exactly
/// when its estimate fits [`Self::in_estimate_units`]`(budget)`. The
/// pruning ladder then keeps summing plain estimates, consistently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstimateScale {
    permille: u32,
}

impl EstimateScale {
    /// The floor: never scale below the heuristic. A provider counting less
    /// than the estimate (prose, a cache quirk) keeps the heuristic, which
    /// then errs towards pruning early.
    pub const MIN_PERMILLE: u32 = 1_000;
    /// The cap: 4x (see the module docs for why it stays this wide).
    pub const MAX_PERMILLE: u32 = 4_000;
    /// No calibration: the heuristic as it is.
    pub const IDENTITY: Self = Self {
        permille: Self::MIN_PERMILLE,
    };

    /// The scale one provider observation implies: `reported` tokens for a
    /// transcript the heuristic put at `estimated`. Both must be positive for
    /// a ratio to exist; otherwise the heuristic stands.
    pub fn observed(reported: usize, estimated: usize) -> Self {
        let permille = match (reported, estimated) {
            (r, e) if r > 0 && e > 0 => {
                // Rounded up, so the calibrated figure never undercounts.
                let ratio = (r as u128 * 1_000).div_ceil(e as u128);
                let bounded = ratio.clamp(
                    u128::from(Self::MIN_PERMILLE),
                    u128::from(Self::MAX_PERMILLE),
                );
                u32::try_from(bounded).unwrap_or(Self::MAX_PERMILLE)
            }
            _ => Self::MIN_PERMILLE,
        };
        let scale = Self { permille };
        debug_assert!(scale.is_in_range(), "an observed scale is clamped");
        scale
    }

    /// The ratio in permille (1000 = the heuristic as it is).
    pub fn permille(self) -> u32 {
        self.permille
    }

    /// `estimate` in calibrated (provider) tokens, rounded up; saturates.
    pub fn calibrated(self, estimate: usize) -> usize {
        debug_assert!(self.is_in_range());
        let wide = (estimate as u128 * u128::from(self.permille)).div_ceil(1_000);
        usize::try_from(wide).unwrap_or(usize::MAX)
    }

    /// How many estimated tokens fit in `budget` calibrated tokens, rounded
    /// down (towards pruning). Never above `budget`: the scale is at least 1.
    pub fn in_estimate_units(self, budget: usize) -> usize {
        debug_assert!(self.is_in_range());
        let wide = budget as u128 * 1_000 / u128::from(self.permille);
        let units = usize::try_from(wide).unwrap_or(budget);
        debug_assert!(
            units <= budget,
            "a scale of at least 1 never grows a budget"
        );
        units
    }

    fn is_in_range(self) -> bool {
        (Self::MIN_PERMILLE..=Self::MAX_PERMILLE).contains(&self.permille)
    }
}

impl Default for EstimateScale {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The room the messages of one request have, in estimate units (#2160,
/// #2212 PR review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageBudget {
    /// What the pruning ladder may leave in the messages.
    pub tokens: usize,
    /// The tool definitions alone fill the model's window: no transcript
    /// fits, and the request is over budget whatever the ladder does.
    pub window_exceeded: bool,
}

/// The messages' budget beside `fixed` tokens every request carries (the
/// tool definitions), all in estimate units at the observed scale.
///
/// `ceiling` is the pruning ceiling (the configured budget, capped by the
/// window). The messages get what the tools leave of it, but at least a
/// quarter of it (#2182), so oversized tools cannot empty every turn. That
/// floor may pass the configured budget, never the model's `window`, a
/// hard provider limit: when known, the messages get at most what the tools
/// leave of the window. When the tools alone fill the window, no budget
/// fits; the floor stands (emptying the transcript would not help) and the
/// budget reports `window_exceeded`.
pub fn message_budget(ceiling: usize, window: Option<usize>, fixed: usize) -> MessageBudget {
    let floor = ceiling / 4;
    let soft = match ceiling.checked_sub(fixed) {
        Some(room) if room >= floor => room,
        Some(_) | None => floor,
    };
    let budget = match window.map(|window| window.checked_sub(fixed)) {
        None => MessageBudget {
            tokens: soft,
            window_exceeded: false,
        },
        Some(Some(room)) if room > 0 => MessageBudget {
            tokens: soft.min(room),
            window_exceeded: false,
        },
        Some(Some(_) | None) => MessageBudget {
            tokens: soft,
            window_exceeded: true,
        },
    };
    debug_assert!(
        budget.window_exceeded
            || window.is_none_or(|window| budget.tokens.saturating_add(fixed) <= window),
        "a budget that is not reported exceeded fits the window"
    );
    budget
}

#[cfg(test)]
#[path = "context_calibration_tests.rs"]
mod tests;
