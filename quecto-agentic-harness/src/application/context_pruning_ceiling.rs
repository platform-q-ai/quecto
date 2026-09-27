// #1046 / #2213: the demotion-ladder context ceiling (full → stub → removed).
//
// Split from `context_pruning_messages.rs` (the 750-line cap and its
// decrease-only ceiling). Pure policy over the message list: no retention
// handle, no store.

use super::{collapse_conversation_message, exempt_flags, message_collapse_stub};
use crate::application::context_pruning::{
    collapse_message, drop_until_under_budget, estimate_message_tokens, estimate_tokens,
    estimate_total_tokens,
};
use crate::domain::message::{Message, Role};

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
/// round-trip. A lower mark buys longer batches with more stubbed context; a
/// higher one drifts back towards a miss per turn.
pub const LOW_WATER_PERCENT: usize = 75;

// A mark of 0 would empty the context; 100 is the old per-turn pruning.
const _: () = assert!(LOW_WATER_PERCENT > 0 && LOW_WATER_PERCENT < 100);

/// The low-water mark of a dial: [`LOW_WATER_PERCENT`] of `limit`, rounded
/// up so a small count dial keeps at least its share (a one-item dial keeps
/// its item). Never above `limit`; computed wide so no limit overflows.
/// Rounding up means count dials of 1 to 3 get no hysteresis (their mark is
/// the dial itself): they still collapse the overflow on every crossing.
pub fn low_water(limit: usize) -> usize {
    let wide = (limit as u128 * LOW_WATER_PERCENT as u128).div_ceil(100);
    let mark = usize::try_from(wide).map_or(limit, |mark| mark.min(limit));
    debug_assert!(mark <= limit, "the low-water mark is under its dial");
    mark
}

/// How many of `live` items a count dial collapses: none at or under
/// `limit`; once `limit` is crossed, enough to leave [`low_water`]`(limit)`.
pub fn count_to_collapse(live: usize, limit: usize) -> usize {
    if live > limit {
        live - low_water(limit)
    } else {
        0
    }
}

/// Where the in-flight exchange starts (#2213): the model has not seen the
/// messages after its last assistant message yet (the results of that
/// message's tool calls), and a tool-calling last assistant message must
/// keep its results paired, so it is in flight too. With no assistant
/// message nothing is in flight. Neither count dial nor ladder rung demotes
/// an in-flight message: a stubbed unseen result would only be recalled.
fn in_flight_start(messages: &[Message]) -> usize {
    match messages.iter().rposition(|m| m.role == Role::Assistant) {
        Some(last) if messages[last].tool_calls.is_empty() => last + 1,
        Some(last) => last,
        None => messages.len(),
    }
}

/// The tool results a count dial collapses (#2213), as `(to_collapse,
/// seen_end)`: collapse `to_collapse` live results, oldest first, within
/// `messages[..seen_end]`, which ends where the in-flight exchange starts.
/// Results never spilled (`spill_id == None`) would mint an unresolvable
/// `recall()` stub: they are neither counted nor collapsed.
pub fn tool_results_to_collapse(messages: &[Message], limit: usize) -> (usize, usize) {
    let collapsible = |m: &Message| m.role == Role::Tool && !m.is_collapsed && m.spill_id.is_some();
    let seen_end = in_flight_start(messages);
    let live = messages.iter().filter(|m| collapsible(m)).count();
    let seen = messages[..seen_end]
        .iter()
        .filter(|m| collapsible(m))
        .count();
    let to_collapse = count_to_collapse(live, limit).min(seen);
    debug_assert!(to_collapse <= seen && seen <= live);
    (to_collapse, seen_end)
}

/// Outcome of one demotion-ladder ceiling pass (#1046 AC6, #1044 AC1).
#[derive(Debug, Clone, Default)]
pub struct CeilingLadderOutcome {
    /// Full conversation messages demoted to recall stubs (first rung).
    pub collapsed_to_stubs: usize,
    /// Stubs removed entirely, manifest-only (second rung).
    pub dropped: usize,
    /// True when the budget is still exceeded after full demotion — the
    /// pinned/exempt set alone is over budget (#1044).
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
    // spill store and manifest). It drops to the low-water mark only when
    // dropping can reach it; when the exempt set alone is above the mark it
    // drops only down to the ceiling, so no stub is deleted in vain.
    if total > max_tokens {
        let droppable: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|&(i, _)| !exempt[i])
            .map(|(i, _)| i)
            .collect();
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
        outcome.dropped = drop_until_under_budget(messages, drop_target, &droppable).len();
    }

    outcome.over_budget = estimate_total_tokens(messages) > max_tokens;
    outcome
}

#[cfg(test)]
#[path = "context_pruning_ceiling_tests.rs"]
mod tests;
