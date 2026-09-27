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

/// Enforce the context ceiling by demoting down the ladder (#1046 AC6):
/// first collapse not-yet-collapsed messages to recall stubs (oldest first —
/// cheap, keeps locality), and only if still over budget remove stubs
/// entirely (manifest-only; content is already on disk from creation-time
/// spilling). Pinned/exempt messages are never demoted at any rung; when
/// they alone exceed the budget the outcome reports `over_budget` so the
/// caller can warn and audit (#1044).
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
    let exempt = exempt_flags(messages, pin_recent_turns, true);

    // First rung: demote full messages to stubs, oldest first. A message
    // whose stub would be no cheaper than its content (tiny messages) is
    // skipped — it goes straight to the second rung instead.
    for (i, msg) in messages.iter_mut().enumerate() {
        if total <= max_tokens {
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

    // Second rung: remove demoted messages entirely, oldest first (content
    // stays recallable via the spill store and manifest).
    if total > max_tokens {
        let droppable: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|&(i, _)| !exempt[i])
            .map(|(i, _)| i)
            .collect();
        outcome.dropped = drop_until_under_budget(messages, max_tokens, &droppable).len();
    }

    outcome.over_budget = estimate_total_tokens(messages) > max_tokens;
    outcome
}
