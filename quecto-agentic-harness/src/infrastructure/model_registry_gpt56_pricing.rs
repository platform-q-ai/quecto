// Built-in GPT-5.6 tier pricing, split out of `model_registry.rs` to respect
// the per-file line cap. Returns the infra `ModelCost`, so it lives in the
// infrastructure layer next to the registry rather than in the domain.

use super::ModelCost;

/// USD-per-1M-token cost for the built-in GPT-5.6 tiers, or `None` otherwise.
/// Cache read 0.10x input (90% discount), cache write 1.25x input (OpenAI
/// GPT-5.6+ caching). Published 2026-07-09; see `builtin` for source URLs.
pub(super) fn gpt_5_6_cost(id: &str) -> Option<ModelCost> {
    let (input, output) = match id {
        "gpt-5.6-sol" => (5.0, 30.0),
        "gpt-5.6-terra" => (2.5, 15.0),
        "gpt-5.6-luna" => (1.0, 6.0),
        _ => return None,
    };
    Some(ModelCost {
        input,
        output,
        cache_read: input * 0.10,
        cache_write: input * 1.25,
    })
}
