//! The published standard rates of the models the harness knows (#1855,
//! #2441, #2328): the one price table. A catalogue entry's cost comes from
//! it (built in, declared by id alone, discovered), and so does the price of
//! a model the catalogue does not list, so usage is costed from the one
//! table whatever path the model took. Rates are USD per million tokens;
//! cache writes are the 5-minute TTL. Long-context, batch, flex, fast and
//! regional multipliers are not modelled: every rate is the base standard
//! one.
//!
//! Sources, read 2026-10-19 and 2026-10-20:
//! - Anthropic: platform.claude.com/docs/en/about-claude/pricing. Sonnet 5's
//!   $2/$10 launch price is its standard price (footnote 3); Opus 5.5 and
//!   Sonnet 5.5 cache hits are 0.05x input, Fable 5.1's 0.025x. Opus 4.5 to
//!   4.8 are $5/$25; the original Opus 4 and Opus 4.1 stay $15/$75.
//! - OpenAI: developers.openai.com/api/docs/models/{gpt-6-astra, gpt-6-sol,
//!   gpt-6.1-sol, gpt-6-luna, gpt-5.6-sol, gpt-5.6-terra, gpt-5.6-luna};
//!   cache reads 0.10x input (GPT-6.1 Sol 0.05x), cache writes 1.25x input.
//! - xAI: docs.x.ai/developers/models (rates below 200k prompt tokens). xAI
//!   lists no cache-write price: cache writes are billed as input.
use crate::domain::catalogue::value_objects::model_facts::ModelCost;
use crate::domain::conversation::value_objects::message::starts_with_ci;

/// `(input, output, cache read, cache write)` USD per million tokens, by
/// case-insensitive id prefix (dated and `@`-versioned ids included). An
/// allowlist, ordered so a more specific prefix comes before the shorter
/// one it extends.
const PUBLISHED_RATES: &[(&str, [f64; 4])] = &[
    ("claude-fable-5-1", [10.0, 50.0, 0.25, 12.5]),
    ("claude-fable-5", [10.0, 50.0, 1.0, 12.5]),
    ("claude-mythos-5-1", [10.0, 50.0, 0.25, 12.5]),
    ("claude-mythos-5", [10.0, 50.0, 1.0, 12.5]),
    // Haiku 5.5's rates for prompts up to 100,000 tokens; its higher tier
    // for longer prompts is not modelled (#2643).
    ("claude-haiku-5-5", [0.1, 0.5, 0.01, 0.125]),
    ("claude-opus-5-5", [4.0, 20.0, 0.2, 5.0]),
    ("claude-opus-5", [5.0, 25.0, 0.5, 6.25]),
    ("claude-opus-4-8", [5.0, 25.0, 0.5, 6.25]),
    ("claude-opus-4-7", [5.0, 25.0, 0.5, 6.25]),
    ("claude-opus-4-6", [5.0, 25.0, 0.5, 6.25]),
    ("claude-opus-4-5", [5.0, 25.0, 0.5, 6.25]),
    // The original Opus 4 (`claude-opus-4-0`, `claude-opus-4-20250514`,
    // `claude-opus-4@20250514`) and Opus 4.1 kept the earlier tier.
    ("claude-opus-4", [15.0, 75.0, 1.5, 18.75]),
    ("claude-sonnet-5-5", [2.0, 10.0, 0.1, 2.5]),
    ("claude-sonnet-5", [2.0, 10.0, 0.2, 2.5]),
    ("claude-sonnet-4", [3.0, 15.0, 0.3, 3.75]),
    ("claude-haiku-4", [1.0, 5.0, 0.1, 1.25]),
    ("gpt-6-astra", [10.0, 50.0, 1.0, 12.5]),
    ("gpt-6-sol", [2.0, 10.0, 0.2, 2.5]),
    ("gpt-6.1-sol", [2.0, 10.0, 0.1, 2.5]),
    ("gpt-6-luna", [0.1, 0.5, 0.01, 0.125]),
    // GPT-5.6 Sol's promotional price, "available at least through
    // November 21, 2026" (its model page); re-read it after that date
    // (#2642).
    ("gpt-5.6-sol", [4.0, 20.0, 0.4, 5.0]),
    ("gpt-5.6-terra", [2.0, 12.0, 0.2, 2.5]),
    ("gpt-5.6-luna", [0.2, 1.2, 0.02, 0.25]),
    ("grok-4.7", [2.0, 6.0, 0.5, 2.0]),
    ("grok-4.6", [2.0, 6.0, 0.5, 2.0]),
    ("grok-4.5", [2.0, 6.0, 0.3, 2.0]),
];

/// The published cost of model `id`, or `None` for a model the table does
/// not know.
pub fn published_cost(id: &str) -> Option<ModelCost> {
    PUBLISHED_RATES
        .iter()
        .find(|(prefix, _)| starts_with_ci(id, prefix))
        .map(|(_, [input, output, cache_read, cache_write])| ModelCost {
            input: *input,
            output: *output,
            cache_read: *cache_read,
            cache_write: *cache_write,
            declared: true,
        })
}

/// The cost a declaration states for model `id` (#1855 review M2), each
/// rate it leaves out filled in: from the published table for `id` when
/// the table knows it; else a cache rate is the input rate (a cache read
/// or write is never free unless declared so), and an input or output rate
/// is zero. A rate the declaration states always wins.
pub fn declared_cost(
    id: &str,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
) -> ModelCost {
    let published = published_cost(id);
    // A declaration that states no rate, for an id the table does not
    // know, states no price.
    let declared = [input, output, cache_read, cache_write]
        .iter()
        .any(Option::is_some)
        || published.is_some();
    let input = input
        .or(published.as_ref().map(|cost| cost.input))
        .unwrap_or(0.0);
    let output = output
        .or(published.as_ref().map(|cost| cost.output))
        .unwrap_or(0.0);
    let cache_read = cache_read
        .or(published.as_ref().map(|cost| cost.cache_read))
        .unwrap_or(input);
    let cache_write = cache_write
        .or(published.as_ref().map(|cost| cost.cache_write))
        .unwrap_or(input);
    ModelCost {
        input,
        output,
        cache_read,
        cache_write,
        declared,
    }
}

/// The cost of model `id` where nothing declares one (an entry the
/// catalogue synthesizes or discovers): the published table's, else none.
pub fn cost_of_id(id: &str) -> ModelCost {
    published_cost(id).unwrap_or_default()
}

#[cfg(test)]
#[path = "published_prices_tests.rs"]
mod tests;
