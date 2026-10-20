//! The published standard rates of the models the registry knows (#1855,
//! #2441, #2328): the one price table, the catalogue's model cost. Usage is
//! costed from the catalogue entry of the active model and from nothing
//! else. Rates are USD per million tokens; cache writes are the 5-minute
//! TTL. Long-context, batch, flex, fast and regional multipliers are not
//! modelled: every rate is the base standard one.
//!
//! Sources, read 2026-10-19:
//! - Anthropic: platform.claude.com/docs/en/about-claude/pricing. Sonnet 5's
//!   $2/$10 launch price is its standard price (footnote 3); Opus 5.5 and
//!   Sonnet 5.5 cache hits are 0.05x input, Fable 5.1's 0.025x.
//! - OpenAI: developers.openai.com/api/docs/models/{gpt-5.6-sol,
//!   gpt-5.6-terra, gpt-5.6-luna, gpt-6-astra, gpt-6.1-sol}; cache reads
//!   0.10x input (GPT-6.1 Sol 0.05x), cache writes 1.25x input.
//! - xAI: docs.x.ai/developers/{grok-4-7, models/grok-4.6, grok-4-5}.

use super::ModelCost;
use crate::domain::conversation::value_objects::message::starts_with_ci;

/// `(input, output, cache read, cache write)` USD per million tokens, by
/// case-insensitive id prefix (dated variants included). An allowlist,
/// ordered so a prefix another extends comes after it.
const KNOWN_RATES: &[(&str, [f64; 4])] = &[
    ("claude-fable-5-1", [10.0, 50.0, 0.25, 12.5]),
    ("claude-fable-5", [10.0, 50.0, 1.0, 12.5]),
    ("claude-opus-5-5", [4.0, 20.0, 0.2, 5.0]),
    ("claude-opus-5", [5.0, 25.0, 0.5, 6.25]),
    // Opus 4.1 and the original Opus 4 kept the earlier $15/$75 tier.
    ("claude-opus-4-1", [15.0, 75.0, 1.5, 18.75]),
    ("claude-opus-4-2025", [15.0, 75.0, 1.5, 18.75]),
    ("claude-opus-4", [5.0, 25.0, 0.5, 6.25]),
    ("claude-sonnet-5-5", [2.0, 10.0, 0.1, 2.5]),
    ("claude-sonnet-5", [2.0, 10.0, 0.2, 2.5]),
    ("claude-sonnet-4", [3.0, 15.0, 0.3, 3.75]),
    ("claude-haiku-4", [1.0, 5.0, 0.1, 1.25]),
    ("gpt-6-astra", [10.0, 50.0, 1.0, 12.5]),
    ("gpt-6-sol", [2.0, 10.0, 0.2, 2.5]),
    ("gpt-6.1-sol", [2.0, 10.0, 0.1, 2.5]),
    ("gpt-6-luna", [0.1, 0.5, 0.01, 0.125]),
    ("gpt-5.6-sol", [5.0, 30.0, 0.5, 6.25]),
    ("gpt-5.6-terra", [2.5, 15.0, 0.25, 3.125]),
    ("gpt-5.6-luna", [1.0, 6.0, 0.1, 1.25]),
    ("grok-4.7", [2.0, 6.0, 0.0, 0.0]),
    ("grok-4.6", [2.0, 6.0, 0.5, 0.0]),
    ("grok-4.5", [2.0, 6.0, 0.0, 0.0]),
];

/// The published cost of model `id`, or `None` for a model the table does
/// not know (its `models.json` declaration may still give one).
pub(super) fn known_cost(id: &str) -> Option<ModelCost> {
    KNOWN_RATES
        .iter()
        .find(|(prefix, _)| starts_with_ci(id, prefix))
        .map(|(_, [input, output, cache_read, cache_write])| ModelCost {
            input: *input,
            output: *output,
            cache_read: *cache_read,
            cache_write: *cache_write,
        })
}
