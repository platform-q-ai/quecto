// #2348: the size-aware collapse. A tool result over `over_tokens` that
// the model has seen for `after_turns` requests collapses to its recall
// stub, whatever the count dial says. A count dial is blind to size: in
// the owner's run one 17k-token bash output rode 98 coordinator requests
// (31% of its input) while a dial of 50 or 100 waited for small results
// to pile up. Pure policy over the message list: no retention handle, no
// store.

use super::snapshots::newest_snapshots;
use super::{collapse_message, collapse_stub, estimate_message_tokens, estimate_tokens};
use crate::domain::large_result_collapse::LargeResultCollapse;
use crate::domain::message::{Message, Role};

/// The tool whose results the size rule never collapses (#2348 review M1):
/// the model asked for that content back, and a re-collapse three turns
/// later would start a recall loop. A result carrying images is exempt too
/// (review L1): a stub drops the image blocks, and recall cannot restore them.
const RECALL: &str = "recall";

/// Collapse every tool result over the dial's size that the model has
/// seen for its turns to its recall stub. Only a live result that was
/// spilled (its stub is recallable) and is not the newest snapshot of its
/// state (#2342) qualifies, and only when its stub is cheaper. Removes no
/// message, so every call keeps its result (#2349). Returns how many it
/// collapsed; a pass that finds nothing new rewrites no prompt prefix.
pub fn collapse_large_results(messages: &mut [Message], dial: LargeResultCollapse) -> usize {
    if dial.over_tokens == usize::MAX {
        return 0;
    }
    // A result is seen once per model response after it (#2213).
    let after_turns = dial.after_turns.max(1) as usize;
    let newest = newest_snapshots(messages);
    let mut responses_after = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    let mut collapsed = 0;
    for (msg, &newest) in messages.iter_mut().zip(&newest) {
        let seen = responses_after >= after_turns;
        match msg.role {
            Role::Assistant => responses_after -= 1,
            Role::Tool
                if seen
                    && !newest
                    && !msg.is_collapsed
                    && msg.spill_id.is_some()
                    && msg.image_blocks.is_empty()
                    && msg.tool_name.as_deref() != Some(RECALL)
                    && large(msg, dial.over_tokens) =>
            {
                collapse_message(msg);
                debug_assert!(msg.is_collapsed, "a large seen result is now a stub");
                collapsed += 1;
            }
            _ => {}
        }
    }
    debug_assert_eq!(responses_after, 0, "every response was counted down");
    collapsed
}

/// Whether `msg` is over `over_tokens` and its stub would be cheaper.
fn large(msg: &Message, over_tokens: usize) -> bool {
    let tokens = estimate_message_tokens(msg);
    if tokens <= over_tokens {
        return false;
    }
    let stub = collapse_stub(
        msg.tool_name.as_deref().unwrap_or("tool"),
        msg.input_preview.as_deref().unwrap_or(""),
        tokens,
        msg.spill_id.as_deref().unwrap_or("unknown"),
    );
    estimate_tokens(&stub) < tokens
}

#[cfg(test)]
#[path = "context_pruning_large_results_tests.rs"]
mod tests;
