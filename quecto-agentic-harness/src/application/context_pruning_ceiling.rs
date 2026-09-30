// #1046 / #2213: the demotion-ladder context ceiling (full → stub → removed).
// Split from `context_pruning_messages.rs` (line cap). Pure policy over the
// message list: no retention handle, no store.

use super::{collapse_conversation_message, exempt_flags, message_collapse_stub};
use crate::application::context_pruning::exchanges::{
    drop_exchanges_until_under_budget, exchange_groups, keep_exchanges_whole,
};
use crate::application::context_pruning::{
    collapse_message, estimate_message_tokens, estimate_tokens, estimate_total_tokens,
};
use crate::domain::message::{Message, Role};
use crate::domain::turn_origin::report_to_keep;

/// The low-water mark of every pruning dial, in percent of the dial (#2213).
///
/// Crossing a dial (the token ceiling, the tool-result count, the
/// conversation-message count) prunes down to this share of it, not just
/// back under it. Each pruning pass rewrites a message early in the
/// conversation, which invalidates the provider's prompt-cache prefix from
/// there on. Pruning only the overflow let the next tool result cross again,
/// so every turn missed the cache (QA: 0 cached tokens on a 471k request,
/// about 7x the input cost). At 75 the quarter left as headroom takes about
/// five 12k-token tool results (or recalls) on the QA session's 250k budget
/// before the next pass, so the miss is paid once per batch, while three
/// quarters of the working context stay in full and few stubs need a recall
/// round-trip. A lower mark buys longer batches, a higher one more misses.
pub const LOW_WATER_PERCENT: usize = 75;

const _: () = assert!(LOW_WATER_PERCENT > 0 && LOW_WATER_PERCENT < 100); // 0 empties; 100 is per-turn

/// The low-water mark of a dial: [`LOW_WATER_PERCENT`] of `limit`, rounded
/// up so a small count dial keeps at least its share (a one-item dial keeps
/// its item). Never above `limit`; computed wide so no limit overflows.
/// Count dials of 1 to 3 so get no hysteresis: they collapse every crossing.
pub fn low_water(limit: usize) -> usize {
    let wide = (limit as u128 * LOW_WATER_PERCENT as u128).div_ceil(100);
    let mark = usize::try_from(wide).map_or(limit, |mark| mark.min(limit));
    debug_assert!(mark <= limit, "the low-water mark is under its dial");
    mark
}

/// Where the in-flight exchange starts (#2213): the model has not seen the
/// messages after its last assistant message yet (the results of that
/// message's tool calls), and a tool-calling last assistant message must
/// keep its results paired, so it is in flight too. With no assistant
/// message nothing is in flight. Neither count dial nor ladder rung demotes
/// an in-flight message: a stubbed unseen result would only be recalled.
pub(super) fn in_flight_start(messages: &[Message]) -> usize {
    match messages.iter().rposition(|m| m.role == Role::Assistant) {
        Some(last) if messages[last].tool_calls.is_empty() => last + 1,
        Some(last) => last,
        None => messages.len(),
    }
}

// #2213 / #2342: the count dials' batch (split out for the line ceiling).
#[path = "context_pruning_count_dial.rs"]
mod count_dial;
pub use count_dial::{count_to_collapse, tool_results_to_collapse};

/// Outcome of one demotion-ladder ceiling pass (#1046 AC6, #1044 AC1).
#[derive(Debug, Clone, Default)]
pub struct CeilingLadderOutcome {
    /// Full conversation messages demoted to recall stubs (first rung).
    pub collapsed_to_stubs: usize,
    /// Stubs removed entirely, manifest-only (second rung).
    pub dropped: usize,
    /// The budget is still exceeded after full demotion: the pinned/exempt
    /// set alone, the kept report among it (#2226), is over it (#1044).
    pub over_budget: bool,
}

/// Enforce the context ceiling by demoting down the ladder (#1046 AC6).
/// Nothing moves at or under `max_tokens`. Once it is crossed, first
/// collapse not-yet-collapsed messages to recall stubs (oldest first —
/// cheap, keeps locality) down to [`low_water`]`(max_tokens)`, so the
/// following turns append without rewriting the prefix (#2213). Only if
/// the ceiling itself is still exceeded remove stubs entirely
/// (manifest-only; content is already on disk from creation-time
/// spilling): down to the low-water mark when the exempt set leaves it
/// reachable, otherwise only down to the ceiling. The in-flight exchange
/// (see `in_flight_start`) is exempt like the pinned set. Pinned/exempt messages
/// are never demoted at any rung; when they alone exceed the budget the
/// outcome reports `over_budget` so the caller can warn and audit (#1044).
pub fn enforce_context_ceiling_ladder(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    pin_recent_turns: u32,
) -> CeilingLadderOutcome {
    let mut outcome = CeilingLadderOutcome::default();
    let mut total = estimate_total_tokens(messages);
    if total <= max_tokens {
        return outcome;
    }
    // Crossed: demote down to the low-water mark, not just under the ceiling.
    let target = low_water(max_tokens);
    debug_assert!(target <= max_tokens);
    let mut exempt = exempt_flags(messages, pin_recent_turns, true);
    // Whatever the pinning, the in-flight exchange is exempt at both rungs.
    let in_flight = in_flight_start(messages);
    exempt[in_flight..].fill(true);

    // First rung: demote full messages to stubs, oldest first. A message
    // whose stub would be no cheaper than its content (tiny messages) is
    // skipped — it goes straight to the second rung instead.
    for (i, msg) in messages.iter_mut().enumerate() {
        if total <= target {
            break;
        }
        if exempt[i] || msg.is_collapsed {
            continue;
        }
        // Unspilled content (spill_id == None: a spill-append failure or a
        // missing store at creation — conversation and tool results alike) is
        // never stubbed: its recall() would be unresolvable. It falls through
        // to the second rung's plain drop, as the pre-#1046 ceiling did.
        if msg.spill_id.is_none() {
            continue;
        }
        let before = estimate_message_tokens(msg);
        let stub_tokens = estimate_tokens(&message_collapse_stub(
            msg.role.as_str(),
            &msg.content,
            before,
            msg.spill_id.as_deref().unwrap_or("unknown"),
        ));
        if stub_tokens >= before {
            continue;
        }
        match msg.role {
            Role::Tool => collapse_message(msg),
            Role::User | Role::Assistant => {
                // Guarded above: conversation messages here have a spill_id.
                let Some(spill_id) = msg.spill_id.clone() else {
                    continue;
                };
                collapse_conversation_message(msg, &spill_id);
            }
            Role::System => continue,
        }
        outcome.collapsed_to_stubs += 1;
        total = total.saturating_sub(before) + estimate_message_tokens(msg);
    }

    // Second rung, a last resort for the ceiling itself: remove demoted
    // messages entirely, oldest first (content stays recallable via the
    // spill store and manifest), to the low-water mark when dropping can
    // reach it, else only to the ceiling, so no stub is deleted in vain. A
    // recallable report is only ever stubbed, never removed: its supervisor
    // must still find it among the messages (#2226, `report_to_keep`).
    if total > max_tokens {
        if let Some(report) = report_to_keep(messages) {
            exempt[report] = true;
        }
        // Whole exchanges only (#2349 review M1): a kept message keeps its
        // exchange, and a dropped exchange goes with all of its results.
        let groups = exchange_groups(messages);
        keep_exchanges_whole(&mut exempt, &groups);
        let exempt_tokens: usize = messages
            .iter()
            .zip(&exempt)
            .filter(|&(_, &keep)| keep)
            .map(|(m, _)| estimate_message_tokens(m))
            .sum();
        let drop_target = if exempt_tokens <= target {
            target
        } else {
            max_tokens
        };
        outcome.dropped =
            drop_exchanges_until_under_budget(messages, drop_target, &exempt, &groups);
    }

    outcome.over_budget = estimate_total_tokens(messages) > max_tokens;
    outcome
}

#[cfg(test)]
#[path = "context_pruning_ceiling_tests.rs"]
mod tests;

// #2349 review M1: removal keeps call/result exchanges whole.
#[cfg(test)]
#[path = "context_pruning_exchange_tests.rs"]
mod exchange_tests;
